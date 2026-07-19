use crate::EventCallback;
use crate::model::{
    CancellationToken, CleanupPolicy, JobEvent, JobEventKind, JobPlan, JobResult, PendingJobInfo,
    Stage,
};
use crate::tools::ToolPaths;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, UNIX_EPOCH};
use thiserror::Error;

const WORK_ROOT: &str = ".kia-dashcam-work";
const MANIFEST_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("no valid, selected AVI files were provided")]
    NoInputs,
    #[error("job was cancelled")]
    Cancelled,
    #[error("filesystem error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid job state: {0}")]
    State(String),
    #[error("{tool} failed during {stage}; see {log}")]
    Command {
        tool: String,
        stage: String,
        log: PathBuf,
    },
    #[error("generated media validation failed for {0}")]
    Validation(PathBuf),
    #[error("could not serialize job state: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl ProcessError {
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::NoInputs | Self::State(_) => 2,
            Self::Command { .. } | Self::Validation(_) | Self::Cancelled => 3,
            Self::Io { .. } | Self::Serialization(_) => 4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JobManifest {
    version: u32,
    fingerprint: String,
    output_path: PathBuf,
    cleanup: CleanupPolicy,
    inputs: Vec<ManifestInput>,
    concat_done: bool,
    compression_done: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestInput {
    id: String,
    source: PathBuf,
    split_done: bool,
    stack_done: bool,
    source_trashed: bool,
}

struct Progress<'a> {
    started: Instant,
    completed: usize,
    total: usize,
    callback: &'a EventCallback,
}

impl<'a> Progress<'a> {
    fn emit(
        &self,
        kind: JobEventKind,
        stage: Stage,
        message: impl Into<String>,
        current_file: Option<String>,
    ) {
        let elapsed = self.started.elapsed().as_secs_f64();
        let eta = if self.completed == 0 {
            None
        } else {
            Some((elapsed / self.completed as f64) * (self.total - self.completed) as f64)
        };
        (self.callback)(JobEvent {
            kind,
            stage,
            message: message.into(),
            current_file,
            completed_tasks: self.completed,
            total_tasks: self.total,
            elapsed_seconds: elapsed,
            eta_seconds: eta,
        });
    }

    fn complete_task(&mut self, stage: Stage, message: impl Into<String>, file: Option<String>) {
        self.completed += 1;
        self.emit(JobEventKind::Progress, stage, message, file);
    }
}

pub fn pending_job(output_dir: &Path) -> Result<Option<PendingJobInfo>, ProcessError> {
    let Some((_workspace, manifest)) = newest_manifest(output_dir)? else {
        return Ok(None);
    };
    let completed_tasks = manifest
        .inputs
        .iter()
        .map(|input| {
            usize::from(input.split_done || input.stack_done) + usize::from(input.stack_done)
        })
        .sum::<usize>()
        + usize::from(manifest.concat_done)
        + usize::from(manifest.compression_done);
    Ok(Some(PendingJobInfo {
        output_path: manifest.output_path,
        input_count: manifest.inputs.len(),
        completed_tasks,
        total_tasks: manifest.inputs.len() * 2 + 2,
    }))
}

