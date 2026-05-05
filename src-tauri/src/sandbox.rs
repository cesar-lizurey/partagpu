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

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const BWRAP: &str = "/usr/bin/bwrap";
const PARTAGPU_USER: &str = "partagpu";
const CGROUP_PATH: &str = "/sys/fs/cgroup/partagpu";
const MAX_STDOUT_BYTES: usize = 1024 * 1024; // 1 MB
const MAX_STDERR_BYTES: usize = 256 * 1024; // 256 KB
const MAX_WORKSPACE_BYTES: u64 = 16 * 1024 * 1024; // 16 MB total

/// A file pushed by the requester into the sandbox workspace before exec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceFile {
    /// Relative path inside the workspace (no leading `/`, no `..`).
    pub path: String,
    /// File contents, base64-encoded.
    pub content_b64: String,
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
}

/// Result of a sandboxed task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Manages the allowlist and runs sandboxed commands.
#[derive(Clone)]
pub struct Sandbox {
    allowlist: Arc<Mutex<HashSet<String>>>,
}

impl Sandbox {
    pub fn new() -> Self {
        let mut defaults = HashSet::new();
        for cmd in [
            "python3", "python", "pip3",
            "env",  // useful for prefixing env vars in DDP launches
            "bash",
            "sh",
            "cat", "head", "tail", "wc", "sort", "uniq", "grep", "awk", "sed",
            "tar", "gzip", "gunzip",
            "nvidia-smi",
            "make", "cmake", "gcc", "g++", "rustc", "cargo",
            "julia", "Rscript",
        ] {
            defaults.insert(cmd.to_string());
        }

        Self {
            allowlist: Arc::new(Mutex::new(defaults)),
        }
    }

    pub fn get_allowlist(&self) -> Vec<String> {
        let list = self.allowlist.lock().unwrap();
        let mut v: Vec<String> = list.iter().cloned().collect();
        v.sort();
        v
    }

    pub fn allow(&self, cmd: &str) {
        self.allowlist.lock().unwrap().insert(cmd.to_string());
    }

    pub fn deny(&self, cmd: &str) {
        self.allowlist.lock().unwrap().remove(cmd);
    }

    pub fn is_available() -> bool {
        Path::new(BWRAP).exists()
    }

    fn validate_command(&self, args: &[String]) -> Result<(), String> {
        if args.is_empty() {
            return Err("Commande vide.".into());
        }
        let exe = Path::new(&args[0])
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| args[0].clone());

