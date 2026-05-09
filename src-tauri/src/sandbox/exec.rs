//! The actual bwrap pipeline : allowlist enforcement, mount construction,
//! GPU passthrough, child spawn, per-task sub-cgroup placement, live
//! stdout/stderr draining, timeout-aware wait and cleanup.

use std::collections::HashSet;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use super::util::{drain_stream, get_user_gid, get_user_uid, wait_with_timeout};
use super::workspace::prepare_workspace;
use super::{OutputSink, SandboxOptions, SandboxResult};

const BWRAP: &str = "/usr/bin/bwrap";
const PARTAGPU_USER: &str = "partagpu";
const CGROUP_PATH: &str = "/sys/fs/cgroup/partagpu";
const MAX_STDOUT_BYTES: usize = 1024 * 1024; // 1 MB
const MAX_STDERR_BYTES: usize = 256 * 1024; // 256 KB
/// Path of the managed Python venv on the host. If present, the sandbox
/// bind-mounts it read-only at `/opt/partagpu-venv` and prepends its `bin/`
/// to PATH so `python3` finds the venv version first (with torch + numpy
/// pre-installed) instead of the bare system python.
const MANAGED_VENV_HOST: &str = "/var/lib/partagpu/venv";
const MANAGED_VENV_SANDBOX: &str = "/opt/partagpu-venv";
/// CUDA MPS pipe directory, set up by the helper at sharing-enable time.
/// Bind-mounted rw into the sandbox at the same path so client processes
/// (running as the partagpu UID) can connect to the daemon's Unix socket.
const MPS_PIPE_DIR: &str = "/var/lib/partagpu/mps";

