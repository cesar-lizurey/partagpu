pub mod api;
pub mod auth;
pub mod crypto;
pub mod discovery;
pub mod http_api;
pub mod peer_api;
pub mod resource;
pub mod sandbox;
pub mod security_log;
pub mod sharing;
pub mod task_runner;
pub mod user_manager;

use auth::AuthManager;
use crypto::EphemeralKey;
use discovery::Discovery;
use resource::ResourceMonitor;
use sandbox::Sandbox;
use security_log::SecurityLog;
use sharing::SharingController;
use std::sync::{Arc, Mutex};
use task_runner::{IncomingTasks, OutgoingTasks};

/// Rotate the peer-API ephemeral keypair on a fixed cadence so the
/// forward-secrecy window stays bounded even within a long-lived session.
/// On each tick we generate a new keypair, push the new pubkey into mDNS
/// (so peers update their cache), and let the previous key live ~60 s for
/// in-flight requests (handled by `EphemeralKey::dh_candidates`).
fn spawn_eph_rotation(server_eph: crypto::EphemeralKey, discovery: discovery::Discovery) {
    /// Re-keying period. 600 s = 10 min : an attacker who steals RAM at
    /// time T can decrypt traffic from at most T-600 s. Trade-off vs the
    /// mDNS noise of re-announcing every 10 min — fine on a LAN.
    const ROTATION_INTERVAL: std::time::Duration = std::time::Duration::from_secs(600);
    std::thread::spawn(move || loop {
        std::thread::sleep(ROTATION_INTERVAL);
        let new_pub = server_eph.rotate();
        discovery.set_ephemeral_pubkey(new_pub);
        // Garbage collection of the previous key is independent of rotation
        // so callers always see a single canonical state. Run it on each
        // tick anyway as a safety net.
        server_eph.gc_expired();
    });
}

/// Load a persistent machine ID from ~/.config/partagpu/machine-id,
/// or generate and save one on first launch. This avoids mDNS ghost
/// services when the app is restarted.
fn load_or_create_machine_id() -> String {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let path = config_dir.join("partagpu").join("machine-id");

    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if !id.is_empty() {
            return id;
        }
    }

    let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &id);
    id
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let hostname = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());

    let machine_id = load_or_create_machine_id();

    let sec_log = SecurityLog::new();
    let auth = AuthManager::new();
    let sharing = SharingController::new();

    // Forward-secret ephemeral keypair for the peer-API. Lives in memory
    // only — never written to disk. Regenerated on every app restart, plus
    // rotated every 10 minutes by `spawn_eph_rotation` below to keep the
    // forward-secrecy window short even within a single session.
    let server_eph = EphemeralKey::generate();

    let mut discovery = Discovery::new(&hostname, &machine_id)
        .expect("Failed to initialize mDNS discovery");
    discovery.set_auth(auth.clone());
    discovery.set_sharing(sharing.clone());
    discovery.set_security_log(sec_log.clone());
    discovery.set_ephemeral_pubkey(server_eph.public_b64());

    spawn_eph_rotation(server_eph.clone(), discovery.clone());

    if let Err(e) = discovery.register() {
        eprintln!("Warning: could not register mDNS service: {e}");
    }
    if let Err(e) = discovery.start_browsing() {
        eprintln!("Warning: could not start mDNS browsing: {e}");
    }
    discovery.start_mdns_refresh();
    let sandbox = Sandbox::new();
    let monitor = Arc::new(Mutex::new(ResourceMonitor::new()));
    let incoming = IncomingTasks::new(sandbox);
    let outgoing = OutgoingTasks::new();

    // Local HTTP API on 127.0.0.1:7654 — used by the Python package to
    // discover peers/GPUs and dispatch tasks to a remote peer.
    http_api::start(
        discovery.clone(),
        sharing.clone(),
        monitor.clone(),
        auth.clone(),
        outgoing.clone(),
    );

    // Peer-to-peer HTTP API on 0.0.0.0:7655 — receives task submissions from
    // verified peers (auth via shared TOTP secret) and runs them in the sandbox.
    peer_api::start(
        incoming.clone(),
        auth.clone(),
        discovery.clone(),
        sharing.clone(),
        sec_log.clone(),
        server_eph.clone(),
    );

    let incoming_for_setup = incoming.clone();
    let outgoing_for_setup = outgoing.clone();

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .setup(move |app| {
            // Hand the AppHandle to the task stores so they can push live
            // "{incoming,outgoing}-tasks-changed" events whenever state
            // mutates — replacing UI polling with push updates.
            incoming_for_setup.set_emitter(app.handle().clone());
            outgoing_for_setup.set_emitter(app.handle().clone());
            Ok(())
        })
        .manage(sec_log)
        .manage(auth)
        .manage(discovery)
        .manage(monitor)
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
            api::cancel_incoming_task,
            api::cancel_outgoing_task,
            api::dispatch_task,
            api::get_managed_venv_status,
            api::setup_managed_venv,
            api::remove_managed_venv,
            api::get_max_concurrent_tasks,
            api::set_max_concurrent_tasks,
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
