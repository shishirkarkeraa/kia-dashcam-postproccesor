use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CleanupPolicy {
    Keep,
    #[default]
    Trash,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TimestampSource {
    ContainerMetadata,
    VideoMetadata,
    Filename,
    ModifiedTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VideoCandidate {
    pub id: String,
    pub path: PathBuf,
    pub display_path: String,
    pub included: bool,
    pub valid: bool,
    pub reason: Option<String>,
    pub recording_time: Option<DateTime<Utc>>,
    pub timestamp_source: Option<TimestampSource>,
    pub video_streams: usize,
    pub audio_streams: usize,
    pub duration_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryRequest {
    pub paths: Vec<PathBuf>,
    pub display_root: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobPlan {
    pub candidates: Vec<VideoCandidate>,
    pub output_dir: PathBuf,
    pub cleanup: CleanupPolicy,
    #[serde(default)]
    pub restart: bool,
    #[serde(default)]
    pub resume_pending: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingJobInfo {
    pub output_path: PathBuf,
    pub input_count: usize,
    pub completed_tasks: usize,
    pub total_tasks: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Preparing,
    Splitting,
    Stacking,
    Stitching,
    Compressing,
    Cleaning,
    Complete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobEventKind {
    Started,
    Progress,
    Warning,
    Log,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEvent {
    pub kind: JobEventKind,
    pub stage: Stage,
    pub message: String,
    pub current_file: Option<String>,
    pub completed_tasks: usize,
    pub total_tasks: usize,
    pub elapsed_seconds: f64,
    pub eta_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    pub output_path: PathBuf,
    pub processed_files: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}