/// Verifie si un daemon CUDA MPS tourne sur l'hote en pgrep-ant le binaire.
/// Utilise au moment ou la sandbox decide d'injecter `CUDA_MPS_PIPE_DIRECTORY`
/// dans l'env de la tache : sans ce check, on injecterait l'env meme si le
/// daemon est mort, ce qui pousserait CUDA a echouer avec l'erreur 805.
fn is_mps_daemon_alive() -> bool {
    Command::new("pgrep")
        .args(["-x", "nvidia-cuda-mps-control"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Manages the allowlist and runs sandboxed commands.
#[derive(Clone)]
pub struct Sandbox {
    allowlist: Arc<Mutex<HashSet<String>>>,
}

impl Default for Sandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Sandbox {
    pub fn new() -> Self {
        let mut defaults = HashSet::new();
        for cmd in [
            "python3",
            "python",
            "pip3",
            "env", // useful for prefixing env vars in DDP launches
            "bash",
            "sh",
            "cat",
            "head",
            "tail",
            "wc",
            "sort",
            "uniq",
            "grep",
            "awk",
            "sed",
            "tar",
            "gzip",
            "gunzip",
            "nvidia-smi",
            "make",
            "cmake",
            "gcc",
            "g++",
            "rustc",
            "cargo",
            "julia",
            "Rscript",
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
        self.execute_with_callbacks(args, timeout_secs, opts, |_| {})
    }

    /// Same as `execute_with_options`, but invokes `on_pid` with the bwrap
    /// PID as soon as the child is spawned. Used by the task runner to
    /// register the PID for cancellation. If `sink` is provided, stdout and
    /// stderr are appended to it as the bytes arrive (live streaming);
    /// otherwise the sandbox uses internal buffers and only the final
    /// SandboxResult exposes the output.
    pub fn execute_with_callbacks<F>(
        &self,
        args: &[String],
        timeout_secs: u64,
        opts: &SandboxOptions,
        on_pid: F,
    ) -> Result<SandboxResult, String>
    where
        F: FnOnce(u32),
    {
        self.execute_with_callbacks_and_sink(args, timeout_secs, opts, on_pid, None)
    }

    /// Variant that also accepts an external `OutputSink` for live streaming.
    pub fn execute_with_callbacks_and_sink<F>(
        &self,
        args: &[String],
        timeout_secs: u64,
        opts: &SandboxOptions,
        on_pid: F,
        sink: Option<&OutputSink>,
    ) -> Result<SandboxResult, String>
    where
        F: FnOnce(u32),
    {
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
            workspace_host
                .path
                .to_str()
                .ok_or("chemin workspace non UTF-8")?,
            "/workspace",
        ]);
        cmd.args(["--chdir", "/workspace"]);

        cmd.args(["--tmpfs", "/tmp"]);

        // If the managed venv has been provisioned on the host, bind-mount it
        // read-only into the sandbox and prepend its `bin/` to PATH. This way
        // `python3` resolved by PATH lookup picks the venv version (with
        // torch + numpy pre-installed) instead of the bare system python.
        let managed_venv_active = Path::new(MANAGED_VENV_HOST).is_dir();
        if managed_venv_active {
            cmd.args(["--ro-bind", MANAGED_VENV_HOST, MANAGED_VENV_SANDBOX]);
        }
        let path_value = if managed_venv_active {
            format!("{MANAGED_VENV_SANDBOX}/bin:/usr/local/bin:/usr/bin:/bin")
        } else {
            "/usr/local/bin:/usr/bin:/bin".to_string()
        };
        cmd.env("PATH", &path_value);
        // Force unbuffered Python so users see prints in the live UI without
        // having to remember `flush=True`. Doesn't affect non-Python tasks.
        cmd.env("PYTHONUNBUFFERED", "1");

        // CUDA MPS integration : si le daemon MPS est actif, bind-mount le
        // pipe dir et exporte les env vars pour que la sandbox puisse parler
        // au control socket. `gpu_limit_percent` cape la part de SM via
        // `CUDA_MPS_ACTIVE_THREAD_PERCENTAGE` (l'enforcement reel du slider).
        //
        // Important : on verifie a la fois que le repertoire existe ET que
        // le daemon est vivant. Sans cette verification, si le daemon a
        // crash apres un setup-mps reussi, la sandbox injecterait quand
        // meme les env vars et chaque tache CUDA echouerait avec l'erreur
        // 805 (« MPS client failed to connect to the MPS control daemon »).
        // En l'absence de daemon vivant on retombe en mode CUDA direct
        // (advisory only, comme avant v1.10.2).
        let mps_active = Path::new(MPS_PIPE_DIR).is_dir() && is_mps_daemon_alive();
        if mps_active {
            cmd.args(["--bind", MPS_PIPE_DIR, MPS_PIPE_DIR]);
            cmd.env("CUDA_MPS_PIPE_DIRECTORY", MPS_PIPE_DIR);
            if let Some(pct) = opts.gpu_limit_percent {
                let clamped = pct.clamp(1, 100);
                cmd.env("CUDA_MPS_ACTIVE_THREAD_PERCENTAGE", clamped.to_string());
            }
        }

        if !opts.network_enabled {
            cmd.arg("--unshare-net");
        } else {
            // DNS : sur Ubuntu/Debian moderne, `/etc/resolv.conf` est un symlink
            // vers `/run/systemd/resolve/stub-resolv.conf`. On a deja bind-mount
            // `/etc` mais le symlink pointerait dans le vide sans ce bind ; sans
            // `getaddrinfo` echoue avec « Temporary failure in name resolution »
            // et les taches qui telechargent un dataset (CIFAR, etc.) plantent.
            if Path::new("/run/systemd/resolve").is_dir() {
                cmd.args(["--ro-bind", "/run/systemd/resolve", "/run/systemd/resolve"]);
            }
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

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Impossible de lancer bwrap : {e}"))?;

        let pid = child.id();
        on_pid(pid);

        // Per-task sub-cgroup. If creation succeeds, the bwrap process (and
        // all its children, thanks to cgroup inheritance) get isolated CPU
        // and memory accounting within the parent partagpu cgroup. If it
        // fails (cgroup not set up, perms wrong, kernel without cgroup v2),
        // we fall back to the parent cgroup.procs.
        let task_cgroup =
            std::path::Path::new(CGROUP_PATH).join(format!("task-{}", uuid::Uuid::new_v4()));
        let in_task_cgroup = std::fs::create_dir(&task_cgroup).is_ok();
        let procs_path = if in_task_cgroup {
            task_cgroup.join("cgroup.procs")
        } else {
            std::path::PathBuf::from(format!("{CGROUP_PATH}/cgroup.procs"))
        };
        let _ = std::fs::write(&procs_path, pid.to_string());

        // Live capture buffers. If the caller provided a sink, reuse it so the
        // task runner sees output in real time; otherwise use local buffers
        // that only matter at process exit.
        let stdout_buf: Arc<Mutex<String>> = sink
            .map(|s| s.stdout.clone())
            .unwrap_or_else(|| Arc::new(Mutex::new(String::new())));
        let stderr_buf: Arc<Mutex<String>> = sink
            .map(|s| s.stderr.clone())
            .unwrap_or_else(|| Arc::new(Mutex::new(String::new())));

        let stdout_handle = child.stdout.take().map(|out| {
            let buf = stdout_buf.clone();
            std::thread::spawn(move || drain_stream(out, buf, MAX_STDOUT_BYTES))
        });
        let stderr_handle = child.stderr.take().map(|err| {
            let buf = stderr_buf.clone();
            std::thread::spawn(move || drain_stream(err, buf, MAX_STDERR_BYTES))
        });

        let wait_result = wait_with_timeout(&mut child, timeout_secs);

        // After the child exits (or is killed), the pipes EOF; the reader
        // threads finish on their own. Join them to ensure all bytes are
        // captured before we read the buffers.
        if let Some(h) = stdout_handle {
            let _ = h.join();
        }
        if let Some(h) = stderr_handle {
            let _ = h.join();
        }

        // Avant de drop le workspace (qui le wipe), on rapatrie les artefacts
        // demandes via `SandboxOptions::outputs`. C'est la seule fenetre ou
        // les fichiers produits par la tache existent encore sur disque.
        // Done meme en cas d'echec : un training qui sauve un checkpoint puis
        // crash plus tard reste utile a recuperer.
        let artifacts =
            collect_artifacts(workspace_host.path.as_path(), &opts.outputs, &stderr_buf);

        let res = match wait_result {
            Ok(status) => {
                let stdout = stdout_buf.lock().map(|s| s.clone()).unwrap_or_default();
                let stderr = stderr_buf.lock().map(|s| s.clone()).unwrap_or_default();
                Ok(SandboxResult {
                    exit_code: status,
                    stdout,
                    stderr,
                    artifacts,
                })
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                Err(e)
            }
        };

        // Remove the per-task sub-cgroup. Should be empty after the bwrap
        // tree is gone (due to --die-with-parent). Fails silently if the
        // dir is somehow non-empty or never existed.
        if in_task_cgroup {
            let _ = std::fs::remove_dir(&task_cgroup);
        }

        // Manual cleanup (Drop on TempWorkspace also handles it, but be defensive).
        drop(workspace_host);
        res
    }
}

/// Lit chaque fichier demande dans `outputs` depuis le workspace host et
/// retourne la liste d'artefacts (path + contenu base64). Les chemins
/// suspects (path traversal, absolute, symlinks pointant hors du workspace)
/// sont rejetes sans bruit. Plafonne le total a `MAX_ARTIFACT_TOTAL_BYTES`
/// pour eviter qu'une tache qui ecrit un fichier geant n'epuise la RAM.
fn collect_artifacts(
    workspace_root: &Path,
    outputs: &[String],
    stderr_buf: &Arc<Mutex<String>>,
) -> Vec<crate::sandbox::Artifact> {
    use std::io::Read;

    if outputs.is_empty() {
        return Vec::new();
    }
    let canon_root = match workspace_root.canonicalize() {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };

    let mut out = Vec::new();
    let mut total_bytes: u64 = 0;
    for rel in outputs {
        // On rejette tout chemin absolu ou contenant `..` apres
        // canonicalize(), pour s'assurer qu'on ne lit que ce qui est dans
        // /workspace. Ca couvre aussi les symlinks fabriques par la tache
        // pour pointer hors-sandbox.
        let candidate = canon_root.join(rel);
        let canon = match candidate.canonicalize() {
            Ok(p) => p,
            Err(_) => continue, // fichier absent → skip silencieusement
        };
        if !canon.starts_with(&canon_root) {
            continue;
        }
        if !canon.is_file() {
            continue;
        }
        let metadata = match std::fs::metadata(&canon) {
            Ok(m) => m,
            Err(_) => continue,
        };
        let size = metadata.len();
        if total_bytes.saturating_add(size) > crate::sandbox::MAX_ARTIFACT_TOTAL_BYTES {
            if let Ok(mut buf) = stderr_buf.lock() {
                buf.push_str(&format!(
                    "\n[partagpu] Artefact « {rel} » ({size} octets) ignore : depasserait le plafond de {} octets.\n",
                    crate::sandbox::MAX_ARTIFACT_TOTAL_BYTES
                ));
            }
            continue;
        }
        let mut bytes = Vec::with_capacity(size as usize);
        if let Ok(mut f) = std::fs::File::open(&canon) {
            if f.read_to_end(&mut bytes).is_err() {
                continue;
            }
        } else {
            continue;
        }
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        let content_b64 = data_encoding::BASE64.encode(&bytes);
        out.push(crate::sandbox::Artifact {
            path: rel.clone(),
            content_b64,
        });
    }
    out
}
