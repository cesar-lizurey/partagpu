🇫🇷 [Version française](TODO.md)

# TODO

No work in progress. Already-shipped measures live in the documentation (`SECURITY.en.md`, `docs/ARCHITECTURE.en.md`, `docs/RELEASING.en.md`) — not here.

---

## ✅ Security — shipped in 1.10.0

Internal threat-modeling audit ("skilled attacker on the LAN or local") → 9 items, all fixed. Details in `SECURITY.md` / `SECURITY.en.md` / `docs/ARCHITECTURE.md`.

| # | Item | Commit |
|---|------|--------|
| 1 | `room.json` `chmod 600` | `8fa7c33` |
| 2 | Origin/Host gate on `127.0.0.1:7654` (anti CSRF + DNS rebinding) | `e6cc705` |
| 3 | Slow KDF (PBKDF2, 600 k iters) on `derive_auth_key` — phase 1 of the mDNS fix | `26a4c35` |
| 4 | `Semaphore(64)` of concurrent connections on the peer API | `44278d1` |
| 5 | `pids.max=1024` + `pids` controller enabled on cgroup | `33bfb6f` |
| 6 | `ReplayCache` (60 s) against replay of a captured request | `f4d69ed` |
| 7 | Strict CSP in `tauri.conf.json` | `73acfed` |
| 8 | Trust boundary made explicit (verified peer = arbitrary code in the target sandbox, by design) | `dd8b8df` |
| 2-bis | Drop `auth_proof` from mDNS + active `/peer/v1/verify` endpoint (challenge-response HMAC) — phase 2 of the mDNS fix | `3b30761` |

## ✅ Integration tests — shipped in 1.10.0

Two new e2e tests that spawn two real peer-API instances on different ports, sharing the same room secret:
- `two_instances_verify_each_other` — each peer correctly answers the other's `/peer/v1/verify` challenge
- `two_instances_dispatch_end_to_end` — A encrypts a task, signs the HMAC, sends to B; B accepts; A decrypts the response

Total e2e tests go from 9 to 11.