pub fn process_job(
    plan: &JobPlan,
    tools: &ToolPaths,
    cancel: &CancellationToken,
    callback: &EventCallback,
) -> Result<JobResult, ProcessError> {
    let selected: Vec<_> = plan
        .candidates
        .iter()
        .filter(|candidate| candidate.valid && candidate.included)
        .collect();
    create_dir_all(&plan.output_dir)?;
    let selected_ids: HashSet<_> = selected
        .iter()
        .map(|candidate| candidate.id.as_str())
        .collect();
    let resumable = if plan.restart {
        None
    } else {
        find_resumable_manifest(
            &plan.output_dir,
            &selected_ids,
            selected.is_empty() && plan.resume_pending,
        )?
    };

    let (workspace, manifest_path, mut manifest, resumed) =
        if let Some((workspace, manifest)) = resumable {
            let manifest_path = workspace.join("job.json");
            (workspace, manifest_path, manifest, true)
        } else {
            if selected.is_empty() {
                return Err(ProcessError::NoInputs);
            }
            let fingerprint = fingerprint(plan, &selected)?;
            let workspace = plan.output_dir.join(WORK_ROOT).join(&fingerprint[..16]);
            if plan.restart && workspace.exists() {
                for candidate in &selected {
                    if !candidate.path.is_file() {
                        return Err(ProcessError::State(format!(
                            "cannot restart because source is no longer present: {}",
                            candidate.path.display()
                        )));
                    }
                }
                remove_dir_all(&workspace)?;
            }
            create_dir_all(&workspace)?;
            let manifest_path = workspace.join("job.json");
            let was_resumed = manifest_path.is_file();
            let manifest = if was_resumed {
                load_manifest(&manifest_path, &fingerprint)?
            } else {
                let manifest = JobManifest {
                    version: MANIFEST_VERSION,
                    fingerprint: fingerprint.clone(),
                    output_path: next_output_path(&plan.output_dir),
                    cleanup: plan.cleanup,
                    inputs: selected
                        .iter()
                        .map(|candidate| ManifestInput {
                            id: candidate.id.clone(),
                            source: candidate.path.clone(),
                            split_done: false,
                            stack_done: false,
                            source_trashed: false,
                        })
                        .collect(),
                    concat_done: false,
                    compression_done: false,
                };
                save_manifest(&manifest_path, &manifest)?;
                manifest
            };
            (workspace, manifest_path, manifest, was_resumed)
        };
    create_dir_all(&workspace)?;
    let log_path = workspace.join("job.log");

    let total_tasks = manifest.inputs.len() * 2 + 2;
    let mut progress = Progress {
        started: Instant::now(),
        completed: 0,
        total: total_tasks,
        callback,
    };
    progress.emit(
        JobEventKind::Started,
        Stage::Preparing,
        if resumed {
            "Resuming interrupted job"
        } else {
            "Preparing job"
        },
        None,
    );

    for index in 0..manifest.inputs.len() {
        check_cancel(cancel, &progress)?;
        let source = manifest.inputs[index].source.clone();
        let display = source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let track1 = workspace.join(format!("{:06}_track1.avi", index + 1));
        let track2 = workspace.join(format!("{:06}_track2.avi", index + 1));
        let stacked = workspace.join(format!("{:06}_stacked.avi", index + 1));

        if manifest.inputs[index].stack_done && !stacked.is_file() {
            manifest.inputs[index].stack_done = false;
            manifest.inputs[index].split_done = false;
        }
        if !manifest.inputs[index].stack_done
            && manifest.inputs[index].split_done
            && (!track1.is_file() || !track2.is_file())
        {
            manifest.inputs[index].split_done = false;
        }

        if manifest.inputs[index].split_done || manifest.inputs[index].stack_done {
            progress.complete_task(
                Stage::Splitting,
                "Split already complete; resumed",
                Some(display.clone()),
            );
        } else {
            progress.emit(
                JobEventKind::Progress,
                Stage::Splitting,
                "Splitting two-channel video",
                Some(display.clone()),
            );
            if !source.is_file() {
                return Err(ProcessError::State(format!(
                    "source is missing and no resumable split exists: {}",
                    source.display()
                )));
            }
            let args = vec![
                os("-fflags"),
                os("+genpts"),
                os("-i"),
                source.as_os_str().to_owned(),
                os("-map"),
                os("0:v:0"),
                os("-map"),
                os("0:a:0"),
                os("-c:v"),
                os("copy"),
                os("-c:a"),
                os("copy"),
                track1.as_os_str().to_owned(),
                os("-map"),
                os("0:v:1"),
                os("-map"),
                os("0:a:0"),
                os("-c:v"),
                os("copy"),
                os("-c:a"),
                os("copy"),
                track2.as_os_str().to_owned(),
                os("-y"),
            ];
            run_command(
                &tools.ffmpeg,
                &args,
                &workspace,
                &log_path,
                "splitting",
                cancel,
            )?;
            validate_media(&track1, tools)?;
            validate_media(&track2, tools)?;
            manifest.inputs[index].split_done = true;
            save_manifest(&manifest_path, &manifest)?;
            progress.complete_task(Stage::Splitting, "Split complete", Some(display.clone()));
        }

        if manifest.inputs[index].stack_done {
            progress.complete_task(
                Stage::Stacking,
                "Stack already complete; resumed",
                Some(display.clone()),
            );
        } else {
            check_cancel(cancel, &progress)?;
            progress.emit(
                JobEventKind::Progress,
                Stage::Stacking,
                "Stacking channels vertically",
                Some(display.clone()),
            );
            let args = vec![
                os("-i"),
                track1.as_os_str().to_owned(),
                os("-i"),
                track2.as_os_str().to_owned(),
                os("-filter_complex"),
                os(
                    "[0:v]scale=1920:-2,format=yuv420p[v0];[1:v]scale=1920:-2,format=yuv420p[v1];[v0][v1]vstack=inputs=2[v]",
                ),
                os("-map"),
                os("[v]"),
                os("-map"),
                os("0:a:0"),
                os("-c:v"),
                os("libx264"),
                os("-preset"),
                os("fast"),
                os("-crf"),
                os("23"),
                os("-c:a"),
                os("aac"),
                os("-b:a"),
                os("192k"),
                stacked.as_os_str().to_owned(),
                os("-y"),
            ];
            run_command(
                &tools.ffmpeg,
                &args,
                &workspace,
                &log_path,
                "stacking",
                cancel,
            )?;
            validate_media(&stacked, tools)?;
            manifest.inputs[index].stack_done = true;
            remove_file_if_present(&track1)?;
            remove_file_if_present(&track2)?;

            if manifest.cleanup == CleanupPolicy::Trash
                && !manifest.inputs[index].source_trashed
                && source.is_file()
            {
                match trash::delete(&source) {
                    Ok(()) => manifest.inputs[index].source_trashed = true,
                    Err(error) => {
                        progress.emit(
                            JobEventKind::Warning,
                            Stage::Cleaning,
                            format!(
                                "Could not move {} to system Trash; original was kept: {error}",
                                source.display()
                            ),
                            Some(display.clone()),
                        );
                    }
                }
            }
            save_manifest(&manifest_path, &manifest)?;
            progress.complete_task(Stage::Stacking, "Stack complete", Some(display));
        }
    }

    let stitched = workspace.join("final_stitched_sequence.avi");
    if manifest.concat_done && !stitched.is_file() {
        manifest.concat_done = false;
    }
    if manifest.concat_done {
        progress.complete_task(
            Stage::Stitching,
            "Stitch already complete; resumed",
            Some("final_stitched_sequence.avi".to_string()),
        );
    } else {
        check_cancel(cancel, &progress)?;
        let concat_list = workspace.join("concat_list.txt");
        let mut list = String::new();
        for index in 0..manifest.inputs.len() {
            list.push_str(&format!("file '{:06}_stacked.avi'\n", index + 1));
        }
        write_file(&concat_list, list.as_bytes())?;
        progress.emit(
            JobEventKind::Progress,
            Stage::Stitching,
            "Stitching final sequence",
            Some("final_stitched_sequence.avi".to_string()),
        );
        let args = vec![
            os("-fflags"),
            os("+genpts"),
            os("-f"),
            os("concat"),
            os("-safe"),
            os("0"),
            os("-i"),
            os("concat_list.txt"),
            os("-c"),
            os("copy"),
            stitched.as_os_str().to_owned(),
            os("-y"),
        ];
        run_command(
            &tools.ffmpeg,
            &args,
            &workspace,
            &log_path,
            "stitching",
            cancel,
        )?;
        validate_media(&stitched, tools)?;
        manifest.concat_done = true;
        for index in 0..manifest.inputs.len() {
            remove_file_if_present(&workspace.join(format!("{:06}_stacked.avi", index + 1)))?;
        }
        remove_file_if_present(&concat_list)?;
        save_manifest(&manifest_path, &manifest)?;
        progress.complete_task(
            Stage::Stitching,
            "Stitch complete",
            Some("final_stitched_sequence.avi".to_string()),
        );
    }

    check_cancel(cancel, &progress)?;
    let compressed = workspace.join("final_stitched_sequence_compressed.mp4");
    if manifest.compression_done && !compressed.is_file() {
        manifest.compression_done = false;
    }
    if !manifest.compression_done {
        progress.emit(
            JobEventKind::Progress,
            Stage::Compressing,
            "Compressing final sequence with H.265",
            Some("final_stitched_sequence_compressed.mp4".to_string()),
        );
        let args = vec![
            os("-i"),
            stitched.as_os_str().to_owned(),
            os("-o"),
            compressed.as_os_str().to_owned(),
            os("-e"),
            os("x265"),
            os("-q"),
            os("22"),
            os("--encoder-preset"),
            os("fast"),
            os("--crop"),
            os("0:0:0:0"),
            os("-E"),
            os("copy"),
        ];
        run_command(
            &tools.handbrake,
            &args,
            &workspace,
            &log_path,
            "compression",
            cancel,
        )?;
        validate_media(&compressed, tools)?;
        manifest.compression_done = true;
        save_manifest(&manifest_path, &manifest)?;
    }

    if manifest.output_path.exists() {
        manifest.output_path = next_output_path(&plan.output_dir);
        save_manifest(&manifest_path, &manifest)?;
    }
    rename_file(&compressed, &manifest.output_path)?;
    progress.complete_task(
        Stage::Compressing,
        "Compression complete",
        manifest
            .output_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string()),
    );
    let output_path = manifest.output_path.clone();
    let processed_files = manifest.inputs.len();
    remove_dir_all(&workspace)?;
    progress.emit(
        JobEventKind::Completed,
        Stage::Complete,
        format!("Created {}", output_path.display()),
        output_path
            .file_name()
            .map(|name| name.to_string_lossy().to_string()),
    );
    Ok(JobResult {
        output_path,
        processed_files,
        warnings: Vec::new(),
    })
}

