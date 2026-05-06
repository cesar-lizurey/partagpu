//! PartaGPU privilege helper — runs via pkexec with root privileges.
//!
//! This binary replaces the old bash helper. It performs privileged operations
//! (user management, cgroup setup, firewall rules) and is invoked by the main
//! PartaGPU application through pkexec.
//!
//! SECURITY: This binary runs as root. All inputs are validated strictly.

use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::os::unix::fs::chown;
use std::path::Path;
use std::process::{self, Command};

// ── Constants ──────────────────────────────────────────────

const PARTAGPU_USER: &str = "partagpu";
const PARTAGPU_HOME: &str = "/var/lib/partagpu";
const STATUS_DIR: &str = "/var/lib/partagpu-status";
const RESTRICTED_SHELL: &str = "/usr/local/lib/partagpu/partagpu-shell";
const SUDOERS_FILE: &str = "/etc/sudoers.d/partagpu-deny";
const SSHD_DENY_FILE: &str = "/etc/ssh/sshd_config.d/partagpu-deny.conf";
const PASSWORD_MAX_DAYS: u32 = 90;
const CGROUP_PATH: &str = "/sys/fs/cgroup/partagpu";
const APP_PORT: u16 = 7654;
const PEER_PORT: u16 = 7655;
const MDNS_PORT: u16 = 5353;
/// Rendezvous range for distributed training (NCCL/Gloo). Default torch.distributed
/// uses 29500; we open a small range so concurrent jobs don't collide.
const RENDEZVOUS_RANGE: &str = "29500:29510";

/// Managed Python venv used by the sandbox so users don't need to do
/// `sudo pip install --break-system-packages` on each peer. The venv is
/// installed once via the helper, then bind-mounted read-only inside the
/// sandbox at `/opt/partagpu-venv`.
const MANAGED_VENV: &str = "/var/lib/partagpu/venv";

// ── Helpers ────────────────────────────────────────────────

fn die(msg: &str) -> ! {
    eprintln!("Error: {msg}");
    process::exit(1);
}

fn run(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run_silent(cmd: &str, args: &[&str]) -> bool {
    Command::new(cmd)
        .args(args)
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn user_exists() -> bool {
    run_silent("id", &[PARTAGPU_USER])
}

fn get_user_shell() -> String {
    Command::new("getent")
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
        .unwrap_or_default()
}

fn mkdir_p(path: &str) {
    let _ = fs::create_dir_all(path);
}

fn write_file(path: &str, content: &str) {
    if let Err(e) = fs::write(path, content) {
        eprintln!("Warning: could not write {path}: {e}");
    }
}

fn set_permissions(path: &str, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(mode);
    let _ = fs::set_permissions(path, perms);
}

fn chown_to_user(path: &str, user: &str) {
    // Get uid/gid from passwd
    let output = Command::new("id").args(["-u", user]).output();
    let uid: u32 = output
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0);

    let output = Command::new("id").args(["-g", user]).output();
    let gid: u32 = output
        .ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse().ok())
        .unwrap_or(0);

    let _ = chown(path, Some(uid), Some(gid));
}

fn chown_recursive(path: &str, user: &str) {
    // Use system chown -R for simplicity and correctness
    let _ = Command::new("chown")
        .args(["-R", &format!("{user}:{user}"), path])
        .status();
}

fn validate_int(name: &str, value: &str) -> u64 {
    value
        .parse::<u64>()
        .unwrap_or_else(|_| die(&format!("{name} must be a positive integer, got: '{value}'")))
}

// ── Commands ───────────────────────────────────────────────

