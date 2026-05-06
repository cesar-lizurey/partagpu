//! Local HTTP API on 127.0.0.1:7654.
//!
//! Used by the `partagpu` Python package and any local client that wants
//! to introspect peers, GPUs, or dispatch a task to a peer.
//!
//! Routes:
//!   GET  /api/peers     → list of discovered peers (JSON)
//!   GET  /api/gpu       → list of available GPUs across verified peers (JSON)
//!   GET  /api/status    → local sharing status (JSON)
//!   POST /api/dispatch  → run a task on a remote peer, blocks until completion
//!
//! /api/dispatch body: { "peer_ip": "192.168.x.y", "args": [...], "timeout_secs": 60, "user": "alice" }

use crate::auth::AuthManager;
use crate::discovery::Discovery;
use crate::resource::ResourceMonitor;
use crate::sandbox::WorkspaceFile;
use crate::sharing::SharingController;
use crate::task_runner::{new_task, OutgoingTasks, Task, TaskStatus};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const LISTEN_ADDR: &str = "127.0.0.1:7654";
const PEER_PORT: u16 = 7655;
const MAX_REQUEST_BYTES: usize = 256 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(500);

/// A GPU resource advertised by a peer (or local). One entry per physical
/// CUDA device — a peer with N GPUs produces N entries (same host, different
/// device_index).
#[derive(Serialize)]
struct GpuInfo {
    host: String,
    ip: String,
    device_index: u32,
    gpu_limit_percent: f32,
    verified: bool,
}

#[derive(Deserialize)]
struct DispatchBody {
    peer_ip: String,
    args: Vec<String>,
    #[serde(default)]
    timeout_secs: Option<u64>,
    #[serde(default)]
    user: Option<String>,
    /// Drop sandbox network isolation on the peer (DDP rendezvous, etc.).
    #[serde(default)]
    network: Option<bool>,
    /// Files to push to the peer's sandbox workspace before exec.
    #[serde(default)]
    workspace: Option<Vec<WorkspaceFile>>,
    /// Optional client-supplied local task id. Lets the client know the id
    /// before the dispatch returns, so it can cancel mid-flight (e.g. on
    /// KeyboardInterrupt). If absent, the app generates a UUID.
    #[serde(default)]
    local_id: Option<String>,
}

#[derive(Deserialize)]
struct CancelBody {
    /// Local outgoing-task id returned by /api/dispatch (in the `id` field of
    /// the returned Task). Accepts either spelling for ergonomics.
    #[serde(default, alias = "task_id")]
    local_id: Option<String>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

#[derive(Clone)]
struct ApiState {
    discovery: Discovery,
    sharing: SharingController,
    monitor: Arc<Mutex<ResourceMonitor>>,
    auth: AuthManager,
    outgoing: OutgoingTasks,
}

pub fn start(
    discovery: Discovery,
    sharing: SharingController,
    monitor: Arc<Mutex<ResourceMonitor>>,
    auth: AuthManager,
    outgoing: OutgoingTasks,
) {
    let state = ApiState {
        discovery,
        sharing,
        monitor,
        auth,
        outgoing,
    };

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("HTTP API: failed to build tokio runtime: {e}");
                return;
            }
        };

        runtime.block_on(async move {
            let listener = match TcpListener::bind(LISTEN_ADDR).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("HTTP API: failed to bind {LISTEN_ADDR}: {e}");
                    return;
                }
            };

            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };
                let state = state.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, state).await {
                        eprintln!("HTTP API: connection error: {e}");
                    }
                });
            }
        });
    });
}

