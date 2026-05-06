//! Peer-to-peer HTTP server on 0.0.0.0:7655.
//!
//! Accepts compute task submissions from other PartaGPU machines on the LAN.
//! Authentication is shared-secret TOTP (same secret as the room) carried in
//! the `X-PartaGPU-TOTP` header — every member of the room can compute the
//! current code, no other machine on the network can.
//!
//! Routes:
//!   GET  /peer/v1/health        → liveness + room state (no auth)
//!   POST /peer/v1/tasks         → submit a task (TOTP required)
//!   GET  /peer/v1/tasks/<id>    → fetch task status/output (TOTP required)

use crate::auth::AuthManager;
use crate::crypto::{self, ENCRYPTED_CONTENT_TYPE};
use crate::discovery::Discovery;
use crate::sandbox::{SandboxOptions, WorkspaceFile};
use crate::security_log::{EventCategory, EventLevel, SecurityLog};
use crate::sharing::{SharingController, SharingStatus};
use crate::task_runner::IncomingTasks;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const LISTEN_ADDR: &str = "0.0.0.0:7655";
const TOTP_HEADER: &str = "x-partagpu-totp";
/// Cap on raw request size (post-base64, post-encryption-envelope). Sized
/// to comfortably hold a 16 MB sandbox workspace after JSON+base64+encrypt
/// inflation (~28 MB worst case).
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;

#[derive(Deserialize)]
struct SubmitBody {
    args: Vec<String>,
    #[serde(default)]
    source_user: String,
    #[serde(default)]
    timeout_secs: Option<u64>,
    /// Whether the sandbox should keep host network access (DDP rendezvous).
    #[serde(default)]
    network_enabled: bool,
    /// Files to materialize in /workspace before exec.
    #[serde(default)]
    workspace: Vec<WorkspaceFile>,
}

#[derive(Serialize)]
struct SubmitResponse {
    task_id: String,
    accepted: bool,
}

#[derive(Serialize)]
struct HealthResponse {
    hostname: String,
    version: &'static str,
    in_room: bool,
    sharing_active: bool,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

pub fn start(
    incoming: IncomingTasks,
    auth: AuthManager,
    discovery: Discovery,
    sharing: SharingController,
    sec_log: SecurityLog,
) {
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("Peer API: failed to build tokio runtime: {e}");
                return;
            }
        };

        runtime.block_on(async move {
            let listener = match TcpListener::bind(LISTEN_ADDR).await {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Peer API: failed to bind {LISTEN_ADDR}: {e}");
                    return;
                }
            };
            eprintln!("Peer API listening on {LISTEN_ADDR}");

            loop {
                let (stream, addr) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };

                let incoming = incoming.clone();
                let auth = auth.clone();
                let discovery = discovery.clone();
                let sharing = sharing.clone();
                let sec_log = sec_log.clone();

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_connection(stream, addr, incoming, auth, discovery, sharing, sec_log)
                            .await
                    {
                        eprintln!("Peer API: connection error from {addr}: {e}");
                    }
                });
            }
        });
    });
}

async fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    incoming: IncomingTasks,
    auth: AuthManager,
    discovery: Discovery,
    sharing: SharingController,
    sec_log: SecurityLog,
) -> Result<(), String> {
    let mut req = read_request(&mut stream).await?;

    // /health is the only unauthenticated, unencrypted endpoint (used as a
    // probe). Everything under /peer/v1/tasks must be encrypted.
    let route_needs_encryption = req.path.starts_with("/peer/v1/tasks");

    // Try to derive the room key. Required for encrypted routes; absent if
    // we're not in a room (auth.get_secret() returns None).
    let room_key: Option<[u8; 32]> = auth
        .get_secret()
        .and_then(|s| crypto::derive_room_key(&s).ok());

    // Decrypt request body if we're on an encrypted route and the body is
    // non-empty (POST). Replace req.body with the plaintext so handlers
    // continue to expect plain JSON.
    let body_decrypt_error = if route_needs_encryption && !req.body.is_empty() {
        let content_type = req
            .headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
            .map(|(_, v)| v.as_str())
            .unwrap_or("");
        if !content_type.contains(ENCRYPTED_CONTENT_TYPE) {
            Some(format!(
                "Content-Type doit être {ENCRYPTED_CONTENT_TYPE} (chiffrement obligatoire entre pairs)"
            ))
        } else {
            match &room_key {
                None => Some(
                    "Cette machine n'est dans aucune salle PartaGPU."
                        .to_string(),
                ),
                Some(key) => {
                    match serde_json::from_str::<crypto::Envelope>(&req.body) {
                        Ok(env) => match crypto::decrypt(key, &env) {
                            Ok(plain) => match String::from_utf8(plain) {
                                Ok(s) => {
                                    req.body = s;
                                    None
                                }
                                Err(_) => Some("plaintext non UTF-8".to_string()),
                            },
                            Err(e) => Some(e),
                        },
                        Err(e) => {
                            Some(format!("envelope JSON invalide : {e}"))
                        }
                    }
                }
            }
        }
    } else {
        None
    };

    let (status, body) = if let Some(err) = body_decrypt_error {
        (
            "415 Unsupported Media Type",
            json_string(&ErrorResponse { error: err }),
        )
    } else {
        match (req.method.as_str(), req.path.as_str()) {
            ("GET", "/peer/v1/health") => handle_health(&auth, &sharing),
            ("POST", "/peer/v1/tasks") => handle_submit(
                &req, &addr, &incoming, &auth, &discovery, &sharing, &sec_log,
            ),
            ("GET", path) if path.starts_with("/peer/v1/tasks/") => {
                let id = &path["/peer/v1/tasks/".len()..];
                handle_get_task(id, &req, &incoming, &auth, &sharing)
            }
            ("DELETE", path) if path.starts_with("/peer/v1/tasks/") => {
                let id = &path["/peer/v1/tasks/".len()..];
                handle_cancel_task(id, &req, &addr, &incoming, &auth, &sharing, &sec_log)
            }
            _ => (
                "404 Not Found",
                json_string(&ErrorResponse {
                    error: "Route inconnue.".into(),
                }),
            ),
        }
    };

    // Encrypt 2xx response bodies on encrypted routes. Errors stay plain
    // (the caller may not have the key — that's why the call failed).
    let (final_body, content_type) = if route_needs_encryption
        && status.starts_with('2')
        && !body.is_empty()
        && room_key.is_some()
    {
        match crypto::encrypt(&room_key.unwrap(), body.as_bytes()) {
            Ok(env) => match serde_json::to_string(&env) {
                Ok(s) => (s, ENCRYPTED_CONTENT_TYPE),
                Err(_) => (body, "application/json"),
            },
            Err(_) => (body, "application/json"),
        }
    } else {
        (body, "application/json")
    };

    write_response(&mut stream, status, &final_body, content_type).await
}

// ── Handlers ───────────────────────────────────────────────────────────────

fn handle_health(auth: &AuthManager, sharing: &SharingController) -> (&'static str, String) {
    let resp = HealthResponse {
        hostname: hostname::get()
            .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".into()),
        version: env!("CARGO_PKG_VERSION"),
        in_room: auth.is_joined(),
        sharing_active: sharing.get_config().status == SharingStatus::Active,
    };
    ("200 OK", json_string(&resp))
}

fn handle_submit(
    req: &Request,
    addr: &SocketAddr,
    incoming: &IncomingTasks,
    auth: &AuthManager,
    discovery: &Discovery,
    sharing: &SharingController,
    sec_log: &SecurityLog,
) -> (&'static str, String) {
    if let Err((code, msg)) = check_auth(req, auth, sharing) {
        sec_log.peer_event(
            EventCategory::TaskRejected,
            &format!("Tâche refusée de {} : {msg}", addr.ip()),
            &addr.ip().to_string(),
            "",
        );
        return (code, json_string(&ErrorResponse { error: msg }));
    }

    let body: SubmitBody = match serde_json::from_str(&req.body) {
        Ok(b) => b,
        Err(e) => {
            return (
                "400 Bad Request",
                json_string(&ErrorResponse {
                    error: format!("Corps JSON invalide : {e}"),
                }),
            );
        }
    };

    if body.args.is_empty() {
        return (
            "400 Bad Request",
            json_string(&ErrorResponse {
                error: "Le champ 'args' est requis et non vide.".into(),
            }),
        );
    }

    // Resolve a friendly source name from the peer list (best effort).
    // Prefer the user-chosen display_name (e.g. "César 1") over the system
    // hostname (e.g. "cesar-Precision-3650-Tower"), falling back to the
    // hostname then the IP if neither is available.
    let peer_ip = addr.ip().to_string();
    let source_machine = discovery
        .get_peers()
        .into_iter()
        .find(|p| p.ip == peer_ip)
        .map(|p| {
            if !p.display_name.is_empty() {
                p.display_name
            } else if !p.hostname.is_empty() {
                p.hostname
            } else {
                peer_ip.clone()
            }
        })
        .unwrap_or_else(|| peer_ip.clone());

    let source_user = if body.source_user.is_empty() {
        "remote".into()
    } else {
        body.source_user
    };

    let target_machine = hostname::get()
        .map(|h: std::ffi::OsString| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "local".into());

    let timeout = body.timeout_secs.unwrap_or(3600).min(24 * 3600);

    let net_tag = if body.network_enabled { " [reseau]" } else { "" };
    sec_log.log(
        EventLevel::Info,
        EventCategory::TaskSubmitted,
        &format!(
            "Tâche acceptée de {source_machine} ({source_user}){net_tag} : {}",
            body.args.join(" ")
        ),
        Some(&peer_ip),
        Some(&source_machine),
    );

    let options = SandboxOptions {
        network_enabled: body.network_enabled,
        workspace: body.workspace,
    };

    match incoming.create_and_run(
        body.args,
        source_machine,
        source_user,
        target_machine,
        timeout,
        options,
    ) {
        Ok(task) => (
            "200 OK",
            json_string(&SubmitResponse {
                task_id: task.id,
                accepted: true,
            }),
        ),
        Err(e) => (
            "500 Internal Server Error",
            json_string(&ErrorResponse { error: e }),
        ),
    }
}

