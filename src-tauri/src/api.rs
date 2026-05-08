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

// ── Room / HMAC auth ──────────────────────────────────────

#[derive(serde::Serialize)]
pub struct CreateRoomResult {
    pub passphrase: String,
    pub secret_base32: String,
}

#[tauri::command]
pub fn create_room(
    auth: State<'_, AuthManager>,
    discovery: State<'_, Discovery>,
    room_name: String,
) -> Result<CreateRoomResult, String> {
    let name = room_name.trim();
    if name.is_empty() {
        return Err("Le nom de la salle est requis.".into());
    }
    let output = auth.create_room(name)?;
    discovery.force_refresh_announcement();
    Ok(CreateRoomResult {
        passphrase: output.passphrase,
        secret_base32: output.secret_base32,
    })
}

#[tauri::command]
pub fn join_room(
    auth: State<'_, AuthManager>,
    discovery: State<'_, Discovery>,
    room_name: String,
    passphrase: String,
) -> Result<(), String> {
    let name = room_name.trim();
    if name.is_empty() {
        return Err("Le nom de la salle est requis.".into());
    }
    let p = passphrase.trim();
    if p.is_empty() {
        return Err("Le code d'accès est requis.".into());
    }
    auth.join_room(name, p)?;
    discovery.force_refresh_announcement();
    Ok(())
}

#[tauri::command]
pub fn leave_room(auth: State<'_, AuthManager>, discovery: State<'_, Discovery>) {
    auth.leave_room();
    discovery.force_refresh_announcement();
}

#[tauri::command]
pub fn get_room_status(auth: State<'_, AuthManager>) -> RoomStatus {
    auth.get_status()
}