        let list = self.allowlist.lock().unwrap();
        if !list.contains(&exe) {
            return Err(format!(
                "Commande refusée : « {exe} » n'est pas dans la liste autorisée. \
                Commandes autorisées : {}",
                list.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(())
    }

    /// Execute a command inside the sandbox.
    pub fn execute(&self, args: &[String], timeout_secs: u64) -> Result<SandboxResult, String> {
        self.execute_with_options(args, timeout_secs, &SandboxOptions::default())
    }

    /// Execute a command with extra options (network, workspace files).
    pub fn execute_with_options(
        &self,
        args: &[String],
        timeout_secs: u64,
        opts: &SandboxOptions,
    ) -> Result<SandboxResult, String> {
        if args.is_empty() {
            return Err("Commande vide.".into());
        }
        self.validate_command(args)?;

        if !Self::is_available() {
            return Err(
                "bubblewrap (bwrap) n'est pas installé. Installez-le : sudo apt install bubblewrap"
                    .into(),
            );
        }

        // Materialize the workspace dir on the host (gets bind-mounted as /workspace).
        // We always create one — even when no files are sent — so the sandbox has a
        // writable cwd backed by the host (cleaner than --tmpfs for partagpu user perms).
        let workspace_host = prepare_workspace(&opts.workspace)?;

        // Build the bwrap command
        let mut cmd = Command::new(BWRAP);

        for dir in ["/usr", "/lib", "/lib64", "/bin", "/sbin", "/etc"] {
            if Path::new(dir).exists() {
                cmd.args(["--ro-bind", dir, dir]);
            }
        }

        cmd.args(["--proc", "/proc"]);
        cmd.args(["--dev", "/dev"]);

        // GPU device passthrough — needed for CUDA inside the sandbox.
        // /dev/nvidia0, /dev/nvidiactl, /dev/nvidia-uvm, /dev/nvidia-modeset, etc.
        if let Ok(entries) = std::fs::read_dir("/dev") {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let n = name.to_string_lossy();
                if n.starts_with("nvidia") {
                    let p = format!("/dev/{n}");
                    cmd.args(["--dev-bind", &p, &p]);
                }
            }
        }

        // Writable workspace: bind the host-prepared directory.
        cmd.args([
            "--bind",
            workspace_host.path.to_str().ok_or("chemin workspace non UTF-8")?,
            "/workspace",
        ]);
        cmd.args(["--chdir", "/workspace"]);

        cmd.args(["--tmpfs", "/tmp"]);

        if !opts.network_enabled {
            cmd.arg("--unshare-net");
        }
        cmd.arg("--unshare-pid");
        cmd.arg("--die-with-parent");
        cmd.arg("--new-session");

        let uid = get_user_uid(PARTAGPU_USER);
        let gid = get_user_gid(PARTAGPU_USER);
        if uid > 0 {
            cmd.args(["--uid", &uid.to_string()]);
            cmd.args(["--gid", &gid.to_string()]);
        }

        cmd.arg("--");
        cmd.args(args);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn().map_err(|e| format!("Impossible de lancer bwrap : {e}"))?;

        let pid = child.id();
        let procs_path = format!("{CGROUP_PATH}/cgroup.procs");
        let _ = std::fs::write(&procs_path, pid.to_string());

        let result = wait_with_timeout(&mut child, timeout_secs);

        // Always clean up the workspace directory (drops on scope, but be explicit
        // about errors so a leak is logged).
        let res = match result {
            Ok(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }

                stdout.truncate(MAX_STDOUT_BYTES);
                stderr.truncate(MAX_STDERR_BYTES);

                Ok(SandboxResult {
                    exit_code: status,
                    stdout,
                    stderr,
                })
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        };

        // Manual cleanup (Drop on TempWorkspace also handles it, but be defensive).
        drop(workspace_host);
        res
    }
}

// ── Workspace materialization ──────────────────────────────────────────────

/// A scratch directory on the host that's bind-mounted as /workspace inside
/// the sandbox. Cleaned up when this struct is dropped.
struct TempWorkspace {
    path: PathBuf,
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn prepare_workspace(files: &[WorkspaceFile]) -> Result<TempWorkspace, String> {
    // /var/lib/partagpu is owned by the partagpu user and gets the right perms;
    // fall back to /tmp if that doesn't work in dev mode.
    let base = if Path::new("/var/lib/partagpu").is_dir() {
        PathBuf::from("/var/lib/partagpu")
    } else {
        std::env::temp_dir()
    };

    let dir = base.join(format!("task-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("création workspace : {e}"))?;

    // Make it writable by the partagpu user (the sandbox runs as that uid).
    let _ = Command::new("chown")
        .args([
            "-R",
            &format!("{PARTAGPU_USER}:{PARTAGPU_USER}"),
            dir.to_str().unwrap_or("."),
        ])
        .status();
    let _ = Command::new("chmod")
        .args(["770", dir.to_str().unwrap_or(".")])
        .status();

    let mut total_bytes: u64 = 0;

    for f in files {
        let safe = sanitize_relative_path(&f.path)?;
        let full = dir.join(&safe);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", f.path))?;
        }
        let bytes = data_encoding::BASE64
            .decode(f.content_b64.as_bytes())
            .map_err(|e| format!("base64 invalide pour {}: {e}", f.path))?;
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_WORKSPACE_BYTES {
            return Err(format!(
                "workspace dépasse la limite de {} octets",
                MAX_WORKSPACE_BYTES
            ));
        }
        std::fs::write(&full, &bytes).map_err(|e| format!("écriture {}: {e}", f.path))?;
        let _ = Command::new("chown")
            .args([
                &format!("{PARTAGPU_USER}:{PARTAGPU_USER}"),
                full.to_str().unwrap_or("."),
            ])
            .status();
    }

    Ok(TempWorkspace { path: dir })
}

/// Validate a workspace-relative path: no absolute, no `..`, no NUL.
fn sanitize_relative_path(p: &str) -> Result<PathBuf, String> {
    if p.is_empty() {
        return Err("chemin workspace vide".into());
    }
    if p.contains('\0') {
        return Err("chemin workspace contient un NUL".into());
    }
    let path = PathBuf::from(p);
    if path.is_absolute() {
        return Err(format!("chemin workspace doit être relatif : {p}"));
    }
    for comp in path.components() {
        use std::path::Component::*;
        match comp {
            Normal(_) | CurDir => {}
            ParentDir => return Err(format!("chemin workspace contient '..' : {p}")),
            RootDir | Prefix(_) => return Err(format!("chemin workspace invalide : {p}")),
        }
    }
    Ok(path)
}

// ── Misc ──────────────────────────────────────────────────────────────────

fn get_user_uid(user: &str) -> u32 {
    Command::new("id")
        .args(["-u", user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

fn get_user_gid(user: &str) -> u32 {
    Command::new("id")
        .args(["-g", user])
        .output()
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0)
}

fn wait_with_timeout(child: &mut std::process::Child, timeout_secs: u64) -> Result<i32, String> {
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(timeout_secs);

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
            Ok(None) => {
                if Instant::now() > deadline {
                    return Err(format!(
                        "Tâche interrompue : dépassement du délai de {timeout_secs} secondes."
                    ));
                }
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => return Err(format!("Erreur d'attente : {e}")),
        }
    }
}
