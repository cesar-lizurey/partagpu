🇫🇷 [Version française](TODO.md)

# TODO

Remaining work. Already-shipped measures are **not** listed here — they live in the documentation (`SECURITY.en.md`, `docs/ARCHITECTURE.en.md`, `docs/RELEASING.en.md`).

---

## 🔴 Security — priority items

From an internal threat-modeling pass ("skilled attacker on the LAN or local").

### `room.json` world-readable

- **Problem**: `~/.config/partagpu/room.json` contains `secret_base32` in plaintext. `fs::write` doesn't apply an explicit `chmod` — the file inherits the default umask (0644) → other local users can read it.
- **Impact**: on a multi-user machine, another user reads the full room secret and can submit arbitrary tasks to peers.
- **Fix**: `set_permissions(&path, 0o600)` after the `fs::write` in `auth.rs::save_room`.
- **Priority**: high (1-line fix, closes an obvious hole).

### CSRF / DNS rebinding on local API `127.0.0.1:7654`

- **Problem**: `http_api.rs` returns `Access-Control-Allow-Origin: *` and checks neither `Origin`, `Referer`, nor `Host`. Any web page open in the victim's browser can `fetch("http://127.0.0.1:7654/api/dispatch", ...)` and dispatch tasks on verified peers. DNS rebinding (`evil.com` → 127.0.0.1) bypasses even PNA on Firefox.
- **Impact**: arbitrary code execution on every verified peer from any browser tab open on the victim's machine.
- **Fix**: refuse any request without `Host: 127.0.0.1:7654` (or whose `Origin` is set and not the Tauri origin). Internal Tauri invocations don't send `Origin`; the Python client uses `requests` with `Host: 127.0.0.1:7654`.
- **Priority**: high.

### Offline brute-force of the passphrase via mDNS — mitigation shipped, redesign pending

- **Problem**: `crypto.rs::current_auth_proof` produces an HMAC truncated to 32 bits (8 hex chars), broadcast in clear in mDNS TXT records. A passive LAN attacker collects 2-3 windows and offline-bruteforces the 256^4 ≈ 4.3 G possible passphrases.
- **Phase 1 (shipped in 1.10.0)**: the `auth_key` derivation moved from HKDF (~1 µs/candidate) to PBKDF2-HMAC-SHA256 with 600 000 iterations (~100 ms/candidate). Brute-force now takes ~7 CPU-days = ~$1 500 of cloud instead of ~10 minutes on a laptop — infeasible for the "curious classmate" threat model.
- **Phase 2 (pending)**: drop `auth_proof` from the mDNS TXT entirely and move verification to a `/peer/v1/verify` route with a bidirectional HMAC challenge, rate-limited per source IP. Eliminates the leak instead of just making it expensive.
- **Priority**: medium (practical risk down by ~10⁵; remaining exposure no longer exploitable at the classroom threat model).

### No concurrent connection cap on the peer API

- **Problem**: `peer_api.rs` accepts connections with no cap, `MAX_REQUEST_BYTES = 32 MB` each. N concurrent connections → trivial OOM.
- **Fix**: `tokio::sync::Semaphore` with ~64 permits, `acquire().await` before `tokio::spawn`.
- **Priority**: medium.

### No `pids.max` on the cgroup

- **Problem**: `helper/src/main.rs::cmd_setup_cgroup` configures `cpu.max` and `memory.max` but not `pids.max`. A fork bomb in the sandbox can exhaust the system `pid_max`.
- **Fix**: `write_file(&format!("{CGROUP_PATH}/pids.max"), "256")` (or a higher number for DDP).
- **Priority**: medium.

### Anti-replay beyond the timestamp window

- **Problem**: the HMAC binds auth to the body, but `task_runner::incoming::create_and_run` doesn't dedupe seen `(ts, body_hash)` pairs. Within the 30-s window, a MITM can replay a captured request → duplicate task.
- **Fix**: bloom filter (or bounded `HashSet`) of `(ts, sha256(body))` over 60 s on the receiver side, reject duplicates.
- **Priority**: medium.

### CSP disabled in Tauri

- **Problem**: `tauri.conf.json` has `"csp": null`. React escapes by default, but defense-in-depth is lost if an HTML sink ever sneaks in.
- **Fix**: `"csp": "default-src 'self'; img-src 'self' data: https://raw.githubusercontent.com; style-src 'self' 'unsafe-inline'; font-src 'self' data:;"` (adjust for existing inline fonts/styles).
- **Priority**: low but easy.

### Default allowlist very permissive (document explicitly)

- **State**: `bash`, `sh`, `gcc`, `g++`, `make`, `cmake`, `cargo`, `rustc` are allowed. By design (ML tasks), but it means **a compromised peer = arbitrary code execution inside bwrap**. The defense becomes the sandbox + hardened partagpu account.
- **Fix**: no code fix — clarify the trust boundary in `SECURITY.en.md` ("a verified peer can run arbitrary code in the target sandbox; that's expected; the defenses are isolation, not command filtering").
- **Priority**: low (doc only).

---

## 🟢 Non-security improvements (low priority)

### Deeper integration tests

- **Missing**: current tests cover the peer side (receiving). Nothing exercises **end-to-end dispatch** (two instances actually talking to each other, one sending a task to the other).
- **Why it's hard**: would also need to fake the mDNS service (or bypass `Discovery`) so one instance can find the other.
- **Priority**: low.

### Extend the `thiserror` migration to the rest of the codebase

- **Current state**: `crypto.rs` uses a typed `CryptoError` enum (since 1.7.x). The rest of the codebase is still on `Result<T, String>`.
- **Why extend it**: would let HTTP handlers pattern-match on variants to map to more precise status codes (415 vs 401 vs 500), instead of grep-heuristic on the error message.
- **Cost**: ~100 sites to touch. Mechanical but tedious. Moderate risk of subtle regressions.
- **Benefit**: small in practice as long as no one pattern-matches on errors UI-side. Mostly design cleanup.
- **Tauri layer**: will stay on `Result<T, String>` (commands serialise errors to JS).
- **Priority**: low. Worth picking up when a typed-error consumer appears.

### Finer-grained re-keying

- **Current state**: the X25519 ephemeral key rotates every 10 minutes (see SECURITY.en.md).
- **Possible improvement**: also rotate after N processed requests (hard cap on the amount of traffic encrypted under any single key).
- **Benefit**: none in practice at current traffic volumes.
- **Priority**: zero for this project as it stands.
