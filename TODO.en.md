🇫🇷 [Version française](TODO.md)

# TODO — Security

Remaining security work. Already-shipped measures are documented in [SECURITY.en.md](SECURITY.en.md).

## Done

- ✅ **Peer-to-peer encryption** (since 1.6.0). AES-256-GCM with an HKDF-SHA256-derived key from the room secret. See [ARCHITECTURE.en.md → Peer-to-peer message encryption](docs/ARCHITECTURE.en.md#peer-to-peer-message-encryption).
- ✅ **Forward secrecy** (since 1.7.0). Per-request ephemeral X25519 Diffie-Hellman exchange (envelope v=2). The server's ephemeral key lives in RAM only, is regenerated at every app start **and rotated every 10 minutes**; the previous key stays valid for ~60 s to absorb in-flight requests.
- ✅ **Per-task cgroup isolation** (since 1.6.0). Each task gets its own `/sys/fs/cgroup/partagpu/task-<uuid>` so one runaway task can't OOM its neighbors.
- ✅ **Concurrent-task cap** (since 1.6.0). Configurable through the UI; beyond the cap, tasks stay in a FIFO queue.
- ✅ **End-to-end peer-API integration tests** (since 1.7.0). 5 tests in `src-tauri/tests/peer_api_e2e.rs` that spin up a real server on `127.0.0.1:0` and check: plaintext refusal, refusal without TOTP, refusal with a wrong room secret, full v=2 round-trip, 404 on unknown task cancel.

## Still to do

No critical work left. Anything below is optional polish.

### Deeper integration tests
- **Missing**: current tests cover the peer side (receiving). Nothing exercises **dispatch end-to-end** (two instances actually talking to each other, one sending a task to the other).
- **Why it's hard**: would also need to fake the mDNS service (or bypass `Discovery`) so one instance can find the other.
- **Priority**: low.

### Finer-grained re-keying
- **Current state**: ephemeral key rotates every 10 minutes. Enough for the classroom threat model.
- **Possible improvement**: rotate after N processed requests (hard cap on the amount of traffic encrypted under any single key). No practical benefit at current traffic volumes.
- **Priority**: zero for this project as it stands.

### Extend the `thiserror` migration to the rest of the codebase
- **Current state**: since 1.7.x, `crypto.rs` uses a typed `CryptoError` enum with variants (`BadEncoding`, `BadLength`, `AeadDecrypt`, `MissingEphPk`, `Json`, `NoMatchingKey`…). Callers (peer_api, http_api) still bridge via `.map_err(|e| e.to_string())` at the boundary because the rest of the codebase is on `Result<T, String>`.
- **Why extend it**: would let HTTP handlers pattern-match on variants to map to more precise status codes (415 vs 401 vs 500), instead of grep-heuristic on the error message. Would also let us drop the `format!()` calls that inflate errors into lossy strings.
- **Cost**: large. ~100 sites across every Rust file (sandbox, task_runner, discovery, auth, http_api, peer_api). Mechanical but tedious. Moderate risk of subtle regressions (unit tests cover few error paths).
- **Benefit**: small in practice as long as no one pattern-matches on errors UI-side. Mostly design cleanup.
- **Tauri layer**: will stay on `Result<T, String>` (commands serialise errors to JS) — internal migration only touches pure Rust code.
- **Priority**: low. Worth picking up when a typed-error consumer arrives (e.g. a `/peer/v1/error-summary` endpoint with stable codes, or tests asserting on a specific variant).

### Drop TOTP in favor of an HMAC + timestamp scheme
- **Current state**: peer-to-peer request auth relies on a 6-digit TOTP code in the `X-PartaGPU-TOTP` header, plus the same code broadcast in mDNS TXT records for passive peer verification.
- **Why replace it**: since AES-256-GCM (1.6.0) and X25519 forward secrecy (1.7.0) shipped, TOTP only does anti-replay over a ~30 s window. A more standard scheme buys exactly the same:
  - HTTP: header `X-PartaGPU-AUTH: HMAC-SHA256(room_key, timestamp || body_hash)` + server check `|now - timestamp| < 30 s` → same anti-replay guarantees, more readable, no `totp-rs` / `base32` dependency.
  - mDNS: broadcast a truncated `HMAC-SHA256(room_key, current_time_window)` (which **is** TOTP mathematically but without the RFC 6238 formalism) or a lightweight HTTP verification challenge.
- **Cost**: peer-to-peer protocol break, so forces a simultaneous upgrade. Probably ~1 day of work including tests + doc migration.
- **Benefit**: smaller dependency surface (`totp-rs`, `base32`, `data-encoding`), more readable code (explicit HMAC instead of a disguised HMAC inside TOTP), fewer overlapping layers doing the same thing.
- **Priority**: low. The system works ; removing TOTP buys nothing on the security front — it's purely design cleanup.
