🇫🇷 [Version française](TODO.md)

# TODO

Remaining work. Already-shipped measures are **not** listed here — they live in the documentation (`SECURITY.en.md`, `docs/ARCHITECTURE.en.md`, `docs/RELEASING.en.md`).

No critical work left. Everything below is optional, ordered by decreasing value.

## Deeper integration tests

- **Missing**: current tests cover the peer side (receiving). Nothing exercises **end-to-end dispatch** (two instances actually talking to each other, one sending a task to the other).
- **Why it's hard**: would also need to fake the mDNS service (or bypass `Discovery`) so one instance can find the other.
- **Priority**: low.

## Extend the `thiserror` migration to the rest of the codebase

- **Current state**: `crypto.rs` uses a typed `CryptoError` enum (since 1.7.x). The rest of the codebase is still on `Result<T, String>`.
- **Why extend it**: would let HTTP handlers pattern-match on variants to map to more precise status codes (415 vs 401 vs 500), instead of grep-heuristic on the error message.
- **Cost**: ~100 sites to touch. Mechanical but tedious. Moderate risk of subtle regressions.
- **Benefit**: small in practice as long as no one pattern-matches on errors UI-side. Mostly design cleanup.
- **Tauri layer**: will stay on `Result<T, String>` (commands serialise errors to JS).
- **Priority**: low. Worth picking up when a typed-error consumer appears.

## Finer-grained re-keying

- **Current state**: the X25519 ephemeral key rotates every 10 minutes (see SECURITY.en.md).
- **Possible improvement**: also rotate after N processed requests (hard cap on the amount of traffic encrypted under any single key).
- **Benefit**: none in practice at current traffic volumes.
- **Priority**: zero for this project as it stands.