async fn handle_connection(mut stream: TcpStream, state: ApiState) -> Result<(), String> {
    let req = read_request(&mut stream).await?;

    let (status, body) = match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/api/peers") => {
            let peers = state.discovery.get_peers();
            (
                "200 OK",
                serde_json::to_string_pretty(&peers).unwrap_or_default(),
            )
        }
        ("GET", "/api/gpu") => {
            let gpus = build_gpu_list(&state.discovery, &state.monitor);
            (
                "200 OK",
                serde_json::to_string_pretty(&gpus).unwrap_or_default(),
            )
        }
        ("GET", "/api/status") => {
            let config = state.sharing.get_config();
            (
                "200 OK",
                serde_json::to_string_pretty(&config).unwrap_or_default(),
            )
        }
        ("POST", "/api/dispatch") => handle_dispatch(&req, &state).await,
        ("POST", "/api/cancel") => handle_cancel(&req, &state).await,
        ("OPTIONS", _) => ("204 No Content", String::new()),
        _ => (
            "404 Not Found",
            r#"{"error":"Not found"}"#.to_string(),
        ),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
         Access-Control-Allow-Origin: *\r\n\
         Access-Control-Allow-Methods: GET, POST, OPTIONS\r\n\
         Access-Control-Allow-Headers: Content-Type\r\n\
         Content-Length: {}\r\n\
         Connection: close\r\n\
         \r\n\
         {body}",
        body.len()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| format!("write: {e}"))?;
    let _ = stream.shutdown().await;
    Ok(())
}

// ── Dispatch handler ───────────────────────────────────────────────────────

async fn handle_dispatch(req: &Request, state: &ApiState) -> (&'static str, String) {
    let body: DispatchBody = match serde_json::from_str(&req.body) {
        Ok(b) => b,
        Err(e) => {
            return error_resp("400 Bad Request", &format!("Corps JSON invalide : {e}"));
        }
    };
    if body.peer_ip.trim().is_empty() {
        return error_resp("400 Bad Request", "Champ 'peer_ip' requis.");
    }
    if body.args.is_empty() {
        return error_resp("400 Bad Request", "Champ 'args' requis et non vide.");
    }

    let auth = state.auth.clone();
    let discovery = state.discovery.clone();
    let outgoing = state.outgoing.clone();

    let result = tokio::task::spawn_blocking(move || {
        dispatch_task_blocking(
            &auth,
            &discovery,
            &outgoing,
            &body.peer_ip.trim().to_string(),
            body.args,
            body.user,
            body.timeout_secs.unwrap_or(3600).min(24 * 3600),
            body.network.unwrap_or(false),
            body.workspace.unwrap_or_default(),
            body.local_id,
        )
    })
    .await;

    match result {
        Ok(Ok(task)) => (
            "200 OK",
            serde_json::to_string(&task).unwrap_or_default(),
        ),
        Ok(Err(e)) => {
            // Map the most common error prefixes to dedicated status codes
            // so the Python client can distinguish "no room" from "peer
            // unreachable" without parsing the message.
            let status = if e.contains("salle PartaGPU") {
                "412 Precondition Failed"
            } else {
                "502 Bad Gateway"
            };
            error_resp(status, &e)
        }
        Err(e) => error_resp("500 Internal Server Error", &format!("interrompu : {e}")),
    }
}

