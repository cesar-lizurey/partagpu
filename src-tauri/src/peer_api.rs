//! Peer-to-peer HTTP server on 0.0.0.0:7655.
//!
//! Accepts compute task submissions from other PartaGPU machines on the LAN.
//! Authentication is **HMAC-SHA256(auth_key, ts || method || path || body_hash)**
//! carried in the `X-PartaGPU-AUTH: <unix_ts>:<hex_hmac>` header — every
//! member of the room can compute it, nobody else can. The HMAC is validated
//! on the **wire bytes** (encrypted envelope) before the body is decrypted,
//! so an attacker who can't compute the HMAC never reaches the AES layer.
//!
//! Routes:
//!   GET    /peer/v1/health        → liveness + room state (no auth)
//!   POST   /peer/v1/tasks         → submit a task (auth required)
//!   GET    /peer/v1/tasks/<id>    → fetch task status/output (auth required)
//!   DELETE /peer/v1/tasks/<id>    → cancel task (auth required)

use crate::auth::AuthManager;
use crate::crypto::{self, EphemeralKey, ENCRYPTED_CONTENT_TYPE};
use crate::discovery::Discovery;
use crate::sandbox::{SandboxOptions, WorkspaceFile};
use crate::security_log::{EventCategory, EventLevel, SecurityLog};
use crate::sharing::{SharingController, SharingStatus};
use crate::task_runner::IncomingTasks;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;

const LISTEN_ADDR: &str = "0.0.0.0:7655";
const AUTH_HEADER: &str = "x-partagpu-auth";
/// Cap on raw request size (post-base64, post-encryption-envelope). Sized
/// to comfortably hold a 16 MB sandbox workspace after JSON+base64+encrypt
/// inflation (~28 MB worst case).
const MAX_REQUEST_BYTES: usize = 32 * 1024 * 1024;
/// Cap on the number of concurrent peer-API connections. Without this an
/// attacker on the LAN could open thousands of TCP streams and have each
/// hold a 32 MB read buffer in flight, exhausting RAM. 64 is generous for
/// a classroom (a single DDP run uses ≤ N peers × ~1 conn) but strict
/// enough that 1 000 spurious connections get queued behind the semaphore
/// instead of all spawning at once.
const MAX_CONCURRENT_CONNECTIONS: usize = 64;
/// Anti-replay cache : remember every successfully-authenticated
/// `X-PartaGPU-AUTH` header for two clock-skew windows. The HMAC scheme
/// already binds auth to ts + method + path + body_hash, so two requests
/// with the same header are by definition byte-identical replays. Cap on
/// entries below acts as a safety net if pruning lags.
const REPLAY_RETENTION_SECS: u64 = crate::crypto::AUTH_WINDOW_SECS * 2;
const REPLAY_CACHE_MAX_ENTRIES: usize = 4096;

/// In-memory cache of recently-seen, successfully-authenticated request
/// headers. `check_and_insert` returns `Ok(())` for a fresh request and
/// `Err(())` for an exact replay seen within `REPLAY_RETENTION_SECS`.
#[derive(Default)]
struct ReplayCache {
    entries: Mutex<HashMap<String, u64>>,
}

impl ReplayCache {
    fn check_and_insert(&self, header: &str) -> Result<(), ()> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let mut m = match self.entries.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(), // poisoned : recover and continue
        };
        // Drop everything older than the retention window. O(n) — acceptable
        // at our scale (entries are bounded by REPLAY_CACHE_MAX_ENTRIES).
        m.retain(|_, ts| now.saturating_sub(*ts) < REPLAY_RETENTION_SECS);
        // Hard cap : if we ever blow past the cap, just clear the cache.
        // Worst case a few legitimate replays slip through during the
        // catch-up, no security impact.
        if m.len() >= REPLAY_CACHE_MAX_ENTRIES {
            m.clear();
        }
        if m.contains_key(header) {
            return Err(());
        }
        m.insert(header.to_string(), now);
        Ok(())
    }
}

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
    /// Chemins relatifs (depuis /workspace) que le client veut recuperer
    /// apres exit. Le champ est attache aux SandboxOptions ; les fichiers
    /// produits sont rapatries en base64 dans `Task.artifacts`.
    #[serde(default)]
    outputs: Vec<String>,
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

/// Start the peer-API listening on the canonical port (0.0.0.0:7655).
/// The integration tests use `start_on_addr` instead so they can bind to
/// 127.0.0.1:0 and avoid port conflicts.
pub fn start(
    incoming: IncomingTasks,
    auth: AuthManager,
    discovery: Discovery,
    sharing: SharingController,
    sec_log: SecurityLog,
    server_eph: EphemeralKey,
) {
    let _ = start_on_addr(
        LISTEN_ADDR,
        incoming,
        auth,
        discovery,
        sharing,
        sec_log,
        server_eph,
    );
}

