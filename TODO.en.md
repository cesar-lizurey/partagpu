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

<!-- Items addressed during the audit — kept here as a short history before
final cleanup. Technical details live in the commits / SECURITY.en.md. -->

### ✅ Concurrent connection cap on the peer API

Shipped in 1.10.0: `tokio::sync::Semaphore` of 64 permits acquired before `accept()`. Excess connections are absorbed by the kernel's TCP backlog.

### ✅ `pids.max` on cgroup

Shipped in 1.10.0: `pids` controller enabled in `subtree_control` + `pids.max=1024` on the parent cgroup (auto-inherited by per-task sub-cgroups).

### ✅ Anti-replay beyond the timestamp window

Shipped in 1.10.0: in-memory `ReplayCache` in `peer_api.rs`. After successful auth, the `X-PartaGPU-AUTH` header is inserted into the cache (60-s retention, 4096-entry cap, full clear on overflow). Byte-identical replays return 401.

### ✅ Strict CSP in Tauri

Shipped in 1.10.0: `tauri.conf.json` moves from `"csp": null` to a strict policy (`default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:; connect-src 'self' ipc: …; object-src 'none'; base-uri 'self'; frame-ancestors 'none'`).

### ✅ Documented trust boundary

Shipped in 1.10.0: dedicated section in `SECURITY.md` / `SECURITY.en.md` making explicit that "a verified peer can run arbitrary code in the target sandbox; that's expected; the defenses are isolation (sandbox + hardened account + cgroup), not command filtering".

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
