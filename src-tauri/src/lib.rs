pub mod api;
pub mod auth;
pub mod discovery;
pub mod resource;
pub mod sandbox;
pub mod security_log;
pub mod sharing;
pub mod task_runner;
pub mod user_manager;

use auth::AuthManager;
use discovery::Discovery;
use resource::ResourceMonitor;
use sandbox::Sandbox;
use security_log::SecurityLog;
use sharing::SharingController;
use std::sync::Mutex;
use task_runner::{IncomingTasks, OutgoingTasks};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hostname = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let machine_id = uuid::Uuid::new_v4().to_string()[..8].to_string();

    let sec_log = SecurityLog::new();
    let auth = AuthManager::new();

    let mut discovery = Discovery::new(&hostname, &machine_id)
        .expect("Failed to initialize mDNS discovery");
    discovery.set_auth(auth.clone());
    discovery.set_security_log(sec_log.clone());

    if let Err(e) = discovery.register() {
        eprintln!("Warning: could not register mDNS service: {e}");
    }
    if let Err(e) = discovery.start_browsing() {
        eprintln!("Warning: could not start mDNS browsing: {e}");
    }
    let sandbox = Sandbox::new();
    let monitor = ResourceMonitor::new();
    let sharing = SharingController::new();
    let incoming = IncomingTasks::new(sandbox);
    let outgoing = OutgoingTasks::new();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(sec_log)
        .manage(auth)
        .manage(discovery)
        .manage(Mutex::new(monitor))
        .manage(sharing)
        .manage(incoming)
        .manage(outgoing)
        .invoke_handler(tauri::generate_handler![
            api::create_room,
            api::join_room,
            api::leave_room,
            api::get_room_status,
            api::get_room_secret,
            api::verify_peer_code,
            api::get_peers,
            api::get_display_name,
            api::set_display_name,
            api::get_user_status,
            api::set_user_password,
            api::get_resources,
            api::get_sharing_config,
            api::enable_sharing,
            api::disable_sharing,
            api::pause_sharing,
            api::resume_sharing,
            api::set_sharing_limits,
            api::get_incoming_tasks,
            api::get_outgoing_tasks,
            api::submit_task,
            api::get_allowlist,
            api::add_to_allowlist,
            api::remove_from_allowlist,
            api::check_sandbox_available,
            api::get_security_log,
            api::clear_security_log,
            api::get_machine_info,
        ])
        .run(tauri::generate_context!())
        .expect("Error while running PartaGPU");
}