fn fingerprint(
    plan: &JobPlan,
    selected: &[&crate::VideoCandidate],
) -> Result<String, ProcessError> {
    let mut hasher = Sha256::new();
    hasher.update(plan.output_dir.to_string_lossy().as_bytes());
    hasher.update(format!("{:?}", plan.cleanup).as_bytes());
    for candidate in selected {
        hasher.update(candidate.path.to_string_lossy().as_bytes());
        let metadata = fs::metadata(&candidate.path).map_err(|source| ProcessError::Io {
            path: candidate.path.clone(),
            source,
        })?;
        hasher.update(metadata.len().to_le_bytes());
        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
        {
            hasher.update(duration.as_nanos().to_le_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn find_resumable_manifest(
    output_dir: &Path,
    selected_ids: &HashSet<&str>,
    allow_empty_resume: bool,
) -> Result<Option<(PathBuf, JobManifest)>, ProcessError> {
    let Some((workspace, manifest)) = newest_manifest(output_dir)? else {
        return Ok(None);
    };
    let visible_ids: HashSet<_> = manifest
        .inputs
        .iter()
        .filter(|input| !input.source_trashed && input.source.is_file())
        .map(|input| input.id.as_str())
        .collect();
    let matches = if selected_ids.is_empty() {
        allow_empty_resume && visible_ids.is_empty()
    } else {
        selected_ids == &visible_ids
    };
    Ok(matches.then_some((workspace, manifest)))
}

fn newest_manifest(output_dir: &Path) -> Result<Option<(PathBuf, JobManifest)>, ProcessError> {
    let root = output_dir.join(WORK_ROOT);
    if !root.is_dir() {
        return Ok(None);
    }
    let entries = fs::read_dir(&root).map_err(|source| ProcessError::Io {
        path: root.clone(),
        source,
    })?;
    let mut manifests = Vec::new();
    for entry in entries.flatten() {
        let workspace = entry.path();
        let manifest_path = workspace.join("job.json");
        if !manifest_path.is_file() {
            continue;
        }
        let modified = fs::metadata(&manifest_path)
            .and_then(|metadata| metadata.modified())
            .unwrap_or(UNIX_EPOCH);
        if let Ok(manifest) = read_manifest(&manifest_path)
            && manifest.version == MANIFEST_VERSION
        {
            manifests.push((modified, workspace, manifest));
        }
    }
    manifests.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    Ok(manifests
        .into_iter()
        .next()
        .map(|(_, workspace, manifest)| (workspace, manifest)))
}

fn load_manifest(path: &Path, fingerprint: &str) -> Result<JobManifest, ProcessError> {
    let manifest = read_manifest(path)?;
    if manifest.version != MANIFEST_VERSION || manifest.fingerprint != fingerprint {
        return Err(ProcessError::State(
            "saved job does not match the selected inputs".to_string(),
        ));
    }
    Ok(manifest)
}

fn read_manifest(path: &Path) -> Result<JobManifest, ProcessError> {
    let mut file = File::open(path).map_err(|source| ProcessError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .map_err(|source| ProcessError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let manifest: JobManifest = serde_json::from_slice(&contents)?;
    Ok(manifest)
}

fn save_manifest(path: &Path, manifest: &JobManifest) -> Result<(), ProcessError> {
    let temporary = path.with_extension("json.tmp");
    write_file(&temporary, &serde_json::to_vec_pretty(manifest)?)?;
    if path.exists() {
        remove_file_if_present(path)?;
    }
    rename_file(&temporary, path)
}

fn next_output_path(output_dir: &Path) -> PathBuf {
    let first = output_dir.join("final_stitched_sequence_compressed.mp4");
    if !first.exists() {
        return first;
    }
    let mut index = 2;
    loop {
        let candidate = output_dir.join(format!("final_stitched_sequence_compressed_{index}.mp4"));
        if !candidate.exists() {
            return candidate;
        }
        index += 1;
    }
}

fn run_command(
    executable: &Path,
    args: &[OsString],
    current_dir: &Path,
    log_path: &Path,
    stage: &str,
    cancel: &CancellationToken,
) -> Result<(), ProcessError> {
    let mut log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|source| ProcessError::Io {
            path: log_path.to_path_buf(),
            source,
        })?;
    writeln!(
        log,
        "\n=== {stage}: {} {:?} ===",
        executable.display(),
        args
    )
    .map_err(|source| ProcessError::Io {
        path: log_path.to_path_buf(),
        source,
    })?;
    let stderr = log.try_clone().map_err(|source| ProcessError::Io {
        path: log_path.to_path_buf(),
        source,
    })?;
    let mut child = Command::new(executable)
        .args(args)
        .current_dir(current_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|source| ProcessError::Io {
            path: executable.to_path_buf(),
            source,
        })?;
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessError::Cancelled);
        }
        match child.try_wait().map_err(|source| ProcessError::Io {
            path: executable.to_path_buf(),
            source,
        })? {
            Some(status) if status.success() => return Ok(()),
            Some(_) => {
                return Err(ProcessError::Command {
                    tool: executable
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    stage: stage.to_string(),
                    log: log_path.to_path_buf(),
                });
            }
            None => thread::sleep(Duration::from_millis(150)),
        }
    }
}

fn validate_media(path: &Path, tools: &ToolPaths) -> Result<(), ProcessError> {
    if fs::metadata(path).map(|value| value.len()).unwrap_or(0) == 0 {
        return Err(ProcessError::Validation(path.to_path_buf()));
    }
    let output = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=index",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|source| ProcessError::Io {
            path: tools.ffprobe.clone(),
            source,
        })?;
    if !output.status.success() {
        return Err(ProcessError::Validation(path.to_path_buf()));
    }
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    let has_stream = value
        .get("streams")
        .and_then(|streams| streams.as_array())
        .is_some_and(|streams| !streams.is_empty());
    if !has_stream {
        return Err(ProcessError::Validation(path.to_path_buf()));
    }
    Ok(())
}