/// Dispatch a task to a peer and block until it reaches a terminal state.
/// Reusable from any blocking context (HTTP handler via spawn_blocking, or a
/// Tauri command on the worker thread). Mutates the OutgoingTasks map as the
/// task progresses.
pub fn dispatch_task_blocking(
    auth: &AuthManager,
    discovery: &Discovery,
    outgoing: &OutgoingTasks,
    peer_ip: &str,
    args: Vec<String>,
    user: Option<String>,
    timeout_secs: u64,
    network: bool,
    workspace: Vec<WorkspaceFile>,
    local_id_override: Option<String>,
) -> Result<Task, String> {
    if peer_ip.is_empty() {
        return Err("peer_ip vide.".into());
    }
    if args.is_empty() {
        return Err("args vide.".into());
    }

    let totp = auth
        .current_code()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| {
            "Cette machine n'est dans aucune salle PartaGPU. Joignez une salle pour pouvoir dispatcher des tâches."
                .to_string()
        })?;

    let user = user.unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "local".into()));
    let local_hostname = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "local".into());

    // Display name for the peer (best effort). Loopback case (target =
    // ourselves) is handled specially because we exclude our own announcement
    // from `get_peers()`, so the lookup would fall back to the raw IP.
    let local_lan_ip = local_ip_address::local_ip().map(|ip| ip.to_string()).ok();
    let is_loopback_target = peer_ip == "127.0.0.1"
        || peer_ip == "0.0.0.0"
        || local_lan_ip.as_deref() == Some(peer_ip);

    let target_machine = if is_loopback_target {
        let dn = discovery.get_display_name();
        if !dn.is_empty() {
            format!("{dn} (local)")
        } else {
            "local".to_string()
        }
    } else {
        discovery
            .get_peers()
            .into_iter()
            .find(|p| p.ip == peer_ip)
            .map(|p| {
                if !p.display_name.is_empty() {
                    p.display_name
                } else {
                    p.hostname
                }
            })
            .unwrap_or_else(|| peer_ip.to_string())
    };

    // Reserve the OutgoingTask entry immediately so the UI shows activity.
    let mut local_task = new_task(
        args.clone(),
        local_hostname,
        user.clone(),
        target_machine,
    );
    if let Some(client_id) = local_id_override.as_ref().filter(|s| !s.is_empty()) {
        local_task.id = client_id.clone();
    }
    local_task.network_enabled = network;
    outgoing.add(local_task.clone());
    let local_id = local_task.id.clone();

    let result = run_remote_blocking(
        peer_ip,
        &args,
        &user,
        timeout_secs,
        &totp,
        network,
        workspace,
        outgoing.clone(),
        &local_id,
    );

    match result {
        Ok(task) => {
            local_task.status = task.status;
            if task.status != TaskStatus::Cancelled {
                local_task.progress = 100.0;
            }
            local_task.output = task.output.clone();
            local_task.error_output = task.error_output.clone();
            local_task.exit_code = task.exit_code;
            outgoing.replace(local_task.clone());
            outgoing.clear_remote_ref(&local_id);
            Ok(local_task)
        }
        Err(e) => {
            outgoing.set_failed(&local_id, &e);
            outgoing.clear_remote_ref(&local_id);
            Err(e)
        }
    }
}

// ── Cancel handler ─────────────────────────────────────────────────────────

/// Cancel an outgoing task by its local id. Blocks (calls ureq).
/// Result.0 = "remote cancellation acknowledged by peer" (true) or "only
/// local state updated, peer was unreachable" (false).
pub fn cancel_outgoing_task(
    auth: &AuthManager,
    outgoing: &OutgoingTasks,
    local_id: &str,
) -> Result<bool, String> {
    let remote = match outgoing.get_remote_ref(local_id) {
        Some(r) => r,
        None => {
            // Either never reached the peer, or already finished. Mark
            // locally regardless.
            outgoing.set_cancelled(local_id);
            return Ok(false);
        }
    };
    let totp = auth
        .current_code()
        .filter(|c| !c.is_empty())
        .ok_or_else(|| "Cette machine n'est dans aucune salle PartaGPU.".to_string())?;

    let url = format!(
        "http://{}:{PEER_PORT}/peer/v1/tasks/{}",
        remote.peer_ip, remote.remote_task_id
    );
    let resp = ureq::delete(&url)
        .set("X-PartaGPU-TOTP", &totp)
        .timeout(Duration::from_secs(10))
        .call();

    let acknowledged = match resp {
        Ok(r) if r.status() >= 200 && r.status() < 300 => true,
        Ok(r) => return Err(format!("le pair a répondu HTTP {}", r.status())),
        Err(ureq::Error::Status(s, _)) => {
            return Err(format!("le pair a répondu HTTP {s}"))
        }
        Err(e) => return Err(format!("erreur de connexion au pair : {e}")),
    };

    outgoing.set_cancelled(local_id);
    outgoing.clear_remote_ref(local_id);
    Ok(acknowledged)
}

