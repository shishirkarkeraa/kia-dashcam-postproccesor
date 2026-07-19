use std::env;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Clone)]
pub struct ToolPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
    pub handbrake: PathBuf,
}

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("could not determine the current executable directory: {0}")]
    Executable(#[from] std::io::Error),
    #[error("required media tool not found: {0}")]
    Missing(String),
}

pub fn resolve_tool_paths(base_dir: Option<&Path>) -> Result<ToolPaths, ToolError> {
    let base = match base_dir {
        Some(path) => path.to_path_buf(),
        None => env::current_exe()?
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf(),
    };
    let media_dir = base.join("media-tools");
    Ok(ToolPaths {
        ffmpeg: resolve_one(&media_dir, "ffmpeg")?,
        ffprobe: resolve_one(&media_dir, "ffprobe")?,
        handbrake: resolve_one(&media_dir, "HandBrakeCLI")?,
    })
}

fn resolve_one(media_dir: &Path, name: &str) -> Result<PathBuf, ToolError> {
    let executable_name = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    let bundled = media_dir.join(&executable_name);
    if bundled.is_file() {
        return Ok(bundled);
    }
    Err(ToolError::Missing(bundled.display().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn executable_name(name: &str) -> String {
        if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.to_string()
        }
    }

    #[test]
    fn resolves_all_tools_only_from_the_private_media_directory() {
        let root = tempdir().unwrap();
        let media = root.path().join("media-tools");
        fs::create_dir(&media).unwrap();
        for name in ["ffmpeg", "ffprobe", "HandBrakeCLI"] {
            fs::write(media.join(executable_name(name)), []).unwrap();
        }

        let tools = resolve_tool_paths(Some(root.path())).unwrap();
        assert_eq!(tools.ffmpeg, media.join(executable_name("ffmpeg")));
        assert_eq!(tools.ffprobe, media.join(executable_name("ffprobe")));
        assert_eq!(tools.handbrake, media.join(executable_name("HandBrakeCLI")));
    }

    #[test]
    fn reports_the_missing_internal_path_without_an_external_fallback() {
        let root = tempdir().unwrap();
        let error = resolve_tool_paths(Some(root.path())).unwrap_err();
        let expected = root
            .path()
            .join("media-tools")
            .join(executable_name("ffmpeg"));
        assert!(error.to_string().contains(&expected.display().to_string()));
    }
}