fn check_cancel(cancel: &CancellationToken, progress: &Progress<'_>) -> Result<(), ProcessError> {
    if cancel.is_cancelled() {
        progress.emit(
            JobEventKind::Cancelled,
            Stage::Preparing,
            "Job cancelled; resumable files were preserved",
            None,
        );
        Err(ProcessError::Cancelled)
    } else {
        Ok(())
    }
}

fn os(value: &str) -> OsString {
    OsString::from(value)
}

fn create_dir_all(path: &Path) -> Result<(), ProcessError> {
    fs::create_dir_all(path).map_err(|source| ProcessError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), ProcessError> {
    fs::write(path, bytes).map_err(|source| ProcessError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn rename_file(from: &Path, to: &Path) -> Result<(), ProcessError> {
    fs::rename(from, to).map_err(|source| ProcessError::Io {
        path: to.to_path_buf(),
        source,
    })
}

fn remove_file_if_present(path: &Path) -> Result<(), ProcessError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProcessError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

fn remove_dir_all(path: &Path) -> Result<(), ProcessError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProcessError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{TimestampSource, VideoCandidate};
    use chrono::{TimeZone, Utc};

    #[test]
    fn versioned_output_never_overwrites() {
        let directory = tempfile::tempdir().unwrap();
        let first = next_output_path(directory.path());
        assert!(first.ends_with("final_stitched_sequence_compressed.mp4"));
        fs::write(&first, b"existing").unwrap();
        let second = next_output_path(directory.path());
        assert!(second.ends_with("final_stitched_sequence_compressed_2.mp4"));
    }

    #[cfg(unix)]
    #[test]
    fn pipeline_preserves_media_arguments_and_creates_final_output() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let source = directory.path().join("KIA 2026-07-19 14-35-01.avi");
        fs::write(&source, b"source").unwrap();
        let command_log = directory.path().join("commands.log");
        let escaped_log = command_log.to_string_lossy().replace('\'', "'\\''");
        let ffmpeg = directory.path().join("ffmpeg");
        let ffprobe = directory.path().join("ffprobe");
        let handbrake = directory.path().join("HandBrakeCLI");

        fs::write(
            &ffmpeg,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{escaped_log}'\nfor arg in \"$@\"; do\n  case \"$arg\" in\n    *.avi) if [ ! -e \"$arg\" ]; then printf 'fake-media' > \"$arg\"; fi ;;\n  esac\ndone\n"
            ),
        )
        .unwrap();
        fs::write(
            &ffprobe,
            "#!/bin/sh\nprintf '%s' '{\"streams\":[{\"index\":0}],\"format\":{\"duration\":\"1.0\"}}'\n",
        )
        .unwrap();
        fs::write(
            &handbrake,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{escaped_log}'\nout=''\nwhile [ \"$#\" -gt 0 ]; do\n  if [ \"$1\" = '-o' ]; then shift; out=\"$1\"; fi\n  shift\ndone\nprintf 'fake-media' > \"$out\"\n"
            ),
        )
        .unwrap();
        for tool in [&ffmpeg, &ffprobe, &handbrake] {
            let mut permissions = fs::metadata(tool).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(tool, permissions).unwrap();
        }

        let plan = JobPlan {
            candidates: vec![VideoCandidate {
                id: "clip-one".to_string(),
                path: source.clone(),
                display_path: "KIA 2026-07-19 14-35-01.avi".to_string(),
                included: true,
                valid: true,
                reason: None,
                recording_time: Some(Utc.with_ymd_and_hms(2026, 7, 19, 14, 35, 1).unwrap()),
                timestamp_source: Some(TimestampSource::Filename),
                video_streams: 2,
                audio_streams: 1,
                duration_seconds: Some(1.0),
            }],
            output_dir: directory.path().to_path_buf(),
            cleanup: CleanupPolicy::Keep,
            restart: false,
            resume_pending: false,
        };
        let tools = ToolPaths {
            ffmpeg,
            ffprobe,
            handbrake,
        };
        let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded_events = events.clone();
        let result = process_job(&plan, &tools, &CancellationToken::new(), &move |event| {
            recorded_events.lock().unwrap().push(event)
        })
        .unwrap();

        assert!(result.output_path.is_file());
        assert!(source.is_file(), "keep policy must preserve the original");
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .any(|event| event.kind == JobEventKind::Completed)
        );
        let commands = fs::read_to_string(command_log).unwrap();
        assert!(commands.contains("-map 0:v:0 -map 0:a:0 -c:v copy -c:a copy"));
        assert!(commands.contains("scale=1920:-2,format=yuv420p"));
        assert!(commands.contains("vstack=inputs=2"));
        assert!(commands.contains("-c:v libx264 -preset fast -crf 23 -c:a aac -b:a 192k"));
        assert!(commands.contains("-f concat -safe 0"));
        assert!(commands.contains("-c copy"));
        assert!(commands.contains("-e x265 -q 22 --encoder-preset fast --crop 0:0:0:0 -E copy"));
    }
}
