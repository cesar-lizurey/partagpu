use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const SERVICE_TYPE: &str = "_partagpu._tcp.local.";
const SERVICE_PORT: u16 = 7654;

/// Maximum number of peers we track. Beyond this, new peers are ignored.
const MAX_PEERS: usize = 50;

/// Minimum interval between updates from the same peer (in seconds).
/// Updates arriving faster than this are silently dropped.
const RATE_LIMIT_SECS: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    pub id: String,
    pub display_name: String,
    pub hostname: String,
    pub ip: String,
    pub port: u16,
    pub sharing_enabled: bool,
    pub cpu_limit: f32,
    pub ram_limit: f32,
    pub gpu_limit: f32,
    /// TOTP code announced by this peer (for verification).
    pub totp_code: String,
    /// Whether this peer's TOTP code has been verified.
    pub verified: bool,
    /// True if another peer already claimed this hostname (possible spoof).
    #[serde(default)]
    pub hostname_conflict: bool,
}

/// Internal tracking data per peer (not serialized to frontend).
struct PeerMeta {
    last_update: Instant,
}

pub struct Discovery {
    daemon: ServiceDaemon,
    peers: Arc<Mutex<HashMap<String, Peer>>>,
    peer_meta: Arc<Mutex<HashMap<String, PeerMeta>>>,
    instance_name: String,
    hostname: String,
    display_name: Arc<Mutex<String>>,
    auth: Option<crate::auth::AuthManager>,
    sec_log: Option<crate::security_log::SecurityLog>,
    /// Cached last TOTP code to avoid re-registering when unchanged.
    last_totp: Arc<Mutex<String>>,
}

impl Discovery {
    pub fn new(hostname: &str, machine_id: &str) -> Result<Self, String> {
        let daemon =
            ServiceDaemon::new().map_err(|e| format!("Failed to create mDNS daemon: {e}"))?;
        let instance_name = format!("partagpu-{machine_id}");

        Ok(Self {
            daemon,
            peers: Arc::new(Mutex::new(HashMap::new())),
            peer_meta: Arc::new(Mutex::new(HashMap::new())),
            instance_name,
            hostname: hostname.to_string(),
            display_name: Arc::new(Mutex::new(hostname.to_string())),
            auth: None,
            sec_log: None,
            last_totp: Arc::new(Mutex::new(String::new())),
        })
    }

    /// Attach an AuthManager so peers can be verified via TOTP.
    pub fn set_auth(&mut self, auth: crate::auth::AuthManager) {
        self.auth = Some(auth);
    }

    /// Attach a SecurityLog for event logging.
    pub fn set_security_log(&mut self, log: crate::security_log::SecurityLog) {
        self.sec_log = Some(log);
    }

    pub fn get_display_name(&self) -> String {
        self.display_name.lock().unwrap().clone()
    }

    pub fn set_display_name(&self, name: &str) {
        *self.display_name.lock().unwrap() = name.to_string();
        let _ = self.register();
    }

    pub fn register(&self) -> Result<(), String> {
        let local_ip =
            local_ip_address::local_ip().map_err(|e| format!("Failed to get local IP: {e}"))?;

        let display_name = self.display_name.lock().unwrap().clone();
        let totp_code = self
            .auth
            .as_ref()
            .and_then(|a| a.current_code())
            .unwrap_or_default();

        let properties = [
            ("hostname", self.hostname.as_str()),
            ("display_name", &display_name),
            ("sharing", "false"),
            ("cpu_limit", "0"),
            ("ram_limit", "0"),
            ("gpu_limit", "0"),
            ("totp", &totp_code),
        ];

        let ip_str = local_ip.to_string();
        let service = ServiceInfo::new(
            SERVICE_TYPE,
            &self.instance_name,
            &format!("{}.local.", self.hostname),
            &ip_str,
            SERVICE_PORT,
            &properties[..],
        )
        .map_err(|e| format!("Failed to create service info: {e}"))?;

        self.daemon
            .register(service)
            .map_err(|e| format!("Failed to register service: {e}"))?;

        Ok(())
    }

