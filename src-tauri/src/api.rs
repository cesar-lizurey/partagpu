use tauri::State;

use crate::auth::{AuthManager, RoomStatus};
use crate::discovery::Discovery;
use crate::resource::ResourceMonitor;
use crate::security_log::{EventCategory, SecurityLog};
use crate::sharing::{SharingConfig, SharingController};
use crate::task_runner::{IncomingTasks, OutgoingTasks, Task};
use crate::user_manager::{UserManager, UserStatus};
use std::sync::Mutex;

// ── Discovery ──────────────────────────────────────────────

#[tauri::command]
pub fn get_peers(discovery: State<'_, Discovery>) -> Vec<crate::discovery::Peer> {
    discovery.get_peers()
}

// ── Instance name ──────────────────────────────────────────

#[tauri::command]
pub fn get_display_name(discovery: State<'_, Discovery>) -> String {
    discovery.get_display_name()
}

#[tauri::command]
pub fn set_display_name(discovery: State<'_, Discovery>, name: String) -> String {
    let trimmed = name.trim();
    let final_name = if trimmed.is_empty() {
        hostname::get()
            .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into())
    } else {
        trimmed.to_string()
    };
    discovery.set_display_name(&final_name);
    final_name
}

// ── Room / TOTP auth ──────────────────────────────────────

#[derive(serde::Serialize)]
pub struct CreateRoomResult {
    pub passphrase: String,
    pub secret_base32: String,
}

#[tauri::command]
pub fn create_room(auth: State<'_, AuthManager>, room_name: String) -> Result<CreateRoomResult, String> {
    let name = room_name.trim();
    if name.is_empty() {
        return Err("Le nom de la salle est requis.".into());
    }
    let output = auth.create_room(name)?;
    Ok(CreateRoomResult {
        passphrase: output.passphrase,
        secret_base32: output.secret_base32,
    })
}

#[tauri::command]
pub fn join_room(auth: State<'_, AuthManager>, room_name: String, passphrase: String) -> Result<(), String> {
    let name = room_name.trim();
    if name.is_empty() {
        return Err("Le nom de la salle est requis.".into());
    }
    let p = passphrase.trim();
    if p.is_empty() {
        return Err("Le code d'accès est requis.".into());
    }
    auth.join_room(name, p)?;
    Ok(())
}

#[tauri::command]
pub fn leave_room(auth: State<'_, AuthManager>) {
    auth.leave_room();
}

#[tauri::command]
pub fn get_room_status(auth: State<'_, AuthManager>) -> RoomStatus {
    auth.get_status()
}

#[tauri::command]
pub fn get_room_secret(auth: State<'_, AuthManager>) -> Option<String> {
    auth.get_secret()
}

#[tauri::command]
pub fn verify_peer_code(auth: State<'_, AuthManager>, code: String) -> bool {
    auth.verify_code(code.trim())
}

// ── User management ───────────────────────────────────────

#[tauri::command]
pub fn get_user_status() -> UserStatus {
    UserManager::get_status()
}

#[tauri::command]
pub fn set_user_password(password: String) -> Result<String, String> {
    UserManager::set_password(&password)?;
    Ok("Mot de passe défini.".into())
}

// ── Resource monitoring ────────────────────────────────────

#[tauri::command]
pub fn get_resources(monitor: State<'_, std::sync::Arc<Mutex<ResourceMonitor>>>) -> crate::resource::ResourceUsage {
    monitor.lock().unwrap().snapshot()
}

// ── Sharing control ────────────────────────────────────────

#[tauri::command]
pub fn get_sharing_config(controller: State<'_, SharingController>) -> SharingConfig {
    controller.get_config()
}

#[tauri::command]
pub fn enable_sharing(controller: State<'_, SharingController>) -> Result<SharingConfig, String> {
    controller.enable()
}

#[tauri::command]
pub fn disable_sharing(controller: State<'_, SharingController>) -> Result<SharingConfig, String> {
    controller.disable()
}

#[tauri::command]
pub fn pause_sharing(controller: State<'_, SharingController>) -> Result<SharingConfig, String> {
    controller.pause()
}

#[tauri::command]
pub fn resume_sharing(controller: State<'_, SharingController>) -> Result<SharingConfig, String> {
    controller.resume()
}

#[tauri::command]
pub fn set_sharing_limits(
    controller: State<'_, SharingController>,
    cpu_percent: u32,
    ram_limit_mb: u64,
    gpu_percent: u32,
) -> Result<SharingConfig, String> {
    controller.set_limits(cpu_percent, ram_limit_mb, gpu_percent)
}

// ── Tasks ──────────────────────────────────────────────────

#[tauri::command]
pub fn get_incoming_tasks(tasks: State<'_, IncomingTasks>) -> Vec<Task> {
    tasks.list()
}

