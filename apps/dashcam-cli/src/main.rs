use clap::{Parser, Subcommand, ValueEnum};
use kia_dashcam_core::{
    CancellationToken, CleanupPolicy, DiscoveryRequest, JobEventKind, JobPlan, ToolPaths,
    discover_inputs, process_job, resolve_tool_paths,
};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Debug, Parser)]
#[command(name = "kia-dashcam-cli")]
#[command(about = "Combine two-channel Kia dashcam AVI files into one stacked H.265 video")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Process {
        #[arg(value_name = "FOLDER")]
        folder: PathBuf,
        #[arg(long, value_name = "FOLDER")]
        output_dir: Option<PathBuf>,
        #[arg(long, value_enum, default_value_t = CleanupArg::Trash)]
        cleanup: CleanupArg,
        #[arg(long)]
        restart: bool,
        #[arg(long)]
        no_update_check: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CleanupArg {
    Trash,
    Keep,
}

impl From<CleanupArg> for CleanupPolicy {
    fn from(value: CleanupArg) -> Self {
        match value {
            CleanupArg::Trash => Self::Trash,
            CleanupArg::Keep => Self::Keep,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err((code, message)) => {
            eprintln!("Error: {message}");
            ExitCode::from(code)
        }
    }
}

fn run(cli: Cli) -> Result<(), (u8, String)> {
    match cli.command {
        Command::Process {
            folder,
            output_dir,
            cleanup,
            restart,
            no_update_check,
        } => {
            if !no_update_check {
                match updater::check_and_install() {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(error) => eprintln!("Update warning: {error}"),
                }
            }
            if !folder.is_dir() {
                return Err((
                    2,
                    format!("input folder does not exist: {}", folder.display()),
                ));
            }
            let output_dir = output_dir.unwrap_or_else(|| folder.clone());
            let tools = resolve_tool_paths(None).map_err(|error| (4, error.to_string()))?;
            process_folder(folder, output_dir, cleanup.into(), restart, tools)
        }
    }
}

fn process_folder(
    folder: PathBuf,
    output_dir: PathBuf,
    cleanup: CleanupPolicy,
    restart: bool,
    tools: ToolPaths,
) -> Result<(), (u8, String)> {
    println!("Scanning {} recursively...", folder.display());
    let candidates = discover_inputs(
        &DiscoveryRequest {
            paths: vec![folder.clone()],
            display_root: Some(folder),
        },
        &tools,
    );
    for candidate in candidates.iter().filter(|candidate| !candidate.valid) {
        eprintln!(
            "Skipping {}: {}",
            candidate.display_path,
            candidate.reason.as_deref().unwrap_or("invalid input")
        );
    }
    let selected_count = candidates
        .iter()
        .filter(|candidate| candidate.valid && candidate.included)
        .count();
    println!("Selected {selected_count} valid AVI file(s).");

    let cancel = CancellationToken::new();
    let signal_token = cancel.clone();
    ctrlc::set_handler(move || signal_token.cancel())
        .map_err(|error| (4, format!("could not install Ctrl-C handler: {error}")))?;
    let plan = JobPlan {
        candidates,
        output_dir,
        cleanup,
        restart,
        resume_pending: true,
    };
    let result = process_job(&plan, &tools, &cancel, &|event| match event.kind {
        JobEventKind::Warning => eprintln!("Warning: {}", event.message),
        JobEventKind::Log => println!("{}", event.message),
        _ => println!(
            "[{}/{}] {:?}: {}{}",
            event.completed_tasks,
            event.total_tasks,
            event.stage,
            event.message,
            event
                .current_file
                .as_deref()
                .map(|file| format!(" — {file}"))
                .unwrap_or_default()
        ),
    })
    .map_err(|error| (error.exit_code() as u8, error.to_string()))?;
    println!("Created {}", result.output_path.display());
    Ok(())
}

mod updater {
    use super::*;
    use semver::Version;
    use std::env;
    use std::error::Error;
    use std::process::Command;

    const UPDATE_REPOSITORY: Option<&str> = option_env!("KIA_DASHCAM_UPDATE_REPO");
    const UPDATE_PUBLIC_KEY_HEX: Option<&str> = option_env!("KIA_CLI_UPDATE_PUBLIC_KEY_HEX");

    pub fn check_and_install() -> Result<bool, Box<dyn Error>> {
        let Some(repository) = UPDATE_REPOSITORY else {
            return Ok(false);
        };
        let Some(public_key) = UPDATE_PUBLIC_KEY_HEX else {
            return Ok(false);
        };
        let public_key: [u8; 32] = hex::decode(public_key)?
            .try_into()
            .map_err(|_| "KIA_CLI_UPDATE_PUBLIC_KEY_HEX must contain exactly 32 bytes")?;
        let Some((owner, name)) = repository.split_once('/') else {
            return Err("KIA_DASHCAM_UPDATE_REPO must be OWNER/REPOSITORY".into());
        };
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(owner)
            .repo_name(name)
            .build()?
            .fetch()?;
        let Some(latest) = releases.first() else {
            return Ok(false);
        };
        let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
        let available = Version::parse(latest.version.trim_start_matches('v'))?;
        if available <= current {
            return Ok(false);
        }
        if !io::stdin().is_terminal() {
            eprintln!(
                "Kia Dashcam Processor {available} is available; rerun interactively to install it."
            );
            return Ok(false);
        }
        print!("Kia Dashcam Processor {available} is available. Install before processing? [Y/n] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !answer.trim().is_empty() && !answer.trim().eq_ignore_ascii_case("y") {
            return Ok(false);
        }
        let status = self_update::backends::github::Update::configure()
            .repo_owner(owner)
            .repo_name(name)
            .bin_name("kia-dashcam-cli")
            .show_download_progress(true)
            .current_version(env!("CARGO_PKG_VERSION"))
            .verifying_keys([public_key])
            .build()?
            .update()?;
        if status.updated() {
            let executable = env::current_exe()?;
            Command::new(executable)
                .args(env::args_os().skip(1))
                .spawn()?;
            return Ok(true);
        }
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_documented_process_contract() {
        let cli = Cli::try_parse_from([
            "kia-dashcam-cli",
            "process",
            "recordings",
            "--output-dir",
            "finished",
            "--cleanup",
            "keep",
            "--restart",
            "--no-update-check",
        ])
        .unwrap();

        let Command::Process {
            folder,
            output_dir,
            cleanup,
            restart,
            no_update_check,
        } = cli.command;
        assert_eq!(folder, PathBuf::from("recordings"));
        assert_eq!(output_dir, Some(PathBuf::from("finished")));
        assert_eq!(cleanup, CleanupArg::Keep);
        assert!(restart);
        assert!(no_update_check);
    }

    #[test]
    fn cleanup_defaults_to_trash_and_rejects_other_values() {
        let cli = Cli::try_parse_from(["kia-dashcam-cli", "process", "recordings"]).unwrap();
        let Command::Process { cleanup, .. } = cli.command;
        assert_eq!(cleanup, CleanupArg::Trash);
        assert!(
            Cli::try_parse_from([
                "kia-dashcam-cli",
                "process",
                "recordings",
                "--cleanup",
                "delete",
            ])
            .is_err()
        );
    }
}