async fn handle_cancel(req: &Request, state: &ApiState) -> (&'static str, String) {
    let body: CancelBody = match serde_json::from_str(&req.body) {
        Ok(b) => b,
        Err(e) => {
            return error_resp(
                "400 Bad Request",
                &format!("Corps JSON invalide : {e}"),
            );
        }
    };
    let local_id = match body.local_id {
        Some(s) if !s.is_empty() => s,
        _ => return error_resp("400 Bad Request", "Champ 'local_id' (ou 'task_id') requis."),
    };

    let auth = state.auth.clone();
    let outgoing = state.outgoing.clone();
    let local_id_for_worker = local_id.clone();
    let result =
        tokio::task::spawn_blocking(move || cancel_outgoing_task(&auth, &outgoing, &local_id_for_worker))
            .await;

    match result {
        Ok(Ok(remote)) => (
            "200 OK",
            json_string(&serde_json::json!({"cancelled": true, "remote": remote})),
        ),
        Ok(Err(e)) => {
            // Couldn't reach the peer, but mark locally so the user sees the
            // intent reflected.
            state.outgoing.set_cancelled(&local_id);
            (
                "502 Bad Gateway",
                json_string(&serde_json::json!({"cancelled": true, "remote": false, "error": e})),
            )
        }
        Err(e) => error_resp("500 Internal Server Error", &format!("interrompu : {e}")),
    }
}

/// Submit + poll a task on a remote peer. Blocking. Updates the matching
/// OutgoingTask entry as it progresses.
fn run_remote_blocking(
    peer_ip: &str,
    args: &[String],
    user: &str,
    timeout_secs: u64,
    totp: &str,
    network_enabled: bool,
    workspace: Vec<WorkspaceFile>,
    outgoing: OutgoingTasks,
    local_id: &str,
) -> Result<Task, String> {
    let url_submit = format!("http://{peer_ip}:{PEER_PORT}/peer/v1/tasks");
    let body = serde_json::json!({
        "args": args,
        "source_user": user,
        "timeout_secs": timeout_secs,
        "network_enabled": network_enabled,
        "workspace": workspace,
    });

    let resp = ureq::post(&url_submit)
        .set("X-PartaGPU-TOTP", totp)
        .set("Content-Type", "application/json")
        .timeout(Duration::from_secs(15))
        .send_json(body)
        .map_err(|e| format!("connexion au pair {peer_ip} échouée : {e}"))?;

    if resp.status() < 200 || resp.status() >= 300 {
        let status = resp.status();
        let text = resp.into_string().unwrap_or_default();
        return Err(format!(
            "Le pair {peer_ip} a refusé la tâche (HTTP {status}) : {text}"
        ));
    }

    #[derive(Deserialize)]
    struct SubmitResp {
        task_id: String,
    }
    let submit: SubmitResp = resp
        .into_json()
        .map_err(|e| format!("réponse du pair invalide : {e}"))?;

    // Remember which peer task corresponds to this local id, so a future
    // cancel can be propagated.
    outgoing.set_remote_ref(local_id, peer_ip, &submit.task_id);
    outgoing.update_progress(local_id, 5.0, TaskStatus::Running);

    // Poll until terminal state, with a wall-clock budget = task timeout + grace.
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.saturating_add(30));
    let url_get = format!("http://{peer_ip}:{PEER_PORT}/peer/v1/tasks/{}", submit.task_id);

    loop {
        if Instant::now() > deadline {
            return Err("dépassement du délai d'attente côté local".into());
        }
        std::thread::sleep(POLL_INTERVAL);

        let r = match ureq::get(&url_get)
            .set("X-PartaGPU-TOTP", totp)
            .timeout(Duration::from_secs(10))
            .call()
        {
            Ok(r) => r,
            Err(ureq::Error::Status(404, _)) => {
                return Err("le pair a perdu la tâche (404)".into());
            }
            Err(e) => {
                // Transient network error — retry next poll.
                eprintln!("dispatch poll: {e}");
                continue;
            }
        };

        let task: Task = match r.into_json() {
            Ok(t) => t,
            Err(e) => {
                eprintln!("dispatch poll decode: {e}");
                continue;
            }
        };

        // Mirror everything live from the peer (output, progress, CPU/RAM/GPU)
        // into the local OutgoingTask so the UI shows real-time state — not
        // just at terminal but during the whole run.
        outgoing.mirror_running(local_id, &task);

        match task.status {
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled => {
                return Ok(task);
            }
            TaskStatus::Running | TaskStatus::Queued => {
                // Make sure status is reflected (mirror_running doesn't touch
                // status). Use update_progress to flip Running on first poll.
                let mut map_status = task.status;
                if map_status == TaskStatus::Queued {
                    map_status = TaskStatus::Running;
                }
                outgoing.update_progress(local_id, task.progress, map_status);
            }
        }
    }
}

