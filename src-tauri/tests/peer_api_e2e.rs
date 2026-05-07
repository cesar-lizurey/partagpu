//! End-to-end integration tests for the peer-to-peer HTTP API.
//!
//! These tests start a real `peer_api::start_on_addr` server bound to
//! `127.0.0.1:0` (random port) in the same process, then talk to it over
//! ureq. Each test isolates the on-disk state by setting `HOME` to a temp
//! directory before constructing `AuthManager` / `IncomingTasks` so the
//! user's real config is never touched.
//!
//! What the tests cover :
//! - v=2 forward-secret round-trip submits a task and reads it back
//! - rejection of plaintext bodies (415, but only when auth is provided)
//! - rejection of bodies without the `X-PartaGPU-AUTH` header (401)
//! - rejection of envelopes from a different room (401, HMAC mismatch)
//! - cancel propagation (DELETE /peer/v1/tasks/<id>)
//!
//! What they DON'T cover : the actual sandbox execution. We submit
//! `args = ["true"]` ; whether bwrap launches successfully isn't important
//! to the protocol layer, and CI doesn't ship a usable bwrap.

use partagpu_lib::auth::AuthManager;
use partagpu_lib::crypto::{self, Envelope, EphemeralKey, ENCRYPTED_CONTENT_TYPE};
use partagpu_lib::discovery::Discovery;
use partagpu_lib::peer_api;
use partagpu_lib::sandbox::Sandbox;
use partagpu_lib::security_log::SecurityLog;
use partagpu_lib::sharing::SharingController;
use partagpu_lib::task_runner::IncomingTasks;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Each test gets its own temp $HOME so on-disk persistence (room.json,
/// incoming-tasks.json, machine-id) doesn't bleed between tests or into
/// the user's real config.
struct TestEnv {
    _tmp: tempdir::TempDir,
    _guard: std::sync::MutexGuard<'static, ()>,
}

/// AuthManager / IncomingTasks read $HOME at construction time. To run
/// several tests sequentially with different $HOME values we serialize
/// access via this lock — the env var is process-global.
static HOME_LOCK: Mutex<()> = Mutex::new(());

fn fresh_env() -> TestEnv {
    let guard = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir::TempDir::new("partagpu-e2e").expect("tempdir");
    std::env::set_var("HOME", tmp.path());
    std::env::remove_var("XDG_CONFIG_HOME");
    TestEnv {
        _tmp: tmp,
        _guard: guard,
    }
}

/// Start a peer-API server bound to a random localhost port and return
/// (port, ephemeral pubkey, room secret base32). The caller can then build
/// requests against `http://127.0.0.1:<port>`.
fn start_test_server() -> (u16, EphemeralKey, String) {
    static MACHINE_COUNT: AtomicUsize = AtomicUsize::new(0);
    let id = MACHINE_COUNT.fetch_add(1, Ordering::Relaxed);

    let auth = AuthManager::new();
    let room = auth
        .create_room(&format!("test-room-{id}"))
        .expect("create_room");
    let secret_b32 = room.secret_base32;

    let sharing = SharingController::new();
    // The peer-API needs sharing.status == Active to accept tasks. We can't
    // call `sharing.enable()` from tests because it tries to use pkexec to
    // create the partagpu user. Instead we toggle status in-memory only
    // via a dedicated test-only method (see crates/lib changes).
    sharing.force_active_for_tests();

    let discovery =
        Discovery::new(&format!("test-host-{id}"), &format!("mid-{id}")).expect("Discovery::new");

    let sandbox = Sandbox::new();
    let incoming = IncomingTasks::new(sandbox);

    let sec_log = SecurityLog::new();
    let server_eph = EphemeralKey::generate();

    let port = peer_api::start_on_addr(
        "127.0.0.1:0",
        incoming,
        auth,
        discovery,
        sharing,
        sec_log,
        server_eph.clone(),
    )
    .expect("start_on_addr");

    // Give the spawned tokio runtime a moment to start its accept loop.
    std::thread::sleep(std::time::Duration::from_millis(50));

    (port, server_eph, secret_b32)
}

/// Build the value of the `X-PartaGPU-AUTH` header for a (method, path, body)
/// triple, signing with the room's auth key. Same primitive the real
/// `http_api::dispatch_task_blocking` uses on the wire.
fn auth_header(secret_b32: &str, method: &str, path: &str, body: &[u8]) -> String {
    let auth_key = crypto::derive_auth_key(secret_b32).expect("derive_auth_key");
    crypto::compute_request_auth(&auth_key, method, path, body)
}

#[test]
fn rejects_plaintext_body() {
    // With a valid auth header, a plaintext body (wrong Content-Type) must
    // be rejected with 415. The auth check passes but the encryption-layer
    // gate refuses the request.
    let _env = fresh_env();
    let (port, _eph, secret_b32) = start_test_server();
    let path = "/peer/v1/tasks";
    let body = r#"{"args":["true"]}"#;
    let url = format!("http://127.0.0.1:{port}{path}");
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .set(
            "X-PartaGPU-AUTH",
            &auth_header(&secret_b32, "POST", path, body.as_bytes()),
        )
        .send_string(body);
    let err = resp.expect_err("plaintext body must be rejected");
    match err {
        ureq::Error::Status(code, _) => assert_eq!(code, 415),
        e => panic!("expected 415, got {e}"),
    }
}

#[test]
fn rejects_v2_without_room_membership() {
    // Server in a room of its own, but client builds an auth header keyed
    // by a *different* room secret. The HMAC check fails before decryption
    // is even attempted ; the client gets 401.
    let _env = fresh_env();
    let (port, server_eph, _server_secret) = start_test_server();

    let other_secret_b32 = data_encoding::BASE32.encode(b"some-other-room!!!!!!!!!!!!!!!!!");
    let other_key = crypto::derive_room_key(&other_secret_b32).unwrap();
    let body = serde_json::json!({"args": ["true"], "source_user": "x"});
    let body_str = serde_json::to_string(&body).unwrap();
    let (env, _) =
        crypto::encrypt_v2(&other_key, &server_eph.public_b64(), body_str.as_bytes()).unwrap();
    let env_json = serde_json::to_string(&env).unwrap();
    let path = "/peer/v1/tasks";
    let bad_auth = auth_header(&other_secret_b32, "POST", path, env_json.as_bytes());

    let url = format!("http://127.0.0.1:{port}{path}");
    let resp = ureq::post(&url)
        .set("Content-Type", ENCRYPTED_CONTENT_TYPE)
        .set("X-PartaGPU-AUTH", &bad_auth)
        .send_string(&env_json);
    match resp {
        Err(ureq::Error::Status(401, _)) => {}
        Ok(r) => panic!("unexpected success {}", r.status()),
        Err(e) => panic!("expected 401, got {e}"),
    }
}

#[test]
fn v2_round_trip_accepts_task() {
    let _env = fresh_env();
    let (port, server_eph, secret_b32) = start_test_server();
    let room_key = crypto::derive_room_key(&secret_b32).unwrap();

    let body = serde_json::json!({
        "args": ["true"],
        "source_user": "alice",
        "timeout_secs": 5,
    });
    let body_str = serde_json::to_string(&body).unwrap();
    let (env, session_key) =
        crypto::encrypt_v2(&room_key, &server_eph.public_b64(), body_str.as_bytes()).unwrap();
    let env_json = serde_json::to_string(&env).unwrap();
    let path = "/peer/v1/tasks";
    let auth = auth_header(&secret_b32, "POST", path, env_json.as_bytes());

    let url = format!("http://127.0.0.1:{port}{path}");
    let resp = ureq::post(&url)
        .set("Content-Type", ENCRYPTED_CONTENT_TYPE)
        .set("X-PartaGPU-AUTH", &auth)
        .send_string(&env_json)
        .expect("submit");
    assert_eq!(resp.status(), 200);
    let resp_body = resp.into_string().expect("read response body");
    let resp_env: Envelope = serde_json::from_str(&resp_body).expect("response is an envelope");
    let plain = crypto::decrypt(&session_key, &resp_env).expect("decrypt response");
    let parsed: serde_json::Value = serde_json::from_slice(&plain).expect("response JSON");
    assert!(parsed["task_id"].is_string());
    assert_eq!(parsed["accepted"], serde_json::Value::Bool(true));
}

#[test]
fn cancel_unknown_task_returns_404() {
    let _env = fresh_env();
    let (port, _eph, secret_b32) = start_test_server();
    let path = "/peer/v1/tasks/non-existent-id";
    // DELETE has no body, so the HMAC covers the empty byte slice.
    let auth = auth_header(&secret_b32, "DELETE", path, b"");
    let url = format!("http://127.0.0.1:{port}{path}");
    let resp = ureq::delete(&url).set("X-PartaGPU-AUTH", &auth).call();
    match resp {
        Err(ureq::Error::Status(404, _)) => {}
        Ok(r) => panic!("unexpected success {}", r.status()),
        Err(e) => panic!("expected 404, got {e}"),
    }
}

#[test]
fn rejects_request_without_auth_header() {
    let _env = fresh_env();
    let (port, server_eph, secret_b32) = start_test_server();
    let room_key = crypto::derive_room_key(&secret_b32).unwrap();
    let body = serde_json::json!({"args": ["true"]});
    let body_str = serde_json::to_string(&body).unwrap();
    let (env, _) =
        crypto::encrypt_v2(&room_key, &server_eph.public_b64(), body_str.as_bytes()).unwrap();
    let env_json = serde_json::to_string(&env).unwrap();
    let url = format!("http://127.0.0.1:{port}/peer/v1/tasks");
    let resp = ureq::post(&url)
        .set("Content-Type", ENCRYPTED_CONTENT_TYPE)
        .send_string(&env_json);
    match resp {
        Err(ureq::Error::Status(401, _)) => {}
        Ok(r) => panic!("unexpected success {}", r.status()),
        Err(e) => panic!("expected 401, got {e}"),
    }
}
