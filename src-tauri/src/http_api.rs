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
use crate::crypto::{self, ENCRYPTED_CONTENT_TYPE};
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
/// Cap on local API request size. Same generous size as peer_api so the
/// /api/dispatch endpoint can accept a 16 MB workspace from Python clients.
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
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

    // ── Anti-CSRF / DNS-rebinding gate ─────────────────────────────────────
    // Reject any request that doesn't look like a local CLI/SDK client.
    // - `Host` MUST be 127.0.0.1:7654 or localhost:7654. DNS rebinding
    //   (evil.com → 127.0.0.1) leaves the original Host in the request even
    //   though the TCP connection lands here, so this catches the rebind.
    // - `Origin` MUST be absent. Browsers always set Origin on cross-origin
    //   fetch ; legitimate callers (`requests`, `curl`, Tauri IPC bypass) do
    //   not. A browser-driven dispatch attempt from any tab thus fails.
    if let Some((status, msg)) = check_local_origin(&req) {
        let body = format!(r#"{{"error":{}}}"#, json_string_escape(msg));
        return write_short_response(&mut stream, status, &body).await;
    }

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
        ("GET", path) if path.starts_with("/api/tasks/") && path.contains("/output") => {
            handle_task_output(path, &state)
        }
        ("OPTIONS", _) => ("204 No Content", String::new()),
        _ => ("404 Not Found", r#"{"error":"Not found"}"#.to_string()),
    };

    // No CORS headers: this API is local-only and explicitly NOT meant to
    // be reached by browsers. The check_local_origin gate above blocks any
    // cross-origin call before it reaches a handler.
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
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
            body.peer_ip.trim(),
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
        Ok(Ok(task)) => ("200 OK", serde_json::to_string(&task).unwrap_or_default()),
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
#[allow(clippy::too_many_arguments)] // dispatch is intrinsically wide ; refactor to a struct would just rename the args
pub fn dispatch_task_blocking(
    auth: &AuthManager,
    discovery: &Discovery,
    outgoing: &OutgoingTasks,
    peer_ip: &str,
    args: Vec<String>,
    user: Option<String>,
    timeout_secs: u64,
    network: bool,
    mut workspace: Vec<WorkspaceFile>,
    local_id_override: Option<String>,
) -> Result<Task, String> {
    // Compress the workspace before encryption (gzip is much faster on the
    // pre-encrypted plaintext than after — and ciphertext is incompressible).
    // Idempotent for clients that already pre-compressed.
    if !workspace.is_empty() {
        crate::sandbox::compress_workspace(&mut workspace)?;
    }
    if peer_ip.is_empty() {
        return Err("peer_ip vide.".into());
    }
    if args.is_empty() {
        return Err("args vide.".into());
    }

    if !auth.is_joined() {
        return Err(
            "Cette machine n'est dans aucune salle PartaGPU. Joignez une salle pour pouvoir dispatcher des tâches."
                .to_string(),
        );
    }
    let secret_b32 = auth.get_secret().ok_or_else(|| {
        "Impossible de dériver la clé de chiffrement (secret de salle indisponible).".to_string()
    })?;
    let key = crypto::derive_room_key(&secret_b32).map_err(|e| e.to_string())?;

    let user = user.unwrap_or_else(|| std::env::var("USER").unwrap_or_else(|_| "local".into()));
    let local_hostname = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "local".into());

    // Display name for the peer (best effort). Loopback case (target =
    // ourselves) is handled specially because we exclude our own announcement
    // from `get_peers()`, so the lookup would fall back to the raw IP.
    let local_lan_ip = local_ip_address::local_ip().map(|ip| ip.to_string()).ok();
    let is_loopback_target =
        peer_ip == "127.0.0.1" || peer_ip == "0.0.0.0" || local_lan_ip.as_deref() == Some(peer_ip);

    // Look up the peer's ephemeral X25519 pubkey from mDNS. Empty string
    // means the peer is on an older PartaGPU that doesn't support v=2 yet —
    // we'll fall back to the v=1 envelope (no forward secrecy) in that case.
    let peer_eph_pk = if is_loopback_target {
        // Talking to ourselves : we don't appear in get_peers(), so just
        // skip ECDH (the room key alone is enough; same machine).
        String::new()
    } else {
        discovery
            .get_peers()
            .into_iter()
            .find(|p| p.ip == peer_ip)
            .map(|p| p.eph_pk)
            .unwrap_or_default()
    };

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
    let mut local_task = new_task(args.clone(), local_hostname, user.clone(), target_machine);
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
        auth,
        &key,
        &peer_eph_pk,
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
    let path = format!("/peer/v1/tasks/{}", remote.remote_task_id);
    // DELETE has no body, so the HMAC is computed over the empty byte slice.
    let auth_header = auth
        .compute_request_auth("DELETE", &path, b"")
        .ok_or_else(|| "Cette machine n'est dans aucune salle PartaGPU.".to_string())?;

    let url = format!("http://{}:{PEER_PORT}{path}", remote.peer_ip);
    let resp = ureq::delete(&url)
        .set("X-PartaGPU-AUTH", &auth_header)
        .timeout(Duration::from_secs(10))
        .call();

    let acknowledged = match resp {
        Ok(r) if r.status() >= 200 && r.status() < 300 => true,
        Ok(r) => return Err(format!("le pair a répondu HTTP {}", r.status())),
        Err(ureq::Error::Status(s, _)) => return Err(format!("le pair a répondu HTTP {s}")),
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
            return error_resp("400 Bad Request", &format!("Corps JSON invalide : {e}"));
        }
    };
    let local_id = match body.local_id {
        Some(s) if !s.is_empty() => s,
        _ => return error_resp("400 Bad Request", "Champ 'local_id' (ou 'task_id') requis."),
    };

    let auth = state.auth.clone();
    let outgoing = state.outgoing.clone();
    let local_id_for_worker = local_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        cancel_outgoing_task(&auth, &outgoing, &local_id_for_worker)
    })
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

