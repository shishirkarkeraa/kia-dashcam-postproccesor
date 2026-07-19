use crate::model::{DiscoveryRequest, VideoCandidate};
use crate::timestamp::choose_timestamp;
use crate::tools::ToolPaths;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use walkdir::WalkDir;

const EXCLUDED_FRAGMENTS: [&str; 4] = ["track1", "track2", "stacked", "final_stitched"];

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    tags: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    tags: Option<HashMap<String, String>>,
}

pub fn discover_inputs(request: &DiscoveryRequest, tools: &ToolPaths) -> Vec<VideoCandidate> {
    let mut paths = Vec::new();
    for input in &request.paths {
        if input.is_dir() {
            for entry in WalkDir::new(input)
                .follow_links(false)
                .into_iter()
                .flatten()
            {
                if entry.file_type().is_file() && is_source_avi(entry.path()) {
                    paths.push(entry.into_path());
                }
            }
        } else if input.is_file() && is_source_avi(input) {
            paths.push(input.clone());
        }
    }

    let mut seen = HashSet::new();
    paths.retain(|path| {
        let identity = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
        seen.insert(identity)
    });

    let mut candidates: Vec<_> = paths
        .into_iter()
        .map(|path| inspect_video(&path, request.display_root.as_deref(), tools))
        .collect();
    candidates.sort_by(candidate_order);
    candidates
}

pub fn inspect_video(
    path: &Path,
    display_root: Option<&Path>,
    tools: &ToolPaths,
) -> VideoCandidate {
    let display_path = display_root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path)
        .to_string_lossy()
        .to_string();
    let metadata = fs::metadata(path).ok();
    let id = stable_path_id(path);

    let probe = Command::new(&tools.ffprobe)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output();

    let output = match probe {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return invalid_candidate(
                id,
                path,
                display_path,
                format!(
                    "FFprobe failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            );
        }
        Err(error) => {
            return invalid_candidate(
                id,
                path,
                display_path,
                format!("Could not start FFprobe: {error}"),
            );
        }
    };

    let probe: ProbeOutput = match serde_json::from_slice(&output.stdout) {
        Ok(value) => value,
        Err(error) => {
            return invalid_candidate(
                id,
                path,
                display_path,
                format!("Invalid FFprobe response: {error}"),
            );
        }
    };
    let video_streams = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("video"))
        .count();
    let audio_streams = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .count();
    let first_video_tags = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .and_then(|stream| stream.tags.as_ref());
    let (recording_time, timestamp_source) = choose_timestamp(
        probe
            .format
            .as_ref()
            .and_then(|format| format.tags.as_ref()),
        first_video_tags,
        path,
        metadata.and_then(|value| value.modified().ok()),
    );
    let reason = if video_streams < 2 {
        Some(format!(
            "Expected at least 2 video channels; found {video_streams}"
        ))
    } else if audio_streams == 0 {
        Some("Expected audio stream 0:a:0; found no audio streams".to_string())
    } else {
        None
    };
    let valid = reason.is_none();

    VideoCandidate {
        id,
        path: path.to_path_buf(),
        display_path,
        included: valid,
        valid,
        reason,
        recording_time,
        timestamp_source,
        video_streams,
        audio_streams,
        duration_seconds: probe
            .format
            .and_then(|format| format.duration)
            .and_then(|duration| duration.parse().ok()),
    }
}

fn invalid_candidate(
    id: String,
    path: &Path,
    display_path: String,
    reason: String,
) -> VideoCandidate {
    VideoCandidate {
        id,
        path: path.to_path_buf(),
        display_path,
        included: false,
        valid: false,
        reason: Some(reason),
        recording_time: None,
        timestamp_source: None,
        video_streams: 0,
        audio_streams: 0,
        duration_seconds: None,
    }
}

fn is_source_avi(path: &Path) -> bool {
    let is_avi = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("avi"));
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    is_avi
        && !EXCLUDED_FRAGMENTS
            .iter()
            .any(|fragment| name.contains(fragment))
}

fn stable_path_id(path: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

fn candidate_order(left: &VideoCandidate, right: &VideoCandidate) -> Ordering {
    match (left.recording_time, right.recording_time) {
        (Some(left_time), Some(right_time)) => left_time
            .cmp(&right_time)
            .then_with(|| path_order(left, right)),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => path_order(left, right),
    }
}

fn path_order(left: &VideoCandidate, right: &VideoCandidate) -> Ordering {
    natord::compare_ignore_case(&left.display_path, &right.display_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn source_filter_is_case_insensitive_and_excludes_generated_names() {
        assert!(is_source_avi(Path::new("clip.AVI")));
        assert!(!is_source_avi(Path::new("clip_track1.avi")));
        assert!(!is_source_avi(Path::new("final_stitched_sequence.avi")));
        assert!(!is_source_avi(Path::new("clip.mp4")));
    }

    #[test]
    fn natural_path_order_handles_numbers() {
        let make = |display: &str| VideoCandidate {
            id: display.into(),
            path: PathBuf::from(display),
            display_path: display.into(),
            included: true,
            valid: true,
            reason: None,
            recording_time: None,
            timestamp_source: None,
            video_streams: 2,
            audio_streams: 1,
            duration_seconds: Some(1.0),
        };
        assert_eq!(
            path_order(&make("clip2.avi"), &make("clip10.avi")),
            Ordering::Less
        );
    }

    #[cfg(unix)]
    #[test]
    fn recursive_discovery_orders_by_filename_time_and_skips_invalid_streams() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let nested = directory.path().join("nested folder");
        fs::create_dir(&nested).unwrap();
        for name in [
            "REC_20260720_120000.AVI",
            "REC_20260719_120000.avi",
            "bad_20260721_120000.avi",
            "ignored_track1.avi",
        ] {
            fs::write(nested.join(name), b"fixture").unwrap();
        }
        let ffprobe = directory.path().join("ffprobe");
        fs::write(
            &ffprobe,
            r##"#!/bin/sh
case "$*" in
  *bad*) printf '%s' '{"streams":[{"codec_type":"video"},{"codec_type":"audio"}],"format":{"duration":"1"}}' ;;
  *) printf '%s' '{"streams":[{"codec_type":"video"},{"codec_type":"video"},{"codec_type":"audio"}],"format":{"duration":"1"}}' ;;
esac
"##,
        )
        .unwrap();
        let mut permissions = fs::metadata(&ffprobe).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&ffprobe, permissions).unwrap();
        let tools = ToolPaths {
            ffmpeg: ffprobe.clone(),
            ffprobe: ffprobe.clone(),
            handbrake: ffprobe,
        };

        let candidates = discover_inputs(
            &DiscoveryRequest {
                paths: vec![directory.path().to_path_buf()],
                display_root: Some(directory.path().to_path_buf()),
            },
            &tools,
        );
        assert_eq!(candidates.len(), 3);
        assert!(candidates[0].display_path.contains("20260719"));
        assert!(candidates[1].display_path.contains("20260720"));
        assert!(!candidates[2].valid);
        assert!(
            candidates[2]
                .reason
                .as_deref()
                .unwrap()
                .contains("at least 2 video channels")
        );
    }
}
