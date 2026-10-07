use crate::error::ErrorKind;
use crate::request::RequestSpec;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    Connecting = 0,
    Downloading = 1,
    Finalizing = 2,
    Verifying = 3,
}

impl Phase {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Phase::Downloading,
            2 => Phase::Finalizing,
            3 => Phase::Verifying,
            _ => Phase::Connecting,
        }
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    None = 0,
    Pause = 1,
    Remove = 2,
    Shutdown = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Running,
    Paused,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskError {
    pub kind: ErrorKind,
    pub message: String,
    pub at: u64,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub routes: Vec<crate::error::RouteFailure>,
}

impl TaskError {
    pub fn summary(&self) -> String {
        crate::error::summary(self.kind, self.status, &self.message)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogLine {
    pub at: u64,
    pub text: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: Uuid,
    pub spec: RequestSpec,
    pub dir: PathBuf,
    pub filename: String,
    pub path: Option<PathBuf>,
    pub size: Option<u64>,
    pub ranges: bool,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub content_type: Option<String>,
    pub final_url: Option<String>,
    pub status: Status,
    pub error: Option<TaskError>,
    pub auto_retries: u32,
    pub next_retry_at: Option<u64>,
    /// Holes still to download; `None` means start from scratch.
    pub remaining: Option<Vec<(u64, u64)>>,
    pub downloaded: u64,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub sha256: Option<String>,
    pub route: Option<String>,
    pub elapsed_secs: f64,
    pub source: String,
    pub log: Vec<LogLine>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoveMode {
    /// Remove the task only; files stay where they are.
    KeepFiles,
    /// Move the finished file to the system trash; the partial file is kept until purge so restore can resume.
    TrashFiles,
    /// Delete everything immediately.
    DeleteFiles,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeletedRecord {
    pub rec: TaskRecord,
    pub deleted_at: u64,
    pub mode: RemoveMode,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AddRequest {
    pub url: String,
    pub mirrors: Vec<String>,
    pub filename: Option<String>,
    pub dir: Option<PathBuf>,
    pub referrer: Option<String>,
    pub cookies: Option<String>,
    pub headers: Vec<(String, String)>,
    pub user_agent: Option<String>,
    pub sha256: Option<String>,
    pub start_paused: bool,
    pub source: Option<String>,
    /// Let a request with the same file name and size revive a failed/paused task (expired links).
    pub allow_refresh: bool,
    pub size_hint: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AddOutcome {
    pub id: Uuid,
    pub existed: bool,
    pub refreshed: bool,
    pub filename: String,
}

#[derive(Clone, Debug)]
pub struct SegView {
    pub start: u64,
    pub done: u64,
    pub end: u64,
    pub active: bool,
}

#[derive(Clone, Debug)]
pub struct TaskView {
    pub id: Uuid,
    pub filename: String,
    pub url: String,
    pub host: String,
    pub dir: PathBuf,
    pub path: Option<PathBuf>,
    pub size: Option<u64>,
    pub downloaded: u64,
    pub speed: f64,
    pub eta_secs: Option<u64>,
    pub status: Status,
    pub phase: Option<Phase>,
    pub error: Option<TaskError>,
    pub connections: usize,
    pub target: usize,
    pub segments: Vec<SegView>,
    pub splits: u64,
    pub history: Vec<f32>,
    pub route: Option<String>,
    pub created_at: u64,
    pub finished_at: Option<u64>,
    pub next_retry_at: Option<u64>,
    pub file_missing: bool,
    pub ranges: bool,
    pub avg_speed: f64,
    pub log: Vec<LogLine>,
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}
