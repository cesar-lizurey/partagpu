//! Sandbox execution for remote tasks using bubblewrap (bwrap).
//!
//! Each task runs inside a minimal sandbox with:
//! - Read-only filesystem (only /usr, /lib, /bin, /etc are visible)
//! - Writable workspace at /workspace (a tmpfs by default, or a host-bind
//!   directory pre-populated with files when the requester sends them)
//! - GPU passthrough: /dev/nvidia* are bind-mounted so CUDA works
//! - No network access by default. Can be opted into per-task (`network_enabled`)
//!   for distributed training (DDP rendezvous)
//! - Runs as the partagpu user, confined to the partagpu cgroup
//! - Only allowlisted executables can be invoked
//!
//! Module layout :
//! - [`exec`] : the [`Sandbox`] type, allowlist management, and the bwrap
//!   invocation pipeline (validate → bind mounts → spawn → wait → cleanup).
//! - [`workspace`] : `prepare_workspace` (host-side scratch dir for the
//!   bind-mount) and [`compress_workspace`] (gzip the per-file payload
//!   before encryption on the wire).
//! - [`util`] : tiny helpers (UTF-8-safe stream draining, partagpu UID
//!   lookup, child wait with timeout).

mod exec;
mod util;
mod workspace;

pub use exec::Sandbox;
pub use workspace::compress_workspace;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// A file pushed by the requester into the sandbox workspace before exec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    /// Relative path inside the workspace (no leading `/`, no `..`).
    pub path: String,
    /// File contents, base64-encoded. If `compression == Some("gzip")`, the
    /// underlying bytes (after base64 decode) are gzipped — the peer must
    /// decompress before writing the file.
    pub content_b64: String,
    /// Optional compression scheme. Currently only `"gzip"` is recognised.
    /// Absent / `None` means raw bytes (legacy format from older clients).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compression: Option<String>,
}

/// Options for a single sandbox execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SandboxOptions {
    /// If true, the sandbox uses the host network namespace. Required for
    /// distributed training (NCCL/Gloo rendezvous over the LAN).
    #[serde(default)]
    pub network_enabled: bool,
    /// Files to materialize in /workspace before exec (for scripts, configs).
    #[serde(default)]
    pub workspace: Vec<WorkspaceFile>,
    /// GPU SM% cap (1-100) honored by the CUDA driver via
    /// `CUDA_MPS_ACTIVE_THREAD_PERCENTAGE` when the MPS daemon is running
    /// on the host. `None` → no cap injected (advisory mode). Has no
    /// effect on hosts without an NVIDIA GPU.
    #[serde(default)]
    pub gpu_limit_percent: Option<u32>,
    /// Chemins relatifs (depuis /workspace) à rapatrier après exit. Permet
    /// au client SDK de récupérer des artefacts produits par la tâche
    /// (typiquement `model.pt` après un entraînement) avant que le sandbox
    /// ne soit détruit. Les fichiers absents sont silencieusement ignorés.
    /// Limite agrégée : `MAX_ARTIFACT_TOTAL_BYTES`.
    #[serde(default)]
    pub outputs: Vec<String>,
}

/// Un artefact rapatrie d'une tache : chemin relatif dans /workspace +
/// contenu brut binaire (transcode en base64 sur le wire).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Chemin relatif (tel que demandé via `SandboxOptions::outputs`).
    pub path: String,
    /// Contenu binaire encodé en base64 standard.
    pub content_b64: String,
}

/// Plafond agrégé sur les artefacts retournés par tâche. 256 MiB binaire
/// → ~340 MiB en base64 ; au-delà on tronque la liste avec un avertissement.
pub const MAX_ARTIFACT_TOTAL_BYTES: u64 = 256 * 1024 * 1024;

/// Result of a sandboxed task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    /// Artefacts rapatries depuis /workspace avant le cleanup. Les fichiers
    /// absents/inaccessibles sont silencieusement skippes (le client peut
    /// detecter via `path not in result.artifacts`).
    #[serde(default)]
    pub artifacts: Vec<Artifact>,
}

/// Shared buffers the sandbox progressively fills with stdout / stderr as the
/// process runs (instead of only at exit). Lets the task runner expose live
/// output to clients polling the task status.
#[derive(Clone)]
pub struct OutputSink {
    pub stdout: Arc<Mutex<String>>,
    pub stderr: Arc<Mutex<String>>,
}

impl Default for OutputSink {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputSink {
    pub fn new() -> Self {
        Self {
            stdout: Arc::new(Mutex::new(String::new())),
            stderr: Arc::new(Mutex::new(String::new())),
        }
    }

    pub fn snapshot(&self) -> (String, String) {
        let out = self.stdout.lock().map(|s| s.clone()).unwrap_or_default();
        let err = self.stderr.lock().map(|s| s.clone()).unwrap_or_default();
        (out, err)
    }
}