fn cmd_create_user() {
    // 1. Create or upgrade the user
    if user_exists() {
        let shell = get_user_shell();
        if shell == "/usr/sbin/nologin" || shell == "/bin/false" || shell == "/bin/bash" {
            // Upgrade to restricted shell
            install_restricted_shell();
            if !run("usermod", &["--shell", RESTRICTED_SHELL, PARTAGPU_USER]) {
                // Fallback to bash if restricted shell fails (e.g. not in /etc/shells)
                let _ = run("usermod", &["--shell", "/bin/bash", PARTAGPU_USER]);
            }
            println!("User {PARTAGPU_USER} shell set");
        } else {
            println!("User {PARTAGPU_USER} already exists");
        }
    } else {
        install_restricted_shell();
        if !run(
            "useradd",
            &[
                "--shell", RESTRICTED_SHELL,
                "--create-home",
                "--home-dir", PARTAGPU_HOME,
                "--comment", "PartaGPU — partage de calcul",
                PARTAGPU_USER,
            ],
        ) {
            // Fallback to bash
            let _ = run(
                "useradd",
                &[
                    "--shell", "/bin/bash",
                    "--create-home",
                    "--home-dir", PARTAGPU_HOME,
                    "--comment", "PartaGPU — partage de calcul",
                    PARTAGPU_USER,
                ],
            );
        }
        println!("User {PARTAGPU_USER} created");
    }

    // 2. Set password expiration (90 days)
    let _ = run("chage", &[
        "--maxdays", &PASSWORD_MAX_DAYS.to_string(),
        PARTAGPU_USER,
    ]);

    // 3. Block SSH access
    install_ssh_deny();

    // 4. Block sudo access
    install_sudoers_deny();

    // 5. Lock down home directory permissions (only partagpu can read)
    set_permissions(PARTAGPU_HOME, 0o700);

    // 6. Autostart
    cmd_setup_autostart();

    // 7. Status directory (world-readable for the marker file)
    mkdir_p(STATUS_DIR);
    set_permissions(STATUS_DIR, 0o755);

    // 8. Ensure ownership
    chown_recursive(PARTAGPU_HOME, PARTAGPU_USER);
}

/// Install a restricted shell script that only launches PartaGPU or a login session.
/// This prevents the user from getting an interactive bash shell via the login screen
/// while still allowing the desktop session to start.
fn install_restricted_shell() {
    let exec_path = ["/usr/bin/partagpu", "/usr/local/bin/partagpu"]
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "partagpu".to_string());

    // The restricted shell starts a minimal X/Wayland session with only PartaGPU.
    // If launched interactively (e.g. `su - partagpu`), it runs PartaGPU and exits.
    // If launched by a display manager, it acts as a session script.
    let shell_script = format!(
        r#"#!/bin/bash
# PartaGPU restricted shell — do not edit manually.
# This shell only allows running PartaGPU. No interactive access.

# If called with -c (e.g. by SSH or su -c), reject everything
case "$1" in
    -c)
        echo "Accès interactif interdit pour le compte partagpu." >&2
        exit 1
        ;;
esac

# Launch PartaGPU, then log out
exec {exec_path}
"#
    );

    let parent = Path::new(RESTRICTED_SHELL).parent().unwrap_or(Path::new("/"));
    mkdir_p(&parent.to_string_lossy());
    write_file(RESTRICTED_SHELL, &shell_script);
    set_permissions(RESTRICTED_SHELL, 0o755);

    // Register in /etc/shells so display managers accept it
    let shells_content = fs::read_to_string("/etc/shells").unwrap_or_default();
    if !shells_content.contains(RESTRICTED_SHELL) {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open("/etc/shells")
            .ok();
        if let Some(ref mut file) = f {
            use std::io::Write;
            let _ = writeln!(file, "{RESTRICTED_SHELL}");
        }
    }
}

/// Block SSH access for the partagpu user.
fn install_ssh_deny() {
    let sshd_dir = Path::new(SSHD_DENY_FILE).parent().unwrap_or(Path::new("/etc/ssh"));

    // Only write if the sshd_config.d directory exists (modern sshd)
    if sshd_dir.is_dir() {
        write_file(
            SSHD_DENY_FILE,
            &format!("# PartaGPU: block SSH access for the sharing account\nDenyUsers {PARTAGPU_USER}\n"),
        );
        set_permissions(SSHD_DENY_FILE, 0o644);
        // Reload sshd to apply (best effort)
        let _ = run_silent("systemctl", &["reload", "sshd"]);
        let _ = run_silent("systemctl", &["reload", "ssh"]);
        println!("SSH access blocked for {PARTAGPU_USER}");
    }
}

/// Explicitly deny sudo for the partagpu user.
fn install_sudoers_deny() {
    let content = format!(
        "# PartaGPU: explicitly deny sudo for the sharing account\n{PARTAGPU_USER} ALL=(ALL) !ALL\n"
    );
    write_file(SUDOERS_FILE, &content);
    set_permissions(SUDOERS_FILE, 0o440);
    println!("sudo blocked for {PARTAGPU_USER}");
}