/// Like `start` but binds to a caller-specified address and returns the
/// actual bound port (useful when the address ends in `:0`). Bind happens
/// synchronously in the calling thread so the listener is ready before
/// this function returns ; the accept loop runs in a background thread.
pub fn start_on_addr(
    addr: &str,
    incoming: IncomingTasks,
    auth: AuthManager,
    discovery: Discovery,
    sharing: SharingController,
    sec_log: SecurityLog,
    server_eph: EphemeralKey,
) -> Result<u16, String> {
    let std_listener = std::net::TcpListener::bind(addr)
        .map_err(|e| format!("Peer API: failed to bind {addr}: {e}"))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| format!("Peer API: set_nonblocking failed: {e}"))?;
    let port = std_listener
        .local_addr()
        .map(|a| a.port())
        .map_err(|e| format!("Peer API: local_addr failed: {e}"))?;

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
            let listener = match TcpListener::from_std(std_listener) {
                Ok(l) => l,
                Err(e) => {
                    eprintln!("Peer API: from_std failed: {e}");
                    return;
                }
            };
            eprintln!("Peer API listening on port {port}");

            // Bound concurrency so a flood of LAN connections can't OOM us.
            // We acquire a permit BEFORE accepting the next stream — the
            // listener's TCP backlog absorbs the queue, and the OS will
            // start refusing connections beyond the kernel limit, which is
            // exactly what we want.
            let conn_sem = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
            // Shared replay-cache across all connections (one process-wide
            // instance — replays would naturally come from different conns).
            let replay_cache = Arc::new(ReplayCache::default());

            loop {
                // Hold the permit for the lifetime of the connection's
                // tokio task ; dropping the permit releases the slot.
                let permit = match conn_sem.clone().acquire_owned().await {
                    Ok(p) => p,
                    Err(_) => return, // semaphore closed → shutdown
                };

                let (stream, addr) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => continue,
                };

                let incoming = incoming.clone();
                let auth = auth.clone();
                let discovery = discovery.clone();
                let sharing = sharing.clone();
                let sec_log = sec_log.clone();
                let server_eph = server_eph.clone();
                let replay_cache = replay_cache.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_connection(
                        stream,
                        addr,
                        incoming,
                        auth,
                        discovery,
                        sharing,
                        sec_log,
                        server_eph,
                        replay_cache,
                    )
                    .await
                    {
                        eprintln!("Peer API: connection error from {addr}: {e}");
                    }
                    drop(permit);
                });
            }
        });
    });

    Ok(port)
}