fn handle_get_task(
    id: &str,
    req: &Request,
    incoming: &IncomingTasks,
    auth: &AuthManager,
    sharing: &SharingController,
) -> (&'static str, String) {
    if let Err((code, msg)) = check_auth(req, auth, sharing) {
        return (code, json_string(&ErrorResponse { error: msg }));
    }
    match incoming.get(id) {
        Some(task) => (
            "200 OK",
            serde_json::to_string(&task).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}")),
        ),
        None => (
            "404 Not Found",
            json_string(&ErrorResponse {
                error: "Tâche introuvable.".into(),
            }),
        ),
    }
}

fn handle_cancel_task(
    id: &str,
    req: &Request,
    addr: &SocketAddr,
    incoming: &IncomingTasks,
    auth: &AuthManager,
    sharing: &SharingController,
    sec_log: &SecurityLog,
) -> (&'static str, String) {
    if let Err((code, msg)) = check_auth(req, auth, sharing) {
        return (code, json_string(&ErrorResponse { error: msg }));
    }
    match incoming.cancel(id) {
        Ok(()) => {
            sec_log.peer_event(
                EventCategory::TaskRejected,
                &format!("Tâche {id} annulée à la demande du pair {}", addr.ip()),
                &addr.ip().to_string(),
                "",
            );
            ("200 OK", "{\"cancelled\":true}".to_string())
        }
        Err(e) => (
            "404 Not Found",
            json_string(&ErrorResponse { error: e }),
        ),
    }
}

// ── Auth ───────────────────────────────────────────────────────────────────

fn check_auth(
    req: &Request,
    auth: &AuthManager,
    sharing: &SharingController,
) -> Result<(), (&'static str, String)> {
    if !auth.is_joined() {
        return Err((
            "403 Forbidden",
            "Cette machine n'est dans aucune salle PartaGPU.".into(),
        ));
    }
    if sharing.get_config().status != SharingStatus::Active {
        return Err((
            "403 Forbidden",
            "Le partage de ressources n'est pas activé sur cette machine.".into(),
        ));
    }
    let code = req
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(TOTP_HEADER))
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();
    if code.is_empty() {
        return Err((
            "401 Unauthorized",
            format!("Header '{TOTP_HEADER}' manquant."),
        ));
    }
    if !auth.verify_code(&code) {
        return Err((
            "401 Unauthorized",
            "Code TOTP invalide ou expiré.".into(),
        ));
    }
    Ok(())
}

// ── HTTP plumbing ──────────────────────────────────────────────────────────

struct Request {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: String,
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
            return Err("connexion fermée avant la fin des en-têtes".into());
        }
        buf.extend_from_slice(&tmp[..n]);
        if buf.len() > MAX_REQUEST_BYTES {
            return Err("requête trop volumineuse".into());
        }
        if let Some(p) = find_double_crlf(&buf) {
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

    // Read remaining body if Content-Length specifies more than what we already buffered.
    let mut body_bytes = buf[header_end + 4..].to_vec();
    while body_bytes.len() < content_length {
        let need = content_length - body_bytes.len();
        if body_bytes.len() + need > MAX_REQUEST_BYTES {
            return Err("corps trop volumineux".into());
        }
        let n = stream
            .read(&mut tmp)
            .await
            .map_err(|e| format!("read body: {e}"))?;
        if n == 0 {
            break;
        }
        body_bytes.extend_from_slice(&tmp[..n.min(need + 4096)]);
    }
    body_bytes.truncate(content_length);
    let body = String::from_utf8(body_bytes).map_err(|_| "corps non UTF-8".to_string())?;

    Ok(Request {
        method,
        path,
        headers,
        body,
    })
}

fn find_double_crlf(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn write_response(
    stream: &mut TcpStream,
    status: &str,
    body: &str,
    content_type: &str,
) -> Result<(), String> {
    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
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

fn json_string<T: Serialize>(v: &T) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}