fn cmd_set_password() {
    // Read password from stdin
    let stdin = io::stdin();
    let password = stdin
        .lock()
        .lines()
        .next()
        .and_then(|l| l.ok())
        .unwrap_or_default();

    if password.is_empty() {
        die("password is required");
    }
    if password.len() < 4 {
        die("password must be at least 4 characters");
    }
    if password.len() > 128 {
        die("password must be at most 128 characters");
    }
    if password.contains('\0') || password.contains('\r') {
        die("password contains invalid characters");
    }

    if !user_exists() {
        die(&format!("user {PARTAGPU_USER} does not exist. Create it first."));
    }

    // Pipe to chpasswd via stdin
    let mut child = Command::new("chpasswd")
        .stdin(process::Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| die(&format!("failed to run chpasswd: {e}")));

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = writeln!(stdin, "{PARTAGPU_USER}:{password}");
    }

    let status = child.wait().unwrap_or_else(|e| die(&format!("chpasswd error: {e}")));
    if !status.success() {
        die("chpasswd failed");
    }

    mkdir_p(STATUS_DIR);
    set_permissions(STATUS_DIR, 0o755);
    write_file(&format!("{STATUS_DIR}/password-set"), "");
    set_permissions(&format!("{STATUS_DIR}/password-set"), 0o644);

    println!("Password set for {PARTAGPU_USER}");
}

fn cmd_setup_autostart() {
    let autostart_dir = format!("{PARTAGPU_HOME}/.config/autostart");
    mkdir_p(&autostart_dir);

    let exec_path = ["/usr/bin/partagpu", "/usr/local/bin/partagpu"]
        .iter()
        .find(|p| Path::new(p).exists())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "partagpu".to_string());

    let desktop = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=PartaGPU\n\
         Comment=Partage de ressources de calcul\n\
         Exec={exec_path}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         StartupNotify=false\n"
    );

    let desktop_file = format!("{autostart_dir}/partagpu.desktop");
    write_file(&desktop_file, &desktop);
    set_permissions(&desktop_file, 0o644);
    chown_to_user(&desktop_file, PARTAGPU_USER);

    println!("Autostart configured");
}

fn cmd_remove_user() {
    if !user_exists() {
        println!("User {PARTAGPU_USER} does not exist");
        return;
    }

    let _ = run_silent("pkill", &["-u", PARTAGPU_USER]);
    std::thread::sleep(std::time::Duration::from_secs(1));

    let _ = run_silent("userdel", &["--remove", PARTAGPU_USER]);

    // Clean up hardening artifacts
    let _ = fs::remove_file(RESTRICTED_SHELL);
    let _ = fs::remove_file(SUDOERS_FILE);
    let _ = fs::remove_file(SSHD_DENY_FILE);
    let _ = run_silent("systemctl", &["reload", "sshd"]);
    let _ = run_silent("systemctl", &["reload", "ssh"]);

    // Remove restricted shell from /etc/shells
    if let Ok(content) = fs::read_to_string("/etc/shells") {
        let filtered: String = content
            .lines()
            .filter(|l| l.trim() != RESTRICTED_SHELL)
            .collect::<Vec<_>>()
            .join("\n") + "\n";
        let _ = fs::write("/etc/shells", filtered);
    }

    let _ = fs::remove_dir_all(STATUS_DIR);
    cmd_close_port();
    cmd_remove_cgroup();

    println!("User {PARTAGPU_USER} removed");
}

fn cmd_setup_cgroup(cpu_str: &str, ram_str: &str) {
    let cpu_percent = validate_int("cpu_percent", cpu_str);
    let ram_limit_mb = validate_int("ram_limit_mb", ram_str);

    if cpu_percent > 100 {
        die(&format!("cpu_percent must be 0-100, got: {cpu_percent}"));
    }

    mkdir_p(CGROUP_PATH);

    // Enable controllers
    let _ = fs::write("/sys/fs/cgroup/cgroup.subtree_control", "+cpu +memory");

    // CPU limit
    if cpu_percent > 0 && cpu_percent <= 100 {
        let quota = cpu_percent * 1000;
        write_file(
            &format!("{CGROUP_PATH}/cpu.max"),
            &format!("{quota} 100000"),
        );
    }

    // RAM limit
    let mem_val = if ram_limit_mb > 0 {
        format!("{ram_limit_mb}M")
    } else {
        "max".to_string()
    };
    write_file(&format!("{CGROUP_PATH}/memory.max"), &mem_val);

    // Grant calling user write access (PKEXEC_UID)
    if let Ok(uid_str) = env::var("PKEXEC_UID") {
        if let Ok(uid) = uid_str.parse::<u32>() {
            for file in ["cpu.max", "memory.max", "cgroup.procs"] {
                let path = format!("{CGROUP_PATH}/{file}");
                let _ = chown(&path, Some(uid), None);
            }
        }
    }

    println!("Cgroup configured: cpu={cpu_percent}% ram={ram_limit_mb}M");
}