#[tauri::command]
pub fn get_outgoing_tasks(tasks: State<'_, OutgoingTasks>) -> Vec<Task> {
    tasks.list()
}

/// Submit a task for local sandboxed execution.
/// `args` is the command split into arguments: ["python3", "train.py", "--epochs", "10"]
/// Rejects tasks from unverified peers when a room is configured.
#[tauri::command]
pub fn submit_task(
    tasks: State<'_, IncomingTasks>,
    auth: State<'_, AuthManager>,
    discovery: State<'_, Discovery>,
    sec_log: State<'_, SecurityLog>,
    args: Vec<String>,
    source_machine: String,
    source_user: String,
    timeout_secs: Option<u64>,
) -> Result<Task, String> {
    if args.is_empty() {
        return Err("La commande ne peut pas être vide.".into());
    }

    // Block tasks from unverified peers when a room is active
    if auth.is_joined() {
        let peers = discovery.get_peers();
        let peer = peers.iter().find(|p| p.hostname == source_machine || p.ip == source_machine);
        match peer {
            Some(p) if !p.verified => {
                sec_log.peer_event(
                    EventCategory::TaskRejected,
                    &format!("Tâche refusée de {} ({}) : pair non vérifié — commande : {}",
                        source_machine, source_user, args.join(" ")),
                    &p.ip, &p.hostname,
                );
                return Err(format!(
                    "Tâche refusée : la machine « {} » n'est pas vérifiée. \
                    Elle doit rejoindre la salle avec le bon code d'accès.",
                    source_machine
                ));
            }
            None => {
                sec_log.log(
                    crate::security_log::EventLevel::Alert,
                    EventCategory::TaskRejected,
                    &format!("Tâche refusée de {} ({}) : pair inconnu — commande : {}",
                        source_machine, source_user, args.join(" ")),
                    Some(&source_machine), None,
                );
                return Err(format!(
                    "Tâche refusée : la machine « {} » est inconnue.",
                    source_machine
                ));
            }
            _ => {} // verified, proceed
        }
    }

    let cmd_str = args.join(" ");

    sec_log.log(
        crate::security_log::EventLevel::Info,
        EventCategory::TaskSubmitted,
        &format!("Tâche acceptée de {} ({}) : {}", source_machine, source_user, cmd_str),
        Some(&source_machine), None,
    );

    let task = Task {
        id: uuid::Uuid::new_v4().to_string(),
        command: cmd_str,
        args,
        source_machine,
        source_user,
        target_machine: hostname::get()
            .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "local".into()),
        status: crate::task_runner::TaskStatus::Queued,
        progress: 0.0,
        cpu_usage: 0.0,
        ram_usage_mb: 0,
        gpu_usage: 0.0,
        output: String::new(),
        error_output: String::new(),
        exit_code: None,
        created_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    };

    let task_id = task.id.clone();
    let task_clone = task.clone();
    tasks.add(task);

    tasks.execute(&task_id, timeout_secs.unwrap_or(3600))?;

    Ok(task_clone)
}

// ── Sandbox allowlist ─────────────────────────────────────

#[tauri::command]
pub fn get_allowlist(tasks: State<'_, IncomingTasks>) -> Vec<String> {
    tasks.get_sandbox().get_allowlist()
}

#[tauri::command]
pub fn add_to_allowlist(tasks: State<'_, IncomingTasks>, command: String) {
    let cmd = command.trim();
    if !cmd.is_empty() {
        tasks.get_sandbox().allow(cmd);
    }
}

#[tauri::command]
pub fn remove_from_allowlist(tasks: State<'_, IncomingTasks>, command: String) {
    tasks.get_sandbox().deny(command.trim());
}

#[tauri::command]
pub fn check_sandbox_available() -> bool {
    crate::sandbox::Sandbox::is_available()
}

// ── Security log ──────────────────────────────────────────

#[tauri::command]
pub fn get_security_log(
    sec_log: State<'_, SecurityLog>,
    since: Option<u64>,
) -> Vec<crate::security_log::SecurityEvent> {
    match since {
        Some(ts) => sec_log.get_since(ts),
        None => sec_log.get_all(),
    }
}

#[tauri::command]
pub fn clear_security_log(sec_log: State<'_, SecurityLog>) {
    sec_log.clear();
}

#[tauri::command]
pub fn get_machine_info(discovery: State<'_, Discovery>) -> Result<MachineInfo, String> {
    let hostname = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".into());

    let user = std::env::var("USER").unwrap_or_else(|_| "unknown".into());

    let display_name = discovery.get_display_name();

    Ok(MachineInfo {
        hostname,
        ip,
        user,
        display_name,
    })
}

#[derive(serde::Serialize)]
pub struct MachineInfo {
    pub hostname: String,
    pub ip: String,
    pub user: String,
    pub display_name: String,
}
