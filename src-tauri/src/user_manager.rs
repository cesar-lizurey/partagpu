use std::fs;
use std::process::Command;

const PARTAGPU_USER: &str = "partagpu";
const CGROUP_PATH: &str = "/sys/fs/cgroup/partagpu";
const PASSWORD_MARKER: &str = "/var/lib/partagpu-status/password-set";

/// Path where the helper is installed system-wide.
const HELPER_INSTALLED: &str = "/usr/local/lib/partagpu/partagpu-helper";

/// Status of the partagpu user on this machine.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum UserStatus {
    /// User does not exist.
    Missing,
    /// User exists but has /usr/sbin/nologin (old setup).
    NoLogin,
    /// User exists with a shell but no password set yet.
    NoPassword,
    /// User is fully configured and can be logged into.
    Ready,
}

pub struct UserManager;

impl UserManager {
    fn helper_path() -> String {
        // 1. Installed system-wide (production .deb)
        if std::path::Path::new(HELPER_INSTALLED).exists() {
            return HELPER_INSTALLED.to_string();
        }

        // 2. Bundled next to the executable (AppImage)
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let candidate = dir.join("resources").join("partagpu-helper");
                if candidate.exists() {
                    return candidate.to_string_lossy().to_string();
                }
            }
        }

        // 3. Dev mode: built by cargo in the workspace target directory
        let target_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/target");
        for profile in ["debug", "release"] {
            let candidate = format!("{target_dir}/{profile}/partagpu-helper");
            if std::path::Path::new(&candidate).exists() {
                return candidate;
            }
        }

        HELPER_INSTALLED.to_string()
    }

    /// Run the helper script via pkexec (shows a native password dialog).
    fn run_helper(args: &[&str]) -> Result<String, String> {
        Self::run_helper_with_stdin(args, None)
    }

    /// Run the helper with optional data piped to stdin.
    fn run_helper_with_stdin(args: &[&str], stdin_data: Option<&str>) -> Result<String, String> {
        use std::io::Write;
        use std::process::Stdio;

        let helper = Self::helper_path();

        let mut child = Command::new("pkexec")
            .arg(&helper)
            .args(args)
            .stdin(if stdin_data.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => {
                    "pkexec n'est pas installé. Installez policykit-1 : sudo apt install policykit-1".to_string()
                }
                _ => format!("Impossible de lancer pkexec : {e}"),
            })?;

        if let Some(data) = stdin_data {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(data.as_bytes());
                // stdin is dropped here, closing the pipe
            }
        }

        let output = child.wait_with_output()
            .map_err(|e| format!("Erreur d'attente du helper : {e}"))?;

        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        } else {
            let code = output.status.code().unwrap_or(-1);
            if code == 126 {
                return Err("Authentification annulée par l'utilisateur.".to_string());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("Erreur du helper (code {code}) : {stderr}"))
        }
    }

    /// Check if the partagpu user exists (no privileges needed).
    pub fn user_exists() -> bool {
        Command::new("id")
            .arg(PARTAGPU_USER)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Query the status of the partagpu user — no root, no pkexec.
    pub fn get_status() -> UserStatus {
        if !Self::user_exists() {
            return UserStatus::Missing;
        }

        // Read the user's shell from /etc/passwd (readable by everyone)
        let shell = Command::new("getent")
            .args(["passwd", PARTAGPU_USER])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let line = String::from_utf8_lossy(&o.stdout).to_string();
                    line.trim().split(':').last().map(|s| s.to_string())
                } else {
                    None
                }
            })
            .unwrap_or_default();

        if shell == "/usr/sbin/nologin" || shell == "/bin/false" || shell.is_empty() {
            return UserStatus::NoLogin;
        }

        // Check if password was set via the marker file (readable by everyone)
        if !std::path::Path::new(PASSWORD_MARKER).exists() {
            return UserStatus::NoPassword;
        }

        UserStatus::Ready
    }

    /// Create the partagpu user (with a login shell).
    pub fn create_user() -> Result<(), String> {
        Self::run_helper(&["create-user"])?;
        Self::run_helper(&["setup-cgroup", "100", "0"])?;
        Ok(())
    }

    /// Set or update the password for the partagpu user.
    /// Password is sent via stdin to avoid exposure in /proc/*/cmdline.
    pub fn set_password(password: &str) -> Result<(), String> {
        if password.len() < 4 {
            return Err("Le mot de passe doit contenir au moins 4 caractères.".into());
        }
        if password.len() > 128 {
            return Err("Le mot de passe ne doit pas dépasser 128 caractères.".into());
        }
        if password.contains('\0') || password.contains('\n') || password.contains('\r') {
            return Err("Le mot de passe contient des caractères invalides.".into());
        }
        // Pass password via stdin, NOT as a CLI argument
        Self::run_helper_with_stdin(&["set-password"], Some(password))?;
        Ok(())
    }

    /// Remove the partagpu user entirely.
    pub fn remove_user() -> Result<(), String> {
        if !Self::user_exists() {
            return Ok(());
        }
        Self::run_helper(&["remove-user"])?;
        Ok(())
    }

    fn cgroup_is_writable() -> bool {
        let cpu_max = format!("{CGROUP_PATH}/cpu.max");
        fs::OpenOptions::new()
            .write(true)
            .open(&cpu_max)
            .is_ok()
    }

    /// Check if the cgroup directory exists.
    fn cgroup_exists() -> bool {
        std::path::Path::new(CGROUP_PATH).exists()
    }

    /// Adjust cgroup limits. Direct write if possible, pkexec fallback.
    /// If the cgroup doesn't exist and pkexec fails, returns Ok (best effort).
    pub fn setup_cgroup(
        cpu_percent: u32,
        ram_limit_mb: u64,
        _gpu_percent: u32,
    ) -> Result<(), String> {
        // Validate inputs before passing to the shell
        let cpu = cpu_percent.min(100);
        let ram = ram_limit_mb.min(1_048_576); // max 1 TB

        if Self::cgroup_is_writable() {
            Self::write_cgroup_limits(cpu, ram)?;
            return Ok(());
        }

        // If cgroup exists but isn't writable, or doesn't exist at all,
        // try pkexec. If that fails (e.g. no admin rights), log and continue.
        match Self::run_helper(&[
            "setup-cgroup",
            &cpu.to_string(),
            &ram.to_string(),
        ]) {
            Ok(_) => Ok(()),
            Err(e) if Self::cgroup_exists() => {
                // Cgroup exists from a previous setup, just can't adjust limits — acceptable
                eprintln!("Warning: could not adjust cgroup limits: {e}");
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    fn write_cgroup_limits(cpu_percent: u32, ram_limit_mb: u64) -> Result<(), String> {
        let cpu_max_path = format!("{CGROUP_PATH}/cpu.max");
        let mem_max_path = format!("{CGROUP_PATH}/memory.max");

        if cpu_percent > 0 && cpu_percent <= 100 {
            let quota = (cpu_percent as u64) * 1000;
            let val = format!("{quota} 100000");
            fs::write(&cpu_max_path, &val)
                .map_err(|e| format!("Impossible d'écrire dans {cpu_max_path} : {e}"))?;
        }

        let mem_val = if ram_limit_mb > 0 {
            format!("{}M", ram_limit_mb)
        } else {
            "max".to_string()
        };
        fs::write(&mem_max_path, &mem_val)
            .map_err(|e| format!("Impossible d'écrire dans {mem_max_path} : {e}"))?;

        Ok(())
    }

    pub fn cgroup_path() -> &'static str {
        CGROUP_PATH
    }

    /// Open the firewall for PartaGPU (TCP 7654 + mDNS).
    pub fn open_port() -> Result<(), String> {
        Self::run_helper(&["open-port"])?;
        Ok(())
    }

    /// Close the firewall for PartaGPU.
    pub fn close_port() -> Result<(), String> {
        Self::run_helper(&["close-port"])?;
        Ok(())
    }
}