/// `GET /api/tasks/<local_id>/output?stdout_since=N&stderr_since=M`
///
/// Renvoie les chunks de stdout/stderr accumules depuis les offsets indiques.
/// Permet aux clients SDK (run_remote(live=True), distribute(live=True))
/// d'afficher la sortie de la tache en streaming pendant que /api/dispatch
/// bloque sur la connexion principale.
///
/// Reponse 200 :
/// ```json
/// {
///   "stdout_chunk": "...",
///   "stderr_chunk": "...",
///   "stdout_total": 1234,
///   "stderr_total": 56,
///   "status": "Running",
///   "exit_code": null
/// }
/// ```
fn handle_task_output(path: &str, state: &ApiState) -> (&'static str, String) {
    // path = "/api/tasks/<id>/output[?stdout_since=N&stderr_since=M]"
    let rest = match path.strip_prefix("/api/tasks/") {
        Some(s) => s,
        None => return error_resp("404 Not Found", "route invalide"),
    };
    let (id, query) = match rest.find("/output") {
        Some(idx) => {
            let id = &rest[..idx];
            let after = &rest[idx + "/output".len()..];
            let q = after.strip_prefix('?').unwrap_or("");
            (id, q)
        }
        None => return error_resp("404 Not Found", "route invalide"),
    };
    if id.is_empty() {
        return error_resp("400 Bad Request", "id manquant");
    }
    let mut stdout_since: usize = 0;
    let mut stderr_since: usize = 0;
    for kv in query.split('&').filter(|s| !s.is_empty()) {
        if let Some((k, v)) = kv.split_once('=') {
            match k {
                "stdout_since" => stdout_since = v.parse().unwrap_or(0),
                "stderr_since" => stderr_since = v.parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    let task = match state.outgoing.get(id) {
        Some(t) => t,
        None => return error_resp("404 Not Found", "tache inconnue"),
    };
    let stdout_total = task.output.len();
    let stderr_total = task.error_output.len();
    // Slice byte-wise mais respecte les bornes UTF-8 : `floor_char_boundary`
    // n'est pas stable, donc on utilise un slice naif et on tolere qu'un
    // chunk se termine au milieu d'un caractere multi-octet (le client
    // re-concatenera au prochain poll).
    let stdout_chunk = task.output.as_bytes().get(stdout_since..).unwrap_or(&[]);
    let stderr_chunk = task
        .error_output
        .as_bytes()
        .get(stderr_since..)
        .unwrap_or(&[]);
    let stdout_chunk = String::from_utf8_lossy(stdout_chunk).to_string();
    let stderr_chunk = String::from_utf8_lossy(stderr_chunk).to_string();
    let body = serde_json::json!({
        "stdout_chunk": stdout_chunk,
        "stderr_chunk": stderr_chunk,
        "stdout_total": stdout_total,
        "stderr_total": stderr_total,
        "status": task.status,
        "exit_code": task.exit_code,
    });
    ("200 OK", body.to_string())
}

/// Submit + poll a task on a remote peer. Blocking. Updates the matching
/// OutgoingTask entry as it progresses.
///
/// When `peer_eph_pk` is non-empty, every request uses a v=2 envelope (fresh
/// X25519 keypair on the client + ECDH against the peer's announced ephemeral
/// public key) for forward secrecy. Each request derives its own session key
/// which is then used to read back the peer's response. When the peer doesn't
/// advertise an `eph_pk` (older version), we fall back to the v=1 envelope
/// keyed by the room secret alone.
#[allow(clippy::too_many_arguments)]
fn run_remote_blocking(
    peer_ip: &str,
    args: &[String],
    user: &str,
    timeout_secs: u64,
    auth: &AuthManager,
    key: &[u8; 32],
    peer_eph_pk: &str,
    network_enabled: bool,
    workspace: Vec<WorkspaceFile>,
    outgoing: OutgoingTasks,
    local_id: &str,
) -> Result<Task, String> {
    /// Compute the auth header for an outgoing request, bound to the
    /// (method, path, body) triple. Bubbles up "no room" as an error.
    fn auth_header(
        auth: &AuthManager,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<String, String> {
        auth.compute_request_auth(method, path, body)
            .ok_or_else(|| "Cette machine n'est dans aucune salle PartaGPU.".to_string())
    }

    /// Encrypt with v=2 if peer publishes an ephemeral pubkey, otherwise v=1.
    /// Returns (envelope_json, session_key_for_response_decrypt).
    fn encrypt_for(
        room_key: &[u8; 32],
        peer_eph_pk: &str,
        plaintext_json: &str,
    ) -> Result<(String, [u8; 32]), String> {
        if peer_eph_pk.is_empty() {
            let env =
                crypto::encrypt(room_key, plaintext_json.as_bytes()).map_err(|e| e.to_string())?;
            let s =
                serde_json::to_string(&env).map_err(|e| format!("envelope sérialisation : {e}"))?;
            Ok((s, *room_key))
        } else {
            let (env, session) =
                crypto::encrypt_v2(room_key, peer_eph_pk, plaintext_json.as_bytes())
                    .map_err(|e| e.to_string())?;
            let s =
                serde_json::to_string(&env).map_err(|e| format!("envelope sérialisation : {e}"))?;
            Ok((s, session))
        }
    }

    let submit_path = "/peer/v1/tasks";
    let url_submit = format!("http://{peer_ip}:{PEER_PORT}{submit_path}");
    let body = serde_json::json!({
        "args": args,
        "source_user": user,
        "timeout_secs": timeout_secs,
        "network_enabled": network_enabled,
        "workspace": workspace,
    });
    let body_str = serde_json::to_string(&body).map_err(|e| format!("JSON sérialisation : {e}"))?;
    let (body_env, submit_session_key) = encrypt_for(key, peer_eph_pk, &body_str)?;
    let submit_auth = auth_header(auth, "POST", submit_path, body_env.as_bytes())?;

    let resp = ureq::post(&url_submit)
        .set("X-PartaGPU-AUTH", &submit_auth)
        .set("Content-Type", ENCRYPTED_CONTENT_TYPE)
        .timeout(Duration::from_secs(15))
        .send_string(&body_env)
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
    let resp_body = resp
        .into_string()
        .map_err(|e| format!("lecture réponse pair : {e}"))?;
    let submit: SubmitResp = crypto::decrypt_json(&submit_session_key, &resp_body)
        .map_err(|e| format!("réponse du pair non déchiffrable : {e}"))?;

    // Remember which peer task corresponds to this local id, so a future
    // cancel can be propagated.
    outgoing.set_remote_ref(local_id, peer_ip, &submit.task_id);
    outgoing.update_progress(local_id, 5.0, TaskStatus::Running);

    // Poll until terminal state, with a wall-clock budget = task timeout + grace.
    let deadline = Instant::now() + Duration::from_secs(timeout_secs.saturating_add(30));
    let get_path = format!("/peer/v1/tasks/{}", submit.task_id);
    let url_get = format!("http://{peer_ip}:{PEER_PORT}{get_path}");

    loop {
        if Instant::now() > deadline {
            return Err("dépassement du délai d'attente côté local".into());
        }
        std::thread::sleep(POLL_INTERVAL);

        // GET has no body so the HMAC covers the empty byte slice ; we
        // recompute the header on every poll because the timestamp must
        // be fresh (server rejects anything outside its 30 s window).
        let poll_auth = auth_header(auth, "GET", &get_path, b"")?;
        let r = match ureq::get(&url_get)
            .set("X-PartaGPU-AUTH", &poll_auth)
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

        let body = match r.into_string() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("dispatch poll body: {e}");
                continue;
            }
        };
        // GET has no body, so the server response is encrypted with the
        // *room key* (v=2 path doesn't apply since there's no client eph_pk
        // for the request). Use the room key to decrypt.
        let task: Task = match crypto::decrypt_json(key, &body) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("dispatch poll decrypt: {e}");
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

/// Minimal JSON string escaper for one-off error bodies built before we
/// hit serde_json. Just covers the characters that would break a literal
/// `{"error":"…"}` envelope.
fn json_string_escape(s: &str) -> String {
    let escaped: String = s
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\r' => vec!['\\', 'r'],
            '\t' => vec!['\\', 't'],
            c if (c as u32) < 0x20 => format!("\\u{:04x}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}

/// Verify the request looks like a local CLI/SDK client and not a browser
/// pivot. Returns `Some((status, msg))` if the request must be rejected.
///
/// Two checks:
/// - `Host` must be `127.0.0.1:7654` or `localhost:7654` (case-insensitive).
///   This catches DNS rebinding (`evil.com` → 127.0.0.1) which preserves the
///   original Host header even though the TCP connection lands here.
/// - `Origin` must be absent. `requests`, `curl`, and Tauri's internal IPC
///   never set Origin ; browsers always set it on cross-origin fetch.
fn check_local_origin(req: &Request) -> Option<(&'static str, &'static str)> {
    let host = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("host"))
        .map(|(_, v)| v.trim().to_ascii_lowercase());
    match host.as_deref() {
        Some("127.0.0.1:7654") | Some("localhost:7654") => {}
        _ => {
            return Some((
                "403 Forbidden",
                "Host header must be 127.0.0.1:7654 (local CLI/SDK only).",
            ));
        }
    }

    let has_origin = req
        .headers
        .iter()
        .any(|(k, v)| k.eq_ignore_ascii_case("origin") && !v.trim().is_empty());
    if has_origin {
        return Some((
            "403 Forbidden",
            "Cross-origin requests are not accepted (local CLI/SDK only).",
        ));
    }

    None
}

async fn write_short_response(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: application/json\r\n\
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

    let header_part =
        std::str::from_utf8(&buf[..header_end]).map_err(|_| "en-têtes non UTF-8".to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn req_with_headers(headers: Vec<(&str, &str)>) -> Request {
        Request {
            method: "POST".into(),
            path: "/api/dispatch".into(),
            body: String::new(),
            headers: headers
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    #[test]
    fn local_origin_accepts_localhost_host() {
        let req = req_with_headers(vec![("Host", "127.0.0.1:7654")]);
        assert!(check_local_origin(&req).is_none());
    }

    #[test]
    fn local_origin_accepts_localhost_alias() {
        let req = req_with_headers(vec![("Host", "localhost:7654")]);
        assert!(check_local_origin(&req).is_none());
    }

    #[test]
    fn local_origin_rejects_dns_rebind_host() {
        // DNS rebinding leaves the original Host even when the connection
        // lands on 127.0.0.1.
        let req = req_with_headers(vec![("Host", "evil.com:7654")]);
        assert_eq!(
            check_local_origin(&req).map(|(s, _)| s),
            Some("403 Forbidden")
        );
    }

    #[test]
    fn local_origin_rejects_missing_host() {
        let req = req_with_headers(vec![]);
        assert_eq!(
            check_local_origin(&req).map(|(s, _)| s),
            Some("403 Forbidden")
        );
    }

    #[test]
    fn local_origin_rejects_browser_origin_header() {
        let req = req_with_headers(vec![
            ("Host", "127.0.0.1:7654"),
            ("Origin", "https://evil.com"),
        ]);
        assert_eq!(
            check_local_origin(&req).map(|(s, _)| s),
            Some("403 Forbidden")
        );
    }

    #[test]
    fn local_origin_accepts_empty_origin_header() {
        // Some clients set an empty Origin (uncommon but harmless) ; we
        // tolerate that and only reject non-empty values.
        let req = req_with_headers(vec![("Host", "127.0.0.1:7654"), ("Origin", "")]);
        assert!(check_local_origin(&req).is_none());
    }
}
