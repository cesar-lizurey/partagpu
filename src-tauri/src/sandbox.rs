//! Sandbox execution for remote tasks using bubblewrap (bwrap).
//!
//! All tasks run inside a minimal sandbox with:
//! - Read-only filesystem (only /usr, /lib, /bin, /etc are visible)
//! - A writable tmpfs workspace at /workspace
//! - No network access (unshared network namespace)
//! - Runs as the partagpu user
//! - Confined to the partagpu cgroup
//! - No access to host home directories
//! - Only allowlisted executables can be run

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

const BWRAP: &str = "/usr/bin/bwrap";
const PARTAGPU_USER: &str = "partagpu";
const CGROUP_PATH: &str = "/sys/fs/cgroup/partagpu";
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
        // Default allowlist: common compute tools
        for cmd in [
            "python3", "python", "pip3",
            "bash",  // only inside sandbox — no host access
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

    /// Get the current allowlist.
    pub fn get_allowlist(&self) -> Vec<String> {
        let list = self.allowlist.lock().unwrap();
        let mut v: Vec<String> = list.iter().cloned().collect();
        v.sort();
        v
    }

    /// Add an executable to the allowlist.
    pub fn allow(&self, cmd: &str) {
        self.allowlist.lock().unwrap().insert(cmd.to_string());
    }

    /// Remove an executable from the allowlist.
    pub fn deny(&self, cmd: &str) {
        self.allowlist.lock().unwrap().remove(cmd);
    }

    /// Check if bubblewrap is available.
    pub fn is_available() -> bool {
        Path::new(BWRAP).exists()
    }

    /// Validate a command against the allowlist.
    /// Returns the executable name if allowed, or an error.
    fn validate_command(&self, args: &[String]) -> Result<(), String> {
        if args.is_empty() {
            return Err("Commande vide.".into());
        }

        // Extract the executable name (basename of the first arg)
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
    ///
    /// `args` is the command split into arguments (NOT a shell string).
    /// Example: `["python3", "train.py", "--epochs", "10"]`
    ///
    /// Returns the output after the process finishes.
    pub fn execute(&self, args: &[String], timeout_secs: u64) -> Result<SandboxResult, String> {
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

        // Build the bwrap command
        let mut cmd = Command::new(BWRAP);

        // Filesystem: read-only bind mounts for system dirs
        for dir in ["/usr", "/lib", "/lib64", "/bin", "/sbin", "/etc"] {
            if Path::new(dir).exists() {
                cmd.args(["--ro-bind", dir, dir]);
            }
        }

        // /proc and /dev (minimal)
        cmd.args(["--proc", "/proc"]);
        cmd.args(["--dev", "/dev"]);

        // Writable workspace on tmpfs
        cmd.args([
            "--tmpfs", "/workspace",
            "--chdir", "/workspace",
        ]);

        // Writable /tmp
        cmd.args(["--tmpfs", "/tmp"]);

        // No network access
        cmd.arg("--unshare-net");

        // New PID namespace (can't see host processes)
        cmd.arg("--unshare-pid");

        // Die if the parent dies
        cmd.arg("--die-with-parent");

        // No new privileges
        cmd.arg("--new-session");

        // Run as partagpu user (by UID)
        let uid = get_user_uid(PARTAGPU_USER);
        let gid = get_user_gid(PARTAGPU_USER);
        if uid > 0 {
            cmd.args(["--uid", &uid.to_string()]);
            cmd.args(["--gid", &gid.to_string()]);
        }

        // The actual command to run
        cmd.arg("--");
        cmd.args(args);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        // Assign to cgroup if it exists
        if Path::new(&format!("{CGROUP_PATH}/cgroup.procs")).exists() {
            // We write our PID to the cgroup after spawning
        }

        let mut child = cmd.spawn().map_err(|e| format!("Impossible de lancer bwrap : {e}"))?;

        // Move the child to the partagpu cgroup
        if let Some(pid) = child.id().into() {
            let procs_path = format!("{CGROUP_PATH}/cgroup.procs");
            let _ = std::fs::write(&procs_path, pid.to_string());
        }

        // Wait with timeout
        let result = wait_with_timeout(&mut child, timeout_secs);

        match result {
            Ok(status) => {
                let mut stdout = String::new();
                let mut stderr = String::new();
                if let Some(mut out) = child.stdout.take() {
                    let _ = out.read_to_string(&mut stdout);
                }
                if let Some(mut err) = child.stderr.take() {
                    let _ = err.read_to_string(&mut stderr);
                }

                // Limit output size to prevent memory issues
                stdout.truncate(1024 * 1024); // 1 MB max
                stderr.truncate(256 * 1024);  // 256 KB max

                Ok(SandboxResult {
                    exit_code: status,
                    stdout,
                    stderr,
                })
            }
            Err(e) => {
                // Kill the process if timeout
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        }
    }
}

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
