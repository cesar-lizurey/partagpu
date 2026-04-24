//! Minimal HTTP API server on localhost:7654.
//!
//! Exposes the peer list and GPU availability so that the `partagpu`
//! Python package can discover available resources without touching mDNS.
//!
//! Routes:
//!   GET /api/peers   → list of discovered peers (JSON)
//!   GET /api/gpu     → list of available GPUs across verified peers (JSON)
//!   GET /api/status  → local sharing status (JSON)

use crate::discovery::Discovery;
use crate::resource::ResourceMonitor;
use crate::sharing::SharingController;
use std::sync::{Arc, Mutex};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;

const LISTEN_ADDR: &str = "127.0.0.1:7654";

/// A GPU resource advertised by a peer (or local).
#[derive(serde::Serialize)]
struct GpuInfo {
    host: String,
    ip: String,
    gpu_limit_percent: f32,
    verified: bool,
}

pub fn start(
    discovery: Discovery,
    sharing: SharingController,
    monitor: Arc<Mutex<ResourceMonitor>>,
) {
    tokio::spawn(async move {
        let listener = match TcpListener::bind(LISTEN_ADDR).await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("HTTP API: failed to bind {LISTEN_ADDR}: {e}");
                return;
            }
        };

        loop {
            let (mut stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => continue,
            };

            let discovery = &discovery;
            let sharing = &sharing;
            let monitor = &monitor;

            // Read the request (we only need the first line)
            let mut buf = [0u8; 1024];
            let n = match tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await {
                Ok(n) if n > 0 => n,
                _ => continue,
            };

            let request = String::from_utf8_lossy(&buf[..n]);
            let first_line = request.lines().next().unwrap_or("");
            let path = first_line.split_whitespace().nth(1).unwrap_or("");

            let (status, body) = match path {
                "/api/peers" => {
                    let peers = discovery.get_peers();
                    ("200 OK", serde_json::to_string_pretty(&peers).unwrap_or_default())
                }
                "/api/gpu" => {
                    let gpus = build_gpu_list(discovery, monitor);
                    ("200 OK", serde_json::to_string_pretty(&gpus).unwrap_or_default())
                }
                "/api/status" => {
                    let config = sharing.get_config();
                    ("200 OK", serde_json::to_string_pretty(&config).unwrap_or_default())
                }
                _ => {
                    ("404 Not Found", r#"{"error":"Not found"}"#.to_string())
                }
            };

            let response = format!(
                "HTTP/1.1 {status}\r\n\
                 Content-Type: application/json\r\n\
                 Access-Control-Allow-Origin: *\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {body}",
                body.len()
            );

            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
}

fn build_gpu_list(discovery: &Discovery, monitor: &Arc<Mutex<ResourceMonitor>>) -> Vec<GpuInfo> {
    let mut gpus = Vec::new();

    // Local GPU
    if let Ok(mut mon) = monitor.lock() {
        let snap = mon.snapshot();
        if snap.gpu_available {
            gpus.push(GpuInfo {
                host: "local".to_string(),
                ip: local_ip_address::local_ip()
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|_| "127.0.0.1".into()),
                gpu_limit_percent: 100.0,
                verified: true,
            });
        }
    }

    // Remote GPUs from verified peers that are sharing
    for peer in discovery.get_verified_peers() {
        if peer.sharing_enabled && peer.gpu_limit > 0.0 {
            gpus.push(GpuInfo {
                host: peer.display_name.clone(),
                ip: peer.ip.clone(),
                gpu_limit_percent: peer.gpu_limit,
                verified: peer.verified,
            });
        }
    }

    gpus
}