#[allow(clippy::too_many_arguments)] // protocol-driven, every arg is required
async fn handle_connection(
    mut stream: TcpStream,
    addr: SocketAddr,
    incoming: IncomingTasks,
    auth: AuthManager,
    discovery: Discovery,
    sharing: SharingController,
    sec_log: SecurityLog,
    server_eph: EphemeralKey,
    replay_cache: Arc<ReplayCache>,
) -> Result<(), String> {
    let mut req = read_request(&mut stream).await?;

    // /health and /verify are the unauthenticated, unencrypted endpoints
    // (used as bootstrap probes). Everything under /peer/v1/tasks must be
    // encrypted. /verify in particular IS the auth bootstrap : a peer that
    // hasn't been verified yet can't sign a request, so the route must be
    // open. Brute-force resistance comes from the PBKDF2-derived auth_key
    // (slow KDF) — collecting tags via /verify doesn't help the attacker.
    let route_needs_encryption = req.path.starts_with("/peer/v1/tasks");
    let route_needs_auth = route_needs_encryption;

    // Try to derive the room key. Required for encrypted routes; absent if
    // we're not in a room (auth.get_secret() returns None).
    let room_key: Option<[u8; 32]> = auth
        .get_secret()
        .and_then(|s| crypto::derive_room_key(&s).ok());

    // ── Auth check (run BEFORE decryption) ────────────────────────────────
    // The HMAC is computed over the wire bytes (encrypted envelope JSON),
    // so an attacker who can't sign requests never reaches the AES-GCM
    // layer. Failures short-circuit with 401/403 without touching the
    // crypto pipeline below.
    let auth_failure: Option<(&'static str, String)> = if route_needs_auth {
        if !auth.is_joined() {
            Some((
                "403 Forbidden",
                "Cette machine n'est dans aucune salle PartaGPU.".into(),
            ))
        } else if sharing.get_config().status != SharingStatus::Active {
            Some((
                "403 Forbidden",
                "Le partage de ressources n'est pas activé sur cette machine.".into(),
            ))
        } else {
            let header_value = req
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(AUTH_HEADER))
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default();
            if header_value.is_empty() {
                Some((
                    "401 Unauthorized",
                    format!("Header '{AUTH_HEADER}' manquant."),
                ))
            } else {
                match auth.verify_request_auth(
                    &header_value,
                    &req.method,
                    &req.path,
                    req.body.as_bytes(),
                ) {
                    Ok(()) => {
                        // Auth passed — guard against an attacker replaying
                        // a captured (legitimate) request within the 30 s
                        // window. Two requests with the same auth header
                        // are byte-identical by construction (HMAC binds
                        // ts + method + path + body_hash) — they can only
                        // be replays.
                        if replay_cache.check_and_insert(&header_value).is_err() {
                            Some((
                                "401 Unauthorized",
                                "requête déjà reçue dans la fenêtre courante (replay)".into(),
                            ))
                        } else {
                            None
                        }
                    }
                    Err(e) => Some(("401 Unauthorized", format!("auth invalide : {e}"))),
                }
            }
        }
    } else {
        None
    };

    if let Some((status, msg)) = auth_failure {
        sec_log.peer_event(
            EventCategory::TaskRejected,
            &format!("Requête refusée de {} : {msg}", addr.ip()),
            &addr.ip().to_string(),
            "",
        );
        return write_response(
            &mut stream,
            status,
            &json_string(&ErrorResponse { error: msg }),
            "application/json",
        )
        .await;
    }

    // Decrypt request body if we're on an encrypted route and the body is
    // non-empty (POST). Replace req.body with the plaintext so handlers
    // continue to expect plain JSON. Capture the session key so the response
    // can be encrypted with the same key (v=2 forward-secret path). For
    // GET/DELETE requests there's no body to decrypt, so response defaults
    // to the room key — no forward secrecy on those, but they leak only the
    // task id and status which is bounded.
    let mut response_key: Option<[u8; 32]> = room_key;
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
                None => Some("Cette machine n'est dans aucune salle PartaGPU.".to_string()),
                Some(key) => match serde_json::from_str::<crypto::Envelope>(&req.body) {
                    Ok(env) => {
                        // Crypto layer returns typed `CryptoError` ; the
                        // peer-API handler propagates errors as String,
                        // so we collapse via `.map_err(|e| e.to_string())`
                        // at this boundary.
                        let result: Result<(Vec<u8>, [u8; 32]), String> = match env.v {
                            1 => crypto::decrypt(key, &env)
                                .map(|plain| (plain, *key))
                                .map_err(|e| e.to_string()),
                            2 => crypto::decrypt_request_v2(key, &server_eph, &env)
                                .map_err(|e| e.to_string()),
                            v => Err(format!("version d'enveloppe non supportée : {v}")),
                        };
                        match result {
                            Ok((plain, session_key)) => match String::from_utf8(plain) {
                                Ok(s) => {
                                    req.body = s;
                                    response_key = Some(session_key);
                                    None
                                }
                                Err(_) => Some("plaintext non UTF-8".to_string()),
                            },
                            Err(e) => Some(e),
                        }
                    }
                    Err(e) => Some(format!("envelope JSON invalide : {e}")),
                },
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
            (m, p) if m == "GET" && p.starts_with("/peer/v1/verify") => {
                handle_verify(&req.path, &auth)
            }
            ("POST", "/peer/v1/tasks") => {
                handle_submit(&req, &addr, &incoming, &discovery, &sharing, &sec_log)
            }
            ("GET", path) if path.starts_with("/peer/v1/tasks/") => {
                let id = &path["/peer/v1/tasks/".len()..];
                handle_get_task(id, &incoming)
            }
            ("DELETE", path) if path.starts_with("/peer/v1/tasks/") => {
                let id = &path["/peer/v1/tasks/".len()..];
                handle_cancel_task(id, &addr, &incoming, &sec_log)
            }
            _ => (
                "404 Not Found",
                json_string(&ErrorResponse {
                    error: "Route inconnue.".into(),
                }),
            ),
        }
    };

    // Encrypt 2xx response bodies on encrypted routes. Use the same session
    // key the request was decrypted with — that's the room key for v=1
    // requests, and a freshly-derived ECDH session key for v=2.
    // Errors stay plain (the caller may not have the key — that's why the
    // call failed).
    let (final_body, content_type) = if let Some(key) = response_key
        .filter(|_| route_needs_encryption && status.starts_with('2') && !body.is_empty())
    {
        match crypto::encrypt_with_session(&key, body.as_bytes()) {
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

#[derive(Serialize)]
struct VerifyResponse {
    hmac: String,
}

/// Handle `GET /peer/v1/verify?nonce=<hex>`. Unauthenticated by design : it
/// IS the auth bootstrap. Returns the HMAC of `(domain || nonce_bytes)`
/// keyed by `auth_key`, hex-encoded. The nonce is supplied by the verifier
/// and must be a fresh random value (the freshness is the verifier's
/// responsibility — the prover doesn't track it).
///
/// Validation : nonce is mandatory, hex-encoded, raw size in
/// `[VERIFY_NONCE_MIN_BYTES, VERIFY_NONCE_MAX_BYTES]`. Anything else is
/// 400. If we're not in a room, return 403 (we can't compute a valid
/// HMAC and shouldn't pretend).
fn handle_verify(full_path: &str, auth: &AuthManager) -> (&'static str, String) {
    // Parse the query string — the path arrives like
    // "/peer/v1/verify?nonce=<hex>". Cheap manual parse since we only
    // accept the one parameter.
    let nonce_hex = full_path
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or("")
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == "nonce")
        .map(|(_, v)| v.trim().to_string())
        .unwrap_or_default();

    if nonce_hex.is_empty() {
        return (
            "400 Bad Request",
            json_string(&ErrorResponse {
                error: "Paramètre 'nonce' manquant.".into(),
            }),
        );
    }
    let nonce_bytes = match data_encoding::HEXLOWER.decode(nonce_hex.as_bytes()) {
        Ok(b) => b,
        Err(_) => {
            // Try uppercase too — be permissive on the wire format.
            match data_encoding::HEXUPPER.decode(nonce_hex.as_bytes()) {
                Ok(b) => b,
                Err(_) => {
                    return (
                        "400 Bad Request",
                        json_string(&ErrorResponse {
                            error: "Nonce invalide (hex attendu).".into(),
                        }),
                    );
                }
            }
        }
    };
    if nonce_bytes.len() < crypto::VERIFY_NONCE_MIN_BYTES
        || nonce_bytes.len() > crypto::VERIFY_NONCE_MAX_BYTES
    {
        return (
            "400 Bad Request",
            json_string(&ErrorResponse {
                error: format!(
                    "Nonce hors plage : {} octets (autorisé : {}–{}).",
                    nonce_bytes.len(),
                    crypto::VERIFY_NONCE_MIN_BYTES,
                    crypto::VERIFY_NONCE_MAX_BYTES,
                ),
            }),
        );
    }

    match auth.compute_verify_response(&nonce_bytes) {
        Some(hmac) => ("200 OK", json_string(&VerifyResponse { hmac })),
        None => (
            "403 Forbidden",
            json_string(&ErrorResponse {
                error: "Cette machine n'est dans aucune salle PartaGPU.".into(),
            }),
        ),
    }
}

fn handle_submit(
    req: &Request,
    addr: &SocketAddr,
    incoming: &IncomingTasks,
    discovery: &Discovery,
    sharing: &SharingController,
    sec_log: &SecurityLog,
) -> (&'static str, String) {
    // Auth is already validated by handle_connection upstream — handlers
    // can assume the room/sharing/HMAC checks all passed.

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

    let net_tag = if body.network_enabled {
        " [reseau]"
    } else {
        ""
    };
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

    // Snapshot the local GPU limit so the sandbox can cap the task's GPU
    // SM share via CUDA MPS (set as an env var read by the CUDA driver).
    // Read here (not at task spawn) so the value reflects what the user
    // had configured at task-acceptance time.
    let gpu_limit = {
        let cfg = sharing.get_config();
        if cfg.gpu_limit_percent > 0 && cfg.gpu_limit_percent < 100 {
            Some(cfg.gpu_limit_percent)
        } else {
            None
        }
    };
    let options = SandboxOptions {
        network_enabled: body.network_enabled,
        workspace: body.workspace,
        gpu_limit_percent: gpu_limit,
        outputs: body.outputs,
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

fn handle_get_task(id: &str, incoming: &IncomingTasks) -> (&'static str, String) {
    // Auth already validated upstream by handle_connection.
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
    addr: &SocketAddr,
    incoming: &IncomingTasks,
    sec_log: &SecurityLog,
) -> (&'static str, String) {
    // Auth already validated upstream by handle_connection.
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
        Err(e) => ("404 Not Found", json_string(&ErrorResponse { error: e })),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_cache_accepts_first_then_rejects_dup() {
        let cache = ReplayCache::default();
        assert!(cache.check_and_insert("header-A").is_ok());
        // Same header within window → replay rejected.
        assert!(cache.check_and_insert("header-A").is_err());
        // Different header → still accepted.
        assert!(cache.check_and_insert("header-B").is_ok());
    }

    #[test]
    fn replay_cache_clears_when_full() {
        let cache = ReplayCache::default();
        // Pump enough distinct headers to exceed the cap.
        for i in 0..(REPLAY_CACHE_MAX_ENTRIES + 10) {
            assert!(cache.check_and_insert(&format!("h-{i}")).is_ok());
        }
        // After overflow the cache was cleared, so an old header that
        // would have collided is now accepted again. Acceptable: the
        // overflow path is a safety net, not a correctness guarantee.
        assert!(cache.check_and_insert("h-0").is_ok());
    }
}