fn cmd_remove_cgroup() {
    if Path::new(CGROUP_PATH).exists() {
        let _ = fs::remove_dir(CGROUP_PATH)
            .or_else(|_| fs::remove_dir_all(CGROUP_PATH));
    }
    println!("Cgroup removed");
}

fn cmd_open_port() {
    let app_tcp = format!("{APP_PORT}/tcp");
    let peer_tcp = format!("{PEER_PORT}/tcp");
    let rdv_tcp = format!("{RENDEZVOUS_RANGE}/tcp");
    let mdns_udp = format!("{MDNS_PORT}/udp");

    if which("ufw") {
        let _ = run_silent("ufw", &["allow", &app_tcp, "comment", "PartaGPU"]);
        let _ = run_silent("ufw", &["allow", &peer_tcp, "comment", "PartaGPU peer"]);
        let _ = run_silent("ufw", &["allow", &rdv_tcp, "comment", "PartaGPU DDP"]);
        let _ = run_silent("ufw", &["allow", &mdns_udp, "comment", "PartaGPU mDNS"]);
        println!(
            "Firewall opened (ufw): TCP {APP_PORT}, TCP {PEER_PORT}, TCP {RENDEZVOUS_RANGE}, UDP {MDNS_PORT}"
        );
    } else if which("iptables") {
        for port in [APP_PORT, PEER_PORT] {
            let comment = if port == APP_PORT { "PartaGPU" } else { "PartaGPU peer" };
            let _ = run_silent(
                "iptables",
                &["-D", "INPUT", "-p", "tcp", "--dport", &port.to_string(),
                  "-m", "comment", "--comment", comment, "-j", "ACCEPT"],
            );
            run(
                "iptables",
                &["-A", "INPUT", "-p", "tcp", "--dport", &port.to_string(),
                  "-m", "comment", "--comment", comment, "-j", "ACCEPT"],
            );
        }
        // Rendezvous range for distributed training
        let _ = run_silent(
            "iptables",
            &["-D", "INPUT", "-p", "tcp", "-m", "multiport", "--dports", RENDEZVOUS_RANGE,
              "-m", "comment", "--comment", "PartaGPU DDP", "-j", "ACCEPT"],
        );
        run(
            "iptables",
            &["-A", "INPUT", "-p", "tcp", "-m", "multiport", "--dports", RENDEZVOUS_RANGE,
              "-m", "comment", "--comment", "PartaGPU DDP", "-j", "ACCEPT"],
        );
        let _ = run_silent(
            "iptables",
            &["-D", "INPUT", "-p", "udp", "--dport", &MDNS_PORT.to_string(),
              "-m", "comment", "--comment", "PartaGPU mDNS", "-j", "ACCEPT"],
        );
        run(
            "iptables",
            &["-A", "INPUT", "-p", "udp", "--dport", &MDNS_PORT.to_string(),
              "-m", "comment", "--comment", "PartaGPU mDNS", "-j", "ACCEPT"],
        );
        println!(
            "Firewall opened (iptables): TCP {APP_PORT}, TCP {PEER_PORT}, TCP {RENDEZVOUS_RANGE}, UDP {MDNS_PORT}"
        );
    } else {
        println!("No firewall detected, skipping");
    }
}

fn cmd_close_port() {
    let app_tcp = format!("{APP_PORT}/tcp");
    let peer_tcp = format!("{PEER_PORT}/tcp");
    let rdv_tcp = format!("{RENDEZVOUS_RANGE}/tcp");

    if which("ufw") {
        let _ = run_silent("ufw", &["delete", "allow", &app_tcp]);
        let _ = run_silent("ufw", &["delete", "allow", &peer_tcp]);
        let _ = run_silent("ufw", &["delete", "allow", &rdv_tcp]);
        println!("Firewall closed (ufw): TCP {APP_PORT}, TCP {PEER_PORT}, TCP {RENDEZVOUS_RANGE}");
    } else if which("iptables") {
        for (port, comment) in [(APP_PORT, "PartaGPU"), (PEER_PORT, "PartaGPU peer")] {
            let _ = run_silent(
                "iptables",
                &["-D", "INPUT", "-p", "tcp", "--dport", &port.to_string(),
                  "-m", "comment", "--comment", comment, "-j", "ACCEPT"],
            );
        }
        let _ = run_silent(
            "iptables",
            &["-D", "INPUT", "-p", "tcp", "-m", "multiport", "--dports", RENDEZVOUS_RANGE,
              "-m", "comment", "--comment", "PartaGPU DDP", "-j", "ACCEPT"],
        );
        println!("Firewall closed (iptables): TCP {APP_PORT}, TCP {PEER_PORT}, TCP {RENDEZVOUS_RANGE}");
    } else {
        println!("No firewall detected, skipping");
    }
}

