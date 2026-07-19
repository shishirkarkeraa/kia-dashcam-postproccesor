mod discovery;
mod model;
mod process;
mod timestamp;
mod tools;

pub use discovery::{discover_inputs, inspect_video};
pub use model::{
    CancellationToken, CleanupPolicy, DiscoveryRequest, JobEvent, JobEventKind, JobPlan, JobResult,
    PendingJobInfo, Stage, TimestampSource, VideoCandidate,
};
pub use process::{ProcessError, pending_job, process_job};
pub use tools::{ToolPaths, resolve_tool_paths};

pub type EventCallback = dyn Fn(JobEvent) + Send + Sync;
