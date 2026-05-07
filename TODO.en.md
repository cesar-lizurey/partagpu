🇫🇷 [Version française](TODO.md)

# TODO

Remaining work. Already-shipped measures are **not** listed here — they live in the documentation (`SECURITY.en.md`, `docs/ARCHITECTURE.en.md`, `docs/RELEASING.en.md`).

---

## 🔴 Security — remaining item

### Offline brute-force of the passphrase via mDNS — phase 2

- **Phase 1 (shipped in 1.10.0)**: `derive_auth_key` moved from HKDF (~1 µs/candidate) to PBKDF2-HMAC-SHA256 with 600 000 iterations (~100 ms/candidate). Passive `auth_proof` brute-force went from ~10 min on a laptop to ~7 CPU-days = ~$1 500 of cloud.
- **Phase 2 (pending)**: drop `auth_proof` from the mDNS TXT entirely and move verification to a `/peer/v1/verify` route that requires a bidirectional HMAC challenge, rate-limited per source IP. Eliminates the passive leak instead of just making it expensive.
- **Cost**: ~3-4 h of work (new route + discovery refactor for async probe + UI state defaulting to `verified=false` + tests).
- **Priority**: medium (residual exposure after phase 1 is no longer exploitable at the classroom threat model).

---

## ✅ Security — shipped in 1.10.0

Short list with commits, kept as history. Details in SECURITY.md / SECURITY.en.md.

| # | Item | Commit |
|---|------|--------|
| 1 | `room.json` `chmod 600` (default umask 0644 was letting other local users read the secret) | `8fa7c33` |
| 2 | Origin/Host gate on `127.0.0.1:7654` against CSRF + DNS rebinding | `e6cc705` |
| 3 | Slow KDF (PBKDF2, 600 k iters) on `auth_key` derivation | `26a4c35` |
| 4 | `Semaphore(64)` of concurrent connections on the peer API | `44278d1` |
| 5 | `pids.max=1024` + `pids` controller enabled on cgroup | `33bfb6f` |
| 6 | `ReplayCache` (60 s) against replay of a captured request | `f4d69ed` |
| 7 | Strict CSP in `tauri.conf.json` | `73acfed` |
| 8 | Trust boundary made explicit (verified peer = arbitrary code in the target sandbox, by design) | `dd8b8df` |

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