    /// Periodically re-register the mDNS service to refresh the TOTP code.
    pub fn start_totp_refresh(&self) {
        let daemon = self.daemon.clone();
        let auth = self.auth.clone();
        let instance_name = self.instance_name.clone();
        let hostname = self.hostname.clone();
        let display_name = self.display_name.clone();
        let last_totp = self.last_totp.clone();

        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));

                let new_code = auth
                    .as_ref()
                    .and_then(|a| a.current_code())
                    .unwrap_or_default();

                // Only re-register if the TOTP code actually changed.
                {
                    let mut last = last_totp.lock().unwrap();
                    if *last == new_code {
                        continue;
                    }
                    *last = new_code.clone();
                }

                let ip = match local_ip_address::local_ip() {
                    Ok(ip) => ip.to_string(),
                    Err(_) => continue,
                };

                let dn = display_name.lock().unwrap().clone();
                let properties = [
                    ("hostname", hostname.as_str()),
                    ("display_name", dn.as_str()),
                    ("sharing", "false"),
                    ("cpu_limit", "0"),
                    ("ram_limit", "0"),
                    ("gpu_limit", "0"),
                    ("totp", new_code.as_str()),
                ];

                if let Ok(service) = ServiceInfo::new(
                    SERVICE_TYPE,
                    &instance_name,
                    &format!("{}.local.", hostname),
                    &ip,
                    SERVICE_PORT,
                    &properties[..],
                ) {
                    let _ = daemon.register(service);
                }
            }
        });
    }

    pub fn start_browsing(&self) -> Result<(), String> {
        let receiver = self
            .daemon
            .browse(SERVICE_TYPE)
            .map_err(|e| format!("Failed to browse: {e}"))?;

        let peers = self.peers.clone();
        let peer_meta = self.peer_meta.clone();
        let my_name = self.instance_name.clone();
        let auth = self.auth.clone();
        let sec_log = self.sec_log.clone();

        std::thread::spawn(move || {
            loop {
                match receiver.recv() {
                    Ok(event) => match event {
                        ServiceEvent::ServiceResolved(info) => {
                            let name = info.get_fullname().to_string();
                            if name.contains(&my_name) {
                                continue;
                            }

                            // ── Rate limiting ──────────────────────
                            {
                                let mut meta = peer_meta.lock().unwrap();
                                if let Some(m) = meta.get(&name) {
                                    if m.last_update.elapsed().as_secs() < RATE_LIMIT_SECS {
                                        continue; // too fast, drop
                                    }
                                }
                                meta.insert(
                                    name.clone(),
                                    PeerMeta {
                                        last_update: Instant::now(),
                                    },
                                );
                            }

                            // ── Max peers limit ────────────────────
                            {
                                let map = peers.lock().unwrap();
                                if map.len() >= MAX_PEERS && !map.contains_key(&name) {
                                    eprintln!(
                                        "SECURITY: max peers ({MAX_PEERS}) reached, ignoring new peer: {name}"
                                    );
                                    continue;
                                }
                            }

                            let ip = info
                                .get_addresses()
                                .iter()
                                .find(|a| matches!(a, IpAddr::V4(_)))
                                .map(|a| a.to_string())
                                .unwrap_or_default();

                            let props = info.get_properties();
                            let hostname = props
                                .get_property_val_str("hostname")
                                .unwrap_or("unknown")
                                .to_string();
                            let display_name = props
                                .get_property_val_str("display_name")
                                .unwrap_or("")
                                .to_string();
                            let sharing = props
                                .get_property_val_str("sharing")
                                .unwrap_or("false")
                                == "true";
                            let cpu_limit: f32 = props
                                .get_property_val_str("cpu_limit")
                                .unwrap_or("0")
                                .parse()
                                .unwrap_or(0.0);
                            let ram_limit: f32 = props
                                .get_property_val_str("ram_limit")
                                .unwrap_or("0")
                                .parse()
                                .unwrap_or(0.0);
                            let gpu_limit: f32 = props
                                .get_property_val_str("gpu_limit")
                                .unwrap_or("0")
                                .parse()
                                .unwrap_or(0.0);
                            let totp_code = props
                                .get_property_val_str("totp")
                                .unwrap_or("")
                                .to_string();

                            let verified = match &auth {
                                Some(a) if !totp_code.is_empty() => a.verify_code(&totp_code),
                                Some(_) => false,
                                None => true,
                            };

                            // ── Hostname conflict detection ────────
                            let hostname_conflict = {
                                let map = peers.lock().unwrap();
                                map.values().any(|p| {
                                    p.hostname == hostname && p.id != name && p.ip != ip
                                })
                            };

                            if hostname_conflict {
                                if let Some(ref log) = sec_log {
                                    log.peer_event(
                                        crate::security_log::EventCategory::HostnameConflict,
                                        &format!("Conflit : « {hostname} » annoncé par {ip} mais déjà connu depuis une autre IP"),
                                        &ip, &hostname,
                                    );
                                }
                            }

                            let peer = Peer {
                                id: name.clone(),
                                display_name,
                                hostname,
                                ip,
                                port: info.get_port(),
                                sharing_enabled: sharing,
                                cpu_limit,
                                ram_limit,
                                gpu_limit,
                                totp_code,
                                verified,
                                hostname_conflict,
                            };

                            if let Ok(mut map) = peers.lock() {
                                let is_new = !map.contains_key(&name);
                                map.insert(name, peer.clone());

                                if let Some(ref log) = sec_log {
                                    if is_new {
                                        let cat = if peer.verified {
                                            crate::security_log::EventCategory::PeerVerified
                                        } else {
                                            crate::security_log::EventCategory::PeerRejected
                                        };
                                        log.peer_event(
                                            cat,
                                            &format!(
                                                "Pair {} ({})",
                                                peer.hostname,
                                                if peer.verified { "vérifié" } else { "non vérifié" },
                                            ),
                                            &peer.ip,
                                            &peer.hostname,
                                        );
                                    }
                                }
                            }
                        }
                        ServiceEvent::ServiceRemoved(_, name) => {
                            if let Ok(mut map) = peers.lock() {
                                if let Some(removed) = map.remove(&name) {
                                    if let Some(ref log) = sec_log {
                                        log.peer_event(
                                            crate::security_log::EventCategory::PeerDisconnected,
                                            &format!("Pair déconnecté : {}", removed.hostname),
                                            &removed.ip,
                                            &removed.hostname,
                                        );
                                    }
                                }
                            }
                            if let Ok(mut meta) = peer_meta.lock() {
                                meta.remove(&name);
                            }
                        }
                        _ => {}
                    },
                    Err(_) => break,
                }
            }
        });

        Ok(())
    }

    /// Get all discovered peers.
    pub fn get_peers(&self) -> Vec<Peer> {
        self.peers
            .lock()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Get only verified peers (for use when a room is active).
    pub fn get_verified_peers(&self) -> Vec<Peer> {
        self.peers
            .lock()
            .map(|map| {
                map.values()
                    .filter(|p| p.verified && !p.hostname_conflict)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn shutdown(&self) {
        let _ = self.daemon.shutdown();
    }
}
