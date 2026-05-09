//! Task lifecycle for both directions of the dispatch flow.
//!
//! Two stores live here, behind the same module to share helpers (path
//! conventions, atomic save, per-task output truncation) :
//!
//! - [`IncomingTasks`] — tasks **another** machine submitted to us. We run
//!   them inside the local sandbox, expose progress, persist them across
//!   restarts. Lives in [`incoming`].
//! - [`OutgoingTasks`] — tasks **we** submitted to a peer. We mirror the
//!   peer's state into a local copy so the UI can show one unified list
//!   without round-trip latency. Lives in [`outgoing`].
//!
//! [`monitor`] holds the helpers the per-task resource monitor uses to
//! aggregate CPU/RAM/GPU across the bwrap process tree.

mod incoming;
mod monitor;
mod outgoing;

pub use incoming::IncomingTasks;
pub use outgoing::{OutgoingTasks, RemoteRef};

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::SystemTime;

/// Per-task output cap when persisting to disk. Avoids saving 1 MB stdout
/// for every task across restarts (x100 tasks would be 100 MB on disk).
pub(crate) const PERSIST_OUTPUT_CAP: usize = 50 * 1024;
pub(crate) const PERSIST_FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub(crate) fn config_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("partagpu")
}

pub(crate) fn save_atomic<T: Serialize>(path: &std::path::Path, value: &T) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(value).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(tmp, path)
}

/// Truncate output fields before persisting so a chatty task doesn't blow
/// up the saved file. Never modifies the in-memory Task — only the saved
/// copy.
///
/// Artefacts ne sont jamais persistes : ils peuvent atteindre des centaines
/// de Mo et n'ont d'utilite que tant que le client est connecte pour les
/// recuperer. Apres un restart, on garde le statut/exit_code/output mais on
/// drop les blobs.
pub(crate) fn task_for_persist(task: &Task) -> Task {
    let mut t = task.clone();
    if t.output.len() > PERSIST_OUTPUT_CAP {
        t.output.truncate(PERSIST_OUTPUT_CAP);
        t.output.push_str("\n[…tronqué pour persistance]");
    }
    if t.error_output.len() > PERSIST_OUTPUT_CAP {
        t.error_output.truncate(PERSIST_OUTPUT_CAP);
        t.error_output.push_str("\n[…tronqué pour persistance]");
    }
    t.artifacts.clear();
    t
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub command: String,
    pub args: Vec<String>,
    pub source_machine: String,
    pub source_user: String,
    pub target_machine: String,
    pub status: TaskStatus,
    pub progress: f32,
    pub cpu_usage: f32,
    pub ram_usage_mb: u64,
    pub gpu_usage: f32,
    pub output: String,
    pub error_output: String,
    pub exit_code: Option<i32>,
    pub created_at: u64,
    /// Unix timestamp (seconds) at which the task transitioned to a terminal
    /// state (Completed / Failed / Cancelled). `None` while the task is
    /// still Queued or Running. Combined with `created_at` it gives the
    /// total wall-clock time visible in the UI.
    #[serde(default)]
    pub ended_at: Option<u64>,
    /// Whether the sandbox was launched with host network access (DDP rendezvous).
    /// Surfaced to the UI as a "network" indicator.
    #[serde(default)]
    pub network_enabled: bool,
    /// Artefacts rapatries depuis /workspace en fin de tache (par ex. un
    /// `model.pt` save par le rang 0). Vide tant que la tache n'a pas
    /// termine ; jamais persiste sur disque (truncate dans
    /// `task_for_persist`) — les blobs ne survivent pas a un restart.
    #[serde(default)]
    pub artifacts: Vec<crate::sandbox::Artifact>,
}

pub(crate) fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Build a fresh Task with default lifecycle fields.
pub fn new_task(
    args: Vec<String>,
    source_machine: String,
    source_user: String,
    target_machine: String,
) -> Task {
    let cmd_str = args.join(" ");
    Task {
        id: uuid::Uuid::new_v4().to_string(),
        command: cmd_str,
        args,
        source_machine,
        source_user,
        target_machine,
        status: TaskStatus::Queued,
        progress: 0.0,
        cpu_usage: 0.0,
        ram_usage_mb: 0,
        gpu_usage: 0.0,
        output: String::new(),
        error_output: String::new(),
        exit_code: None,
        created_at: now_secs(),
        ended_at: None,
        network_enabled: false,
        artifacts: Vec::new(),
    }
}