// ── Managed venv ──────────────────────────────────────────────────────────

fn cmd_setup_venv() {
    // Use the system python to create the venv.
    if !std::path::Path::new("/usr/bin/python3").exists() {
        die("/usr/bin/python3 introuvable. Installez python3 d'abord.");
    }

    // Ensure the parent dir exists and is owned by partagpu.
    if let Some(parent) = std::path::Path::new(MANAGED_VENV).parent() {
        mkdir_p(&parent.to_string_lossy());
    }

    // Create the venv (idempotent — `python3 -m venv` reuses an existing dir
    // if it already has a `bin/python` matching the current python).
    if !run("/usr/bin/python3", &["-m", "venv", MANAGED_VENV]) {
        die("python3 -m venv a échoué.");
    }

    let pip = format!("{MANAGED_VENV}/bin/pip");

    // Upgrade pip (best effort — old pip versions sometimes can't install
    // recent torch wheels). Failure is non-fatal.
    let _ = run(&pip, &["install", "--upgrade", "pip"]);

    // Curated "ML basics" toolkit — what most data science / ML courses
    // need on day one. Tradeoff : larger initial install (~3 GB vs ~2 GB
    // for torch alone) but the user doesn't have to manually pip install
    // pandas/sklearn/matplotlib for every task.
    //
    // - torch + torchvision : core deep learning + image utilities
    // - numpy + scipy       : foundation
    // - pandas              : tabular data
    // - scikit-learn        : classical ML
    // - matplotlib          : plots
    // - pillow              : image I/O (transitive of torchvision but
    //                         explicit for clarity / robustness)
    let packages = [
        "torch",
        "torchvision",
        "numpy",
        "scipy",
        "pandas",
        "scikit-learn",
        "matplotlib",
        "pillow",
    ];
    println!(
        "Installation de la toolkit ML : {} (peut prendre 5-10 minutes, ~3 Go)…",
        packages.join(" + ")
    );
    let mut args = vec!["install", "--upgrade"];
    args.extend(packages.iter().copied());
    if !run(&pip, &args) {
        die("pip install de la toolkit ML a échoué.");
    }

    // Hand ownership to partagpu so the sandbox UID can read the files
    // (technically, world-readable mode would suffice, but this keeps perms
    // clean if pip ever installs with mode 600 something).
    chown_recursive(MANAGED_VENV, PARTAGPU_USER);

    println!("Venv géré installé dans {MANAGED_VENV}");
}

fn cmd_remove_venv() {
    if std::path::Path::new(MANAGED_VENV).exists() {
        if let Err(e) = fs::remove_dir_all(MANAGED_VENV) {
            die(&format!("rm -rf {MANAGED_VENV} : {e}"));
        }
        println!("Venv géré supprimé.");
    } else {
        println!("Aucun venv géré à supprimer.");
    }
}

fn which(cmd: &str) -> bool {
    Command::new("which")
        .arg(cmd)
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ── Main ───────────────────────────────────────────────────

fn usage() -> ! {
    eprintln!("Usage: partagpu-helper <command> [args...]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  create-user              Create the partagpu login user");
    eprintln!("  set-password             Set or update the login password (reads from stdin)");
    eprintln!("  remove-user              Remove the partagpu user");
    eprintln!("  setup-cgroup <cpu%> <ram_mb>");
    eprintln!("  remove-cgroup");
    eprintln!("  open-port                Open firewall for PartaGPU");
    eprintln!("  close-port               Close firewall for PartaGPU");
    eprintln!("  setup-venv               Create /var/lib/partagpu/venv with torch + numpy");
    eprintln!("  remove-venv              Remove the managed venv");
    process::exit(1);
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        usage();
    }

    match args[1].as_str() {
        "create-user" => cmd_create_user(),
        "set-password" => cmd_set_password(),
        "remove-user" => cmd_remove_user(),
        "setup-cgroup" => {
            let cpu = args.get(2).map(|s| s.as_str()).unwrap_or("100");
            let ram = args.get(3).map(|s| s.as_str()).unwrap_or("0");
            cmd_setup_cgroup(cpu, ram);
        }
        "remove-cgroup" => cmd_remove_cgroup(),
        "open-port" => cmd_open_port(),
        "close-port" => cmd_close_port(),
        "setup-venv" => cmd_setup_venv(),
        "remove-venv" => cmd_remove_venv(),
        _ => {
            eprintln!("Unknown command: {}", args[1]);
            usage();
        }
    }
}