fn build_gpu_list(discovery: &Discovery, _monitor: &Arc<Mutex<ResourceMonitor>>) -> Vec<GpuInfo> {
    let mut gpus = Vec::new();

    // Local: one entry per visible CUDA device (avoids snapshot lock contention).
    let local_ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "127.0.0.1".into());
    for d in crate::resource::list_gpus() {
        gpus.push(GpuInfo {
            host: "local".to_string(),
            ip: local_ip.clone(),
            device_index: d.index,
            gpu_limit_percent: 100.0,
            verified: true,
        });
    }

    // Remote peers: expand peer.gpu_count into N entries with synthetic
    // device indices 0..gpu_count. The peer announces its actual GPU count
    // via mDNS; we don't know each device's name from here, only the count.
    for peer in discovery.get_verified_peers() {
        if !peer.sharing_enabled || peer.gpu_limit <= 0.0 {
            continue;
        }
        let count = peer.gpu_count.max(1); // backwards compat: if peer
        // doesn't announce gpu_count (older app), assume 1 GPU.
        for idx in 0..count {
            gpus.push(GpuInfo {
                host: peer.display_name.clone(),
                ip: peer.ip.clone(),
                device_index: idx,
                gpu_limit_percent: peer.gpu_limit,
                verified: peer.verified,
            });
        }
    }

    gpus
}

fn error_resp(status: &'static str, msg: &str) -> (&'static str, String) {
    (
        status,
        json_string(&ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

fn json_string<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

// ── HTTP request reader ────────────────────────────────────────────────────

struct Request {
    method: String,
    path: String,
    body: String,
    #[allow(dead_code)]
    headers: Vec<(String, String)>,
}

async fn read_request(stream: &mut TcpStream) -> Result<Request, String> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let header_end;

    loop {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return Err("connexion fermée".into());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_REQUEST_BYTES {
            return Err("requête trop volumineuse".into());
        }
        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            header_end = p;
            break;
        }
    }

    let header_part = std::str::from_utf8(&buf[..header_end])
        .map_err(|_| "en-têtes non UTF-8".to_string())?;
    let mut lines = header_part.split("\r\n");
    let first = lines.next().ok_or("requête vide".to_string())?;
    let mut parts = first.split_whitespace();
    let method = parts.next().ok_or("méthode manquante")?.to_string();
    let path = parts.next().ok_or("chemin manquant")?.to_string();

    let mut headers = Vec::new();
    let mut content_length: usize = 0;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            let key = k.trim().to_string();
            let val = v.trim().to_string();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = val.parse().unwrap_or(0);
            }
            headers.push((key, val));
        }
    }

    let mut body_bytes = buf[header_end + 4..].to_vec();
    while body_bytes.len() < content_length {
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            break;
        }
        body_bytes.extend_from_slice(&tmp[..n]);
        if body_bytes.len() > MAX_REQUEST_BYTES {
            return Err("corps trop volumineux".into());
        }
    }
    body_bytes.truncate(content_length);
    let body = String::from_utf8(body_bytes).map_err(|_| "corps non UTF-8".to_string())?;

    Ok(Request {
        method,
        path,
        body,
        headers,
    })
}
