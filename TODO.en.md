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