#[tauri::command]
pub fn get_room_secret(auth: State<'_, AuthManager>) -> Option<String> {
    auth.get_secret()
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
pub fn get_resources(
    monitor: State<'_, std::sync::Arc<Mutex<ResourceMonitor>>>,
) -> crate::resource::ResourceUsage {
    monitor.lock().unwrap().snapshot()
}

/// Snapshot of the recent resource history (last ~5 min, sampled every 5 s).
/// Used by the UI to render CPU/RAM/GPU sparklines.
#[tauri::command]
pub fn get_resource_history(
    monitor: State<'_, std::sync::Arc<Mutex<ResourceMonitor>>>,
) -> Vec<crate::resource::ResourceSample> {
    monitor.lock().unwrap().history()
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
#[allow(clippy::too_many_arguments)] // Tauri commands need every State + every payload field
pub fn submit_task(
    tasks: State<'_, IncomingTasks>,
    auth: State<'_, AuthManager>,
    discovery: State<'_, Discovery>,
    sharing: State<'_, SharingController>,
    sec_log: State<'_, SecurityLog>,
    args: Vec<String>,
    source_machine: String,
    source_user: String,
    timeout_secs: Option<u64>,
    network_enabled: Option<bool>,
    workspace: Option<Vec<crate::sandbox::WorkspaceFile>>,
    outputs: Option<Vec<String>>,
) -> Result<Task, String> {
    if args.is_empty() {
        return Err("La commande ne peut pas être vide.".into());
    }

    // Block tasks from unverified peers when a room is active
    if auth.is_joined() {
        let peers = discovery.get_peers();
        let peer = peers
            .iter()
            .find(|p| p.hostname == source_machine || p.ip == source_machine);
        match peer {
            Some(p) if !p.verified => {
                sec_log.peer_event(
                    EventCategory::TaskRejected,
                    &format!(
                        "Tâche refusée de {} ({}) : pair non vérifié — commande : {}",
                        source_machine,
                        source_user,
                        args.join(" ")
                    ),
                    &p.ip,
                    &p.hostname,
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
                    &format!(
                        "Tâche refusée de {} ({}) : pair inconnu — commande : {}",
                        source_machine,
                        source_user,
                        args.join(" ")
                    ),
                    Some(&source_machine),
                    None,
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
        &format!(
            "Tâche acceptée de {} ({}) : {}",
            source_machine, source_user, cmd_str
        ),
        Some(&source_machine),
        None,
    );

    let target_machine = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "local".into());

    // Local /api/dispatch is the loopback path used when the user runs
    // `partagpu.run_remote(local, ...)` against their own machine. The
    // local sharing config gates whether the task even runs ; we read the
    // GPU cap from there so the same MPS env var is set as for an
    // incoming dispatch from a remote peer.
    let gpu_limit = {
        let cfg = sharing.get_config();
        if cfg.gpu_limit_percent > 0 && cfg.gpu_limit_percent < 100 {
            Some(cfg.gpu_limit_percent)
        } else {
            None
        }
    };
    let options = crate::sandbox::SandboxOptions {
        network_enabled: network_enabled.unwrap_or(false),
        workspace: workspace.unwrap_or_default(),
        gpu_limit_percent: gpu_limit,
        outputs: outputs.unwrap_or_default(),
    };

    tasks.create_and_run(
        args,
        source_machine,
        source_user,
        target_machine,
        timeout_secs.unwrap_or(3600),
        options,
    )
}

// ── Concurrency cap ───────────────────────────────────────

/// Read the max number of incoming tasks that may run at once on this peer.
/// Tasks beyond this stay Queued until a slot frees.
#[tauri::command]
pub fn get_max_concurrent_tasks(tasks: State<'_, IncomingTasks>) -> usize {
    tasks.max_concurrent()
}

/// Update the concurrency cap. Lowering it doesn't kill in-flight tasks;
/// raising it pulls from the pending queue immediately.
#[tauri::command]
pub fn set_max_concurrent_tasks(tasks: State<'_, IncomingTasks>, n: usize) {
    tasks.set_max_concurrent(n);
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

// ── Managed venv ───────────────────────────────────────────────────────────

#[derive(serde::Serialize)]
pub struct ManagedVenvStatus {
    pub installed: bool,
    pub path: String,
}

#[tauri::command]
pub fn get_managed_venv_status() -> ManagedVenvStatus {
    ManagedVenvStatus {
        installed: UserManager::managed_venv_exists(),
        path: UserManager::managed_venv_path().to_string(),
    }
}

/// Provision the managed venv (creates `/var/lib/partagpu/venv` and installs
/// the ML toolkit via pkexec → helper setup-venv). Async because the install
/// takes 5-10 minutes (~3 GB download). Streams the helper's stdout/stderr
/// to the frontend as Tauri events `helper-output` / `helper-output-err`
/// so the UI can show a live install log.
#[tauri::command]
pub async fn setup_managed_venv(app: tauri::AppHandle) -> Result<(), String> {
    tokio::task::spawn_blocking(move || UserManager::setup_managed_venv(Some(&app)))
        .await
        .map_err(|e| format!("setup-venv interrompu : {e}"))?
}

#[tauri::command]
pub async fn remove_managed_venv() -> Result<(), String> {
    tokio::task::spawn_blocking(UserManager::remove_managed_venv)
        .await
        .map_err(|e| format!("remove-venv interrompu : {e}"))?
}

// ── Dispatch (UI) ──────────────────────────────────────────────────────────

/// Dispatcher une tâche sur un pair depuis l'UI. Async : libère le thread IPC
/// principal pendant que le dispatch bloque côté ureq, ce qui permet à l'UI
/// de continuer à poller `getOutgoingTasks` (live output) en parallèle.
/// Si `local_id` est fourni, c'est cet id qui est utilisé pour l'OutgoingTask.
/// `workspace` : fichiers à pousser dans `/workspace` du sandbox du pair
/// avant l'exécution (déjà encodés en base64 côté caller).
#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri commands need every State + every payload field
pub async fn dispatch_task(
    auth: State<'_, AuthManager>,
    discovery: State<'_, Discovery>,
    outgoing: State<'_, crate::task_runner::OutgoingTasks>,
    peer_ip: String,
    args: Vec<String>,
    timeout_secs: Option<u64>,
    network: Option<bool>,
    user: Option<String>,
    local_id: Option<String>,
    workspace: Option<Vec<crate::sandbox::WorkspaceFile>>,
) -> Result<Task, String> {
    // Clone the state out of the lifetime-bound State references so we can
    // move into a blocking worker task. The inner types are all Clone (Arc).
    let auth = auth.inner().clone();
    let discovery = discovery.inner().clone();
    let outgoing = outgoing.inner().clone();
    let timeout = timeout_secs.unwrap_or(3600).min(24 * 3600);
    let network = network.unwrap_or(false);
    let workspace = workspace.unwrap_or_default();

    tokio::task::spawn_blocking(move || {
        crate::http_api::dispatch_task_blocking(
            &auth,
            &discovery,
            &outgoing,
            &peer_ip,
            args,
            user,
            timeout,
            network,
            workspace,
            local_id,
            Vec::new(),
        )
    })
    .await
    .map_err(|e| format!("dispatch interrompu : {e}"))?
}

// ── Cancel ─────────────────────────────────────────────────────────────────

/// Annule une tâche entrante (que la machine exécute pour un pair).
#[tauri::command]
pub fn cancel_incoming_task(
    tasks: State<'_, IncomingTasks>,
    task_id: String,
) -> Result<(), String> {
    tasks.inner().cancel(&task_id)
}

/// Annule une tâche sortante (que la machine a soumise à un pair).
/// Propage l'annulation au pair via `DELETE /peer/v1/tasks/<id>`.
/// Async pour ne pas bloquer le thread IPC pendant la requête HTTP au pair.
#[tauri::command]
pub async fn cancel_outgoing_task(
    auth: State<'_, AuthManager>,
    outgoing: State<'_, crate::task_runner::OutgoingTasks>,
    local_id: String,
) -> Result<bool, String> {
    let auth = auth.inner().clone();
    let outgoing = outgoing.inner().clone();
    tokio::task::spawn_blocking(move || {
        crate::http_api::cancel_outgoing_task(&auth, &outgoing, &local_id)
    })
    .await
    .map_err(|e| format!("cancel interrompu : {e}"))?
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
