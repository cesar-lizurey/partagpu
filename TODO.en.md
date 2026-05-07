🇫🇷 [Version française](TODO.md)

# TODO

Remaining work. Already-shipped measures are **not** listed here — they live in the documentation (`SECURITY.en.md`, `docs/ARCHITECTURE.en.md`, `docs/RELEASING.en.md`).

---

## ✅ Security — all items from the internal audit shipped in 1.10.0

Short list with commits, kept as history. Details in SECURITY.md / SECURITY.en.md / docs/ARCHITECTURE.md.

| # | Item | Commit |
|---|------|--------|
| 1 | `room.json` `chmod 600` (default umask 0644 was letting other local users read the secret) | `8fa7c33` |
| 2 | Origin/Host gate on `127.0.0.1:7654` against CSRF + DNS rebinding | `e6cc705` |
| 3 | Slow KDF (PBKDF2, 600 k iters) on `auth_key` derivation — phase 1 of the mDNS fix | `26a4c35` |
| 4 | `Semaphore(64)` of concurrent connections on the peer API | `44278d1` |
| 5 | `pids.max=1024` + `pids` controller enabled on cgroup | `33bfb6f` |
| 6 | `ReplayCache` (60 s) against replay of a captured request | `f4d69ed` |
| 7 | Strict CSP in `tauri.conf.json` | `73acfed` |
| 8 | Trust boundary made explicit (verified peer = arbitrary code in the target sandbox, by design) | `dd8b8df` |
| 2-bis | **Drop `auth_proof` from mDNS** + active `/peer/v1/verify` endpoint (challenge-response HMAC) — phase 2 of the mDNS fix, eliminates the passive leak entirely instead of just making it expensive | (this commit) |

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
