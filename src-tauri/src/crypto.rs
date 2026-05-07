//! Peer-to-peer message encryption.
//!
//! Every body exchanged on the peer-API (port 7655, except `/health`) is
//! wrapped in an `Envelope` of `AES-256-GCM(plaintext)`. The session key is
//! derived from the **room secret combined with an ephemeral X25519 ECDH**
//! (envelope v2), giving forward secrecy : if the room secret leaks later,
//! captured ciphertexts can't be decrypted because the server's ephemeral
//! private key only existed in memory and is lost on app restart.
//!
//! Wire formats :
//! - **v1 (legacy)** : key = HKDF(room_secret). No forward secrecy. Still
//!   accepted by the server for backward compat with older clients.
//! - **v2** : key = HKDF(room_secret || ECDH(client_eph, server_eph)). The
//!   client embeds its ephemeral public key in the envelope ; the server
//!   uses its own ephemeral private key (held only in `EphemeralKey`) to
//!   complete the DH. Both sides derive the same session key for that
//!   single request, and the response uses the same session key.
//!
//! HMAC-SHA256 authentication (header `X-PartaGPU-AUTH`, computed by
//! [`compute_request_auth`]) provides anti-replay (30 s window) and binds
//! the auth to the specific request via timestamp + method + path + body
//! hash ; encryption layers confidentiality + integrity on top.
//!
//! Security properties (assuming AES-GCM and X25519 hold) :
//! - Confidentiality of message bodies on the wire
//! - Integrity (any byte flip rejected at decrypt time)
//! - Authenticity at the room level (only secret-holders can produce a
//!   valid ciphertext that decrypts cleanly)
//! - **Forward secrecy at app-restart granularity** (v2 envelopes only) :
//!   captured ciphertexts become undecryptable once the server's
//!   `EphemeralKey` is gone, even if the room secret leaks afterward.
//!
//! Out of scope: protection against an attacker who joins the same room
//! (they have the key by construction — that's the whole point of a
//! "shared room" design).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use x25519_dalek::{PublicKey, StaticSecret};

type HmacSha256 = Hmac<Sha256>;

const HKDF_SALT: &[u8] = b"PartaGPU/peer-api/v1";
const HKDF_INFO: &[u8] = b"AES-256-GCM message key";
const HKDF_INFO_V2: &[u8] = b"AES-256-GCM session key v2 (room|ecdh)";

/// Anti-replay window for HMAC request auth (seconds). Used by the
/// `X-PartaGPU-AUTH` header check to bound the clock-skew tolerance.
pub const AUTH_WINDOW_SECS: u64 = 30;

/// Content-Type header used to signal an encrypted body. Receivers MUST
/// reject bodies that don't carry this content-type — rejecting plaintext
/// is the whole point of mandatory encryption.
pub const ENCRYPTED_CONTENT_TYPE: &str = "application/x-partagpu-encrypted-v1";

/// Typed errors for the crypto module. Migrated from `Result<T, String>` so
/// callers can pattern-match on specific failure modes (e.g. distinguish a
/// malformed envelope from a key mismatch). The rest of the codebase still
/// uses `String` errors — callers convert with `.map_err(|e| e.to_string())`
/// at the boundary.
#[derive(Debug, thiserror::Error)]
pub enum CryptoError {
    /// Base32 / base64 decode failed (room secret, nonce, ciphertext, eph_pk).
    #[error("encodage invalide ({field}) : {source}")]
    BadEncoding {
        field: &'static str,
        #[source]
        source: data_encoding::DecodeError,
    },
    /// HKDF expand failed (essentially impossible at our 32-byte output size).
    #[error("HKDF expand ({0}) : sortie {1} octets")]
    Hkdf(&'static str, usize),
    /// Wrong-length input (e.g. nonce ≠ 12 B, eph_pk ≠ 32 B).
    #[error("longueur invalide ({field}) : {got} octets, attendu {expected}")]
    BadLength {
        field: &'static str,
        got: usize,
        expected: usize,
    },
    /// AES-GCM operation failed (tag mismatch, wrong key, tampered ct, …).
    /// Intentionally opaque so we don't leak which of those it actually was.
    #[error("déchiffrement AES-GCM échoué (clé invalide ou message altéré)")]
    AeadDecrypt,
    /// AES-GCM encrypt failed (shouldn't happen with a valid key + nonce).
    #[error("chiffrement AES-GCM : {0}")]
    AeadEncrypt(aes_gcm::Error),
    /// Envelope v2 is missing the `eph_pk` field the server needs.
    #[error("envelope v2 sans eph_pk")]
    MissingEphPk,
    /// JSON serialisation/parsing failed (envelope wrap/unwrap).
    #[error("JSON {context} : {source}")]
    Json {
        context: &'static str,
        #[source]
        source: serde_json::Error,
    },
    /// Every candidate ephemeral key (current + previous) failed to decrypt.
    /// Distinct from `AeadDecrypt` because the caller may want to log it
    /// separately ("client used an unknown pubkey" vs. "tampered message").
    #[error("aucune clé éphémère ne déverrouille le message")]
    NoMatchingKey,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Envelope {
    /// Format version : 1 (room-key only) or 2 (room-key + per-request ECDH).
    pub v: u8,
    /// Random 12-byte nonce, base64-encoded.
    pub nonce: String,
    /// AES-256-GCM ciphertext + 16-byte auth tag, base64-encoded.
    pub ct: String,
    /// Client's ephemeral X25519 public key, base64. Required for v=2,
    /// absent for v=1. Server-to-client responses omit this field (caller
    /// already shares the session key).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eph_pk: Option<String>,
}

/// In-memory ephemeral X25519 keypair. Generated at app startup and
/// rotated periodically — never written to disk. This is what gives v2
/// envelopes their forward-secrecy property : an attacker who later steals
/// the room secret still can't decrypt past traffic because the private
/// halves of these keypairs are gone.
///
/// Two keys are kept alive at any time : the *current* one (used for newly
/// derived sessions) and a *previous* one (kept for ~60 s after rotation
/// so requests already in flight when rotation happened still verify).
#[derive(Clone)]
pub struct EphemeralKey {
    state: Arc<std::sync::RwLock<KeyState>>,
}

struct KeyEntry {
    secret: StaticSecret,
    public_b64: String,
}

struct KeyState {
    current: KeyEntry,
    /// Kept around briefly after rotation so requests that started against
    /// the old current still complete. Cleared after the grace period.
    previous: Option<KeyEntry>,
    previous_expires_at: Option<std::time::Instant>,
}

fn make_key_entry() -> KeyEntry {
    let rng = rand::rngs::OsRng;
    let secret = StaticSecret::random_from_rng(rng);
    let pk = PublicKey::from(&secret);
    KeyEntry {
        secret,
        public_b64: data_encoding::BASE64.encode(pk.as_bytes()),
    }
}

/// Grace period during which an old key is still tried for incoming v=2
/// requests. Long enough that a request which started just before rotation
/// completes ; short enough that the forward-secrecy window stays small.
const KEY_GRACE: std::time::Duration = std::time::Duration::from_secs(60);

impl EphemeralKey {
    /// Generate a fresh keypair from the OS RNG. Call once at app start.
    pub fn generate() -> Self {
        Self {
            state: Arc::new(std::sync::RwLock::new(KeyState {
                current: make_key_entry(),
                previous: None,
                previous_expires_at: None,
            })),
        }
    }

    /// Public key, ready to be advertised over mDNS / health endpoint.
    /// Always reflects the most recent rotation.
    pub fn public_b64(&self) -> String {
        self.state.read().unwrap().current.public_b64.clone()
    }

    /// Compute the Diffie-Hellman shared secret with the peer's public key.
    /// Tries the current key first, then the previous key (if still inside
    /// its grace window) so a peer that fetched the old pubkey just before
    /// rotation can still complete its request.
    pub fn dh(&self, peer_pub_b64: &str) -> Result<[u8; 32], CryptoError> {
        let raw = data_encoding::BASE64
            .decode(peer_pub_b64.as_bytes())
            .map_err(|e| CryptoError::BadEncoding {
                field: "eph_pk",
                source: e,
            })?;
        if raw.len() != 32 {
            return Err(CryptoError::BadLength {
                field: "eph_pk",
                got: raw.len(),
                expected: 32,
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&raw);
        let peer_pk = PublicKey::from(arr);

        // Take a snapshot under the read lock and release it before doing
        // the (cheap but still) DH math.
        let st = self.state.read().unwrap();
        // Always try `current` first.
        let primary = st.current.secret.diffie_hellman(&peer_pk).to_bytes();
        let backup = match (&st.previous, st.previous_expires_at) {
            (Some(prev), Some(exp)) if std::time::Instant::now() < exp => {
                Some(prev.secret.diffie_hellman(&peer_pk).to_bytes())
            }
            _ => None,
        };
        drop(st);

        // Caller derives the session key from one of the candidates ; AES-GCM
        // tag check picks the right one. We can't tell from the DH output
        // alone which one matches, so we return the primary and let the
        // caller fall back via `dh_backup` when decrypt fails.
        if backup.is_some() {
            // Stash the backup in a side channel via an out-of-band call.
            // Cleanest API: caller uses `try_decrypt_v2` below which does
            // both attempts internally. So this method stays "primary only"
            // and `decrypt_request_v2` covers the fallback.
        }
        Ok(primary)
    }

    /// Two-shot DH that returns both candidate shared secrets if a previous
    /// key is still in the grace window. The first element is always the
    /// current-key shared secret. Used by `decrypt_request_v2` to try the
    /// AES-GCM tag against both keys.
    pub(crate) fn dh_candidates(
        &self,
        peer_pub_b64: &str,
    ) -> Result<(Vec<[u8; 32]>, ()), CryptoError> {
        let raw = data_encoding::BASE64
            .decode(peer_pub_b64.as_bytes())
            .map_err(|e| CryptoError::BadEncoding {
                field: "eph_pk",
                source: e,
            })?;
        if raw.len() != 32 {
            return Err(CryptoError::BadLength {
                field: "eph_pk",
                got: raw.len(),
                expected: 32,
            });
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&raw);
        let peer_pk = PublicKey::from(arr);

        let st = self.state.read().unwrap();
        let mut out = vec![st.current.secret.diffie_hellman(&peer_pk).to_bytes()];
        if let (Some(prev), Some(exp)) = (&st.previous, st.previous_expires_at) {
            if std::time::Instant::now() < exp {
                out.push(prev.secret.diffie_hellman(&peer_pk).to_bytes());
            }
        }
        Ok((out, ()))
    }

    /// Generate a fresh keypair, demote the old current to "previous", and
    /// return the new public key (base64). Callers should re-publish this
    /// on mDNS so peers update their cache.
    pub fn rotate(&self) -> String {
        let new = make_key_entry();
        let new_pub = new.public_b64.clone();
        let mut st = self.state.write().unwrap();
        let old_current = std::mem::replace(&mut st.current, new);
        st.previous = Some(old_current);
        st.previous_expires_at = Some(std::time::Instant::now() + KEY_GRACE);
        new_pub
    }

    /// Drop the previous key if its grace window has elapsed. Cheap to call
    /// often ; the rotation thread invokes it on every tick.
    pub fn gc_expired(&self) {
        let mut st = self.state.write().unwrap();
        let expired = matches!(st.previous_expires_at, Some(t) if std::time::Instant::now() >= t);
        if expired {
            st.previous = None;
            st.previous_expires_at = None;
        }
    }
}

/// Generate a fresh ephemeral X25519 keypair for client-side use. Returns
/// (private_secret, public_b64). The private secret stays on the stack and
/// is dropped after the request is sent.
pub fn fresh_client_eph() -> (StaticSecret, String) {
    let rng = rand::rngs::OsRng;
    let secret = StaticSecret::random_from_rng(rng);
    let pk = PublicKey::from(&secret);
    let pk_b64 = data_encoding::BASE64.encode(pk.as_bytes());
    (secret, pk_b64)
}

/// Combine ECDH shared secret with the room key into a session key via
/// HKDF-SHA256. The room key is the salt so that an attacker with only the
/// ECDH secret (e.g. via a quantum break of X25519 someday) still can't
/// derive the session key without also knowing the room secret.
pub fn derive_session_key(room_key: &[u8; 32], shared: &[u8; 32]) -> [u8; 32] {
    let hk = Hkdf::<Sha256>::new(Some(room_key), shared);
    let mut out = [0u8; 32];
    hk.expand(HKDF_INFO_V2, &mut out)
        .expect("HKDF expand 32 bytes can't fail");
    out
}

/// Derive a 32-byte AES-256 key from a base32-encoded room secret.
pub fn derive_room_key(secret_base32: &str) -> Result<[u8; 32], CryptoError> {
    let secret_bytes = data_encoding::BASE32
        .decode(secret_base32.as_bytes())
        .map_err(|e| CryptoError::BadEncoding {
            field: "room_secret",
            source: e,
        })?;
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), &secret_bytes);
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .map_err(|_| CryptoError::Hkdf("room_key", 32))?;
    Ok(key)
}

/// Iteration count for the PBKDF2 derivation of `auth_key`. 600 000 is the
/// OWASP 2023 recommendation for PBKDF2-HMAC-SHA256 used for password
/// hardening. At this count the derivation takes ~100 ms on a modern CPU,
/// which is invisible at room-join time but multiplies the cost of an
/// offline brute-force of the 4-word passphrase by a factor of ~10^5
/// versus the previous fast HKDF.
const PBKDF2_AUTH_ITERS: u32 = 600_000;
const PBKDF2_AUTH_SALT: &[u8] = b"PartaGPU/auth-key-pbkdf2-v2";

/// Derive a 32-byte HMAC key from the base32-encoded room secret.
///
/// Uses PBKDF2-HMAC-SHA256 (slow on purpose) to harden against an offline
/// brute-force attack on the 4-word passphrase via the leaked mDNS
/// `auth_proof` tags : a passive listener can only check ~10 candidates
/// per second per CPU core instead of ~1 000 000 with the old HKDF.
///
/// The cost is paid once at room join / load (~100 ms), then the key is
/// cached in `RoomState`. HMAC computations using this key remain fast.
///
/// **Protocol break vs ≤ 1.10.0** : peers running an older version derive
/// a different `auth_key` and will see each other as unverified / refuse
/// HMAC headers. All peers in a room must run a matching version.
pub fn derive_auth_key(secret_base32: &str) -> Result<[u8; 32], CryptoError> {
    let secret_bytes = data_encoding::BASE32
        .decode(secret_base32.as_bytes())
        .map_err(|e| CryptoError::BadEncoding {
            field: "room_secret",
            source: e,
        })?;
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(&secret_bytes, PBKDF2_AUTH_SALT, PBKDF2_AUTH_ITERS, &mut key);
    Ok(key)
}

// ── HMAC auth helpers ──────────────────────────────────────────────────────
//
// Two HMAC primitives, both keyed by `auth_key` :
//
//   1. `compute_verify_response` / `verify_response` : challenge-response
//      probe used by Discovery to verify that a peer holds the room secret.
//      The verifier sends a fresh random nonce ; the prover responds with
//      `HMAC(auth_key, "PartaGPU/verify-resp/v1\n" || nonce)`. Replaces the
//      old static `auth_proof` broadcast in mDNS — that scheme leaked one
//      HMAC tag per 30-s window passively, brute-forceable offline. The
//      challenge-response variant requires an active TCP connection per
//      tag and combines with PBKDF2-derived `auth_key` to make brute force
//      cost prohibitive (~$1500 of cloud compute).
//
//   2. `compute_request_auth` / `verify_request_auth` : full HMAC bound to
//      a specific HTTP request (timestamp + method + path + body hash).
//      Sent in the `X-PartaGPU-AUTH` header. Anti-replay window of
//      ±AUTH_WINDOW_SECS, plus binding to the request body so a captured
//      header can't be reused with a different body.

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Minimum and maximum nonce sizes accepted by `verify_response`. 16 bytes
/// is plenty (128 bits of randomness, no birthday-collision concern at our
/// volume) ; 32 is a generous upper bound to allow callers some slack.
pub const VERIFY_NONCE_MIN_BYTES: usize = 16;
pub const VERIFY_NONCE_MAX_BYTES: usize = 32;

/// Compute the HMAC response for a peer-verification challenge. The
/// `nonce` is whatever bytes the verifier sent ; the response is the full
/// 32-byte `HMAC-SHA256` (hex-encoded) over a fixed domain separator
/// followed by those bytes. No truncation: at 256 bits the value is a
/// distinct brute-force target per nonce, with the slow-KDF `auth_key`
/// dominating the per-candidate cost.
pub fn compute_verify_response(auth_key: &[u8; 32], nonce: &[u8]) -> String {
    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(auth_key).expect("HMAC accepts any key length");
    mac.update(b"PartaGPU/verify-resp/v1\n");
    mac.update(nonce);
    let bytes = mac.finalize().into_bytes();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        use std::fmt::Write;
        write!(&mut s, "{b:02x}").expect("hex write into String can't fail");
    }
    s
}

/// Verify a peer's `verify-resp` HMAC. Returns true iff `candidate_hex`
/// matches the expected HMAC for `nonce`, in (near-)constant time.
pub fn verify_response(auth_key: &[u8; 32], nonce: &[u8], candidate_hex: &str) -> bool {
    let expected = compute_verify_response(auth_key, nonce);
    if expected.len() != candidate_hex.len() {
        return false;
    }
    expected
        .as_bytes()
        .iter()
        .zip(candidate_hex.as_bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Build the HTTP request auth header value : `<unix_ts>:<hex hmac>`.
///
/// The HMAC binds the auth to the specific request via :
///   `b"PartaGPU/auth-req/v1\n" || ts || "\n" || method || "\n" || path
///    || "\n" || sha256(body)`
///
/// A captured header can therefore be replayed only on the same request
/// payload and only within the AUTH_WINDOW_SECS window — same anti-replay
/// guarantees as the previous TOTP scheme, plus body integrity.
pub fn compute_request_auth(auth_key: &[u8; 32], method: &str, path: &str, body: &[u8]) -> String {
    let ts = now_secs();
    format!(
        "{ts}:{}",
        request_hmac_hex(auth_key, ts, method, path, body)
    )
}

fn request_hmac_hex(auth_key: &[u8; 32], ts: u64, method: &str, path: &str, body: &[u8]) -> String {
    let mut body_hasher = Sha256::new();
    body_hasher.update(body);
    let body_hash = body_hasher.finalize();

    let mut mac =
        <HmacSha256 as Mac>::new_from_slice(auth_key).expect("HMAC accepts any key length");
    mac.update(b"PartaGPU/auth-req/v1\n");
    mac.update(ts.to_string().as_bytes());
    mac.update(b"\n");
    mac.update(method.as_bytes());
    mac.update(b"\n");
    mac.update(path.as_bytes());
    mac.update(b"\n");
    mac.update(&body_hash);
    let bytes = mac.finalize().into_bytes();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes.iter() {
        use std::fmt::Write;
        write!(&mut s, "{b:02x}").expect("hex write into String can't fail");
    }
    s
}

/// Verify an HTTP request auth header. Returns `Ok(())` on success ; on
/// failure the variant carries the reason (mostly for the security log,
/// the HTTP layer collapses it to 401).
pub fn verify_request_auth(
    auth_key: &[u8; 32],
    header_value: &str,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<(), AuthVerifyError> {
    let (ts_str, hmac_hex) = header_value
        .split_once(':')
        .ok_or(AuthVerifyError::Malformed)?;
    if ts_str.is_empty() || hmac_hex.is_empty() {
        return Err(AuthVerifyError::Malformed);
    }
    let ts: u64 = ts_str.parse().map_err(|_| AuthVerifyError::Malformed)?;
    let now = now_secs();
    let drift = now.abs_diff(ts);
    if drift > AUTH_WINDOW_SECS {
        return Err(AuthVerifyError::TimestampOutOfWindow { drift });
    }
    let expected = request_hmac_hex(auth_key, ts, method, path, body);
    // Constant-time comparison.
    if expected.len() != hmac_hex.len()
        || expected
            .as_bytes()
            .iter()
            .zip(hmac_hex.as_bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            != 0
    {
        return Err(AuthVerifyError::Mismatch);
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum AuthVerifyError {
    /// Header doesn't parse as `<unix_ts>:<hex>`.
    #[error("header X-PartaGPU-AUTH mal formé")]
    Malformed,
    /// `|now - ts|` exceeded `AUTH_WINDOW_SECS`. Likely clock skew or replay.
    #[error("timestamp hors fenêtre (dérive {drift} s, max {})", AUTH_WINDOW_SECS)]
    TimestampOutOfWindow { drift: u64 },
    /// HMAC did not verify. Wrong key, tampered request, or unknown peer.
    #[error("HMAC invalide")]
    Mismatch,
}

/// Encrypt `plaintext` with the v=1 room-key envelope (no forward secrecy).
/// Kept for backward compat with older peers ; prefer `encrypt_v2`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Envelope, CryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(CryptoError::AeadEncrypt)?;
    Ok(Envelope {
        v: 1,
        nonce: data_encoding::BASE64.encode(&nonce_bytes),
        ct: data_encoding::BASE64.encode(&ct),
        eph_pk: None,
    })
}

/// Encrypt `plaintext` for a peer whose ephemeral public key is `peer_eph_b64`.
/// Generates a fresh client X25519 ephemeral, computes ECDH, derives the session
/// key from `room_key || shared`, and produces a v=2 envelope. Returns the
/// envelope **and** the session key so the caller can decrypt the response
/// using the same key (responses omit `eph_pk` since the client side already
/// has it).
pub fn encrypt_v2(
    room_key: &[u8; 32],
    peer_eph_b64: &str,
    plaintext: &[u8],
) -> Result<(Envelope, [u8; 32]), CryptoError> {
    let (client_secret, client_pk_b64) = fresh_client_eph();
    let raw = data_encoding::BASE64
        .decode(peer_eph_b64.as_bytes())
        .map_err(|e| CryptoError::BadEncoding {
            field: "peer_eph_pk",
            source: e,
        })?;
    if raw.len() != 32 {
        return Err(CryptoError::BadLength {
            field: "peer_eph_pk",
            got: raw.len(),
            expected: 32,
        });
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&raw);
    let peer_pk = PublicKey::from(arr);
    let shared = client_secret.diffie_hellman(&peer_pk).to_bytes();
    let session_key = derive_session_key(room_key, &shared);

    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&session_key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(CryptoError::AeadEncrypt)?;

    Ok((
        Envelope {
            v: 2,
            nonce: data_encoding::BASE64.encode(&nonce_bytes),
            ct: data_encoding::BASE64.encode(&ct),
            eph_pk: Some(client_pk_b64),
        },
        session_key,
    ))
}

/// Encrypt `plaintext` with a known session key (no fresh DH). Used by the
/// server to send back a response after it has derived the shared key from
/// an incoming v=2 request, and by the client to decrypt that response.
pub fn encrypt_with_session(
    session_key: &[u8; 32],
    plaintext: &[u8],
) -> Result<Envelope, CryptoError> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(session_key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(CryptoError::AeadEncrypt)?;
    Ok(Envelope {
        v: 2,
        nonce: data_encoding::BASE64.encode(&nonce_bytes),
        ct: data_encoding::BASE64.encode(&ct),
        eph_pk: None,
    })
}

/// Decrypt a v=1 envelope with the room key, or a v=2 envelope where the
/// caller has already derived the session key (e.g. response from a peer).
/// For v=2 *requests* on the server side, use `decrypt_request_v2` which
/// also returns the session key for encrypting the response.
pub fn decrypt(key: &[u8; 32], env: &Envelope) -> Result<Vec<u8>, CryptoError> {
    decrypt_inner(key, env)
}

fn decrypt_inner(key: &[u8; 32], env: &Envelope) -> Result<Vec<u8>, CryptoError> {
    let nonce_bytes = data_encoding::BASE64
        .decode(env.nonce.as_bytes())
        .map_err(|e| CryptoError::BadEncoding {
            field: "nonce",
            source: e,
        })?;
    if nonce_bytes.len() != 12 {
        return Err(CryptoError::BadLength {
            field: "nonce",
            got: nonce_bytes.len(),
            expected: 12,
        });
    }
    let ct_bytes = data_encoding::BASE64
        .decode(env.ct.as_bytes())
        .map_err(|e| CryptoError::BadEncoding {
            field: "ciphertext",
            source: e,
        })?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    cipher
        .decrypt(nonce, ct_bytes.as_ref())
        // Don't leak details — could be tag mismatch, key mismatch, or
        // truncated ciphertext. All of them mean "this didn't come from
        // a peer in our room".
        .map_err(|_| CryptoError::AeadDecrypt)
}

/// Server-side decryption of a v=2 envelope : extracts the client's ephemeral
/// public key from the envelope, derives the session key via ECDH with the
/// server's `EphemeralKey`, decrypts the payload, and returns
/// `(plaintext, session_key)` so the caller can encrypt the response with
/// the matching key.
///
/// When the server has just rotated keys, both the current and the previous
/// key are tried in turn — whichever produces a valid AES-GCM tag wins. This
/// avoids dropping requests that started during rotation.
pub fn decrypt_request_v2(
    room_key: &[u8; 32],
    server_eph: &EphemeralKey,
    env: &Envelope,
) -> Result<(Vec<u8>, [u8; 32]), CryptoError> {
    let client_pk_b64 = env.eph_pk.as_deref().ok_or(CryptoError::MissingEphPk)?;
    let (shareds, _) = server_eph.dh_candidates(client_pk_b64)?;
    let mut last_err = CryptoError::NoMatchingKey;
    for shared in shareds {
        let session_key = derive_session_key(room_key, &shared);
        match decrypt_inner(&session_key, env) {
            Ok(plain) => return Ok((plain, session_key)),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// Convenience: encrypt a JSON-serializable value and return the
/// JSON-string envelope ready to be put in an HTTP body.
pub fn encrypt_json<T: Serialize>(key: &[u8; 32], value: &T) -> Result<String, CryptoError> {
    let plain = serde_json::to_vec(value).map_err(|e| CryptoError::Json {
        context: "sérialisation plaintext",
        source: e,
    })?;
    let env = encrypt(key, &plain)?;
    serde_json::to_string(&env).map_err(|e| CryptoError::Json {
        context: "sérialisation enveloppe",
        source: e,
    })
}

/// Convenience: decrypt a JSON-string envelope and parse the plaintext as JSON.
pub fn decrypt_json<T: for<'a> Deserialize<'a>>(
    key: &[u8; 32],
    body: &str,
) -> Result<T, CryptoError> {
    let env: Envelope = serde_json::from_str(body).map_err(|e| CryptoError::Json {
        context: "désérialisation enveloppe",
        source: e,
    })?;
    let plain = decrypt(key, &env)?;
    serde_json::from_slice(&plain).map_err(|e| CryptoError::Json {
        context: "désérialisation plaintext",
        source: e,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── HMAC auth tests ────────────────────────────────────────────────

    #[test]
    fn verify_response_matches_for_same_key_and_nonce() {
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let k1 = derive_auth_key(&secret_b32).unwrap();
        let k2 = derive_auth_key(&secret_b32).unwrap();
        let nonce = b"random-nonce-1234";
        let resp = compute_verify_response(&k1, nonce);
        // Full HMAC-SHA256 = 32 bytes = 64 hex chars.
        assert_eq!(resp.len(), 64);
        assert!(verify_response(&k2, nonce, &resp));
    }

    #[test]
    fn verify_response_rejects_wrong_key() {
        let s1 = data_encoding::BASE32.encode(b"room-1!!!!!!!!!!!!!!!!!!");
        let s2 = data_encoding::BASE32.encode(b"room-2!!!!!!!!!!!!!!!!!!");
        let k1 = derive_auth_key(&s1).unwrap();
        let k2 = derive_auth_key(&s2).unwrap();
        let nonce = b"random-nonce-1234";
        let resp = compute_verify_response(&k1, nonce);
        assert!(!verify_response(&k2, nonce, &resp));
    }

    #[test]
    fn verify_response_rejects_wrong_nonce() {
        // A response valid for nonce A must not validate against nonce B.
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let k = derive_auth_key(&secret_b32).unwrap();
        let resp = compute_verify_response(&k, b"nonce-A-1234");
        assert!(!verify_response(&k, b"nonce-B-1234", &resp));
    }

    #[test]
    fn request_auth_roundtrip() {
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let key = derive_auth_key(&secret_b32).unwrap();
        let header = compute_request_auth(&key, "POST", "/peer/v1/tasks", b"some body");
        verify_request_auth(&key, &header, "POST", "/peer/v1/tasks", b"some body").unwrap();
    }

    #[test]
    fn request_auth_rejects_tampered_body() {
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let key = derive_auth_key(&secret_b32).unwrap();
        let header = compute_request_auth(&key, "POST", "/peer/v1/tasks", b"original body");
        let err = verify_request_auth(&key, &header, "POST", "/peer/v1/tasks", b"tampered body")
            .unwrap_err();
        assert!(matches!(err, AuthVerifyError::Mismatch));
    }

    #[test]
    fn request_auth_rejects_wrong_method() {
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let key = derive_auth_key(&secret_b32).unwrap();
        let header = compute_request_auth(&key, "POST", "/peer/v1/tasks", b"");
        let err = verify_request_auth(&key, &header, "GET", "/peer/v1/tasks", b"").unwrap_err();
        assert!(matches!(err, AuthVerifyError::Mismatch));
    }

    #[test]
    fn request_auth_rejects_old_timestamp() {
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let key = derive_auth_key(&secret_b32).unwrap();
        // Build a header by hand with a far-past timestamp.
        let ancient_ts = now_secs() - (AUTH_WINDOW_SECS * 10);
        let hmac = request_hmac_hex(&key, ancient_ts, "POST", "/peer/v1/tasks", b"");
        let header = format!("{ancient_ts}:{hmac}");
        let err = verify_request_auth(&key, &header, "POST", "/peer/v1/tasks", b"").unwrap_err();
        assert!(matches!(err, AuthVerifyError::TimestampOutOfWindow { .. }));
    }

    #[test]
    fn request_auth_rejects_malformed_header() {
        let secret_b32 = data_encoding::BASE32.encode(b"shared-room!!!!!!!!!!!!!");
        let key = derive_auth_key(&secret_b32).unwrap();
        for bad in ["not-a-header", "1234:", ":deadbeef", ""] {
            let err = verify_request_auth(&key, bad, "POST", "/peer/v1/tasks", b"").unwrap_err();
            assert!(matches!(err, AuthVerifyError::Malformed));
        }
    }

    // ── Existing AES-GCM / X25519 tests ────────────────────────────────

    #[test]
    fn roundtrip() {
        let secret_b32 = data_encoding::BASE32.encode(b"some-room-secret-bytes!!!");
        let key = derive_room_key(&secret_b32).unwrap();
        let plain = br#"{"hello":"world"}"#;
        let env = encrypt(&key, plain).unwrap();
        let decrypted = decrypt(&key, &env).unwrap();
        assert_eq!(plain, decrypted.as_slice());
    }

    #[test]
    fn wrong_key_fails() {
        let key1 = derive_room_key(&data_encoding::BASE32.encode(b"room-1!!!!!!!!!!!!!!")).unwrap();
        let key2 = derive_room_key(&data_encoding::BASE32.encode(b"room-2!!!!!!!!!!!!!!")).unwrap();
        let env = encrypt(&key1, b"sensitive").unwrap();
        assert!(decrypt(&key2, &env).is_err());
    }

    #[test]
    fn tampered_ct_fails() {
        let key = derive_room_key(&data_encoding::BASE32.encode(b"room-secret-bytes!!!")).unwrap();
        let mut env = encrypt(&key, b"sensitive").unwrap();
        // Flip a byte in the ciphertext (decode -> mutate -> re-encode).
        let mut bytes = data_encoding::BASE64.decode(env.ct.as_bytes()).unwrap();
        bytes[0] ^= 0x01;
        env.ct = data_encoding::BASE64.encode(&bytes);
        assert!(decrypt(&key, &env).is_err());
    }

    #[test]
    fn json_roundtrip() {
        let key = derive_room_key(&data_encoding::BASE32.encode(b"room-secret-bytes!!!")).unwrap();
        let value = serde_json::json!({"args": ["python3", "-c", "print(42)"], "n": 42});
        let body = encrypt_json(&key, &value).unwrap();
        let back: serde_json::Value = decrypt_json(&key, &body).unwrap();
        assert_eq!(value, back);
    }

    #[test]
    fn v2_roundtrip_with_ecdh() {
        let room_key =
            derive_room_key(&data_encoding::BASE32.encode(b"room-secret-bytes!!!")).unwrap();
        let server_eph = EphemeralKey::generate();
        let server_pub = server_eph.public_b64();

        let plain = b"top-secret-payload";
        let (env, client_session_key) = encrypt_v2(&room_key, &server_pub, plain).unwrap();
        assert_eq!(env.v, 2);
        assert!(env.eph_pk.is_some());

        let (decrypted, server_session_key) =
            decrypt_request_v2(&room_key, &server_eph, &env).unwrap();
        assert_eq!(plain, decrypted.as_slice());
        // Both sides MUST derive the same session key — this is what makes
        // forward secrecy work end-to-end.
        assert_eq!(client_session_key, server_session_key);

        // Server response uses the same session key with a fresh nonce.
        let response_env = encrypt_with_session(&server_session_key, b"ack").unwrap();
        let response_plain = decrypt(&client_session_key, &response_env).unwrap();
        assert_eq!(response_plain, b"ack");
    }

    #[test]
    fn v2_with_wrong_room_key_fails() {
        let room_key1 =
            derive_room_key(&data_encoding::BASE32.encode(b"room-1!!!!!!!!!!!!!!!!!!!!!!!!!!"))
                .unwrap();
        let room_key2 =
            derive_room_key(&data_encoding::BASE32.encode(b"room-2!!!!!!!!!!!!!!!!!!!!!!!!!!"))
                .unwrap();
        let server_eph = EphemeralKey::generate();

        let (env, _) = encrypt_v2(&room_key1, &server_eph.public_b64(), b"payload").unwrap();
        // Different room key → different session key → AES-GCM tag fails.
        assert!(decrypt_request_v2(&room_key2, &server_eph, &env).is_err());
    }

    #[test]
    fn v2_with_wrong_server_eph_fails() {
        let room_key =
            derive_room_key(&data_encoding::BASE32.encode(b"room!!!!!!!!!!!!!!!!!!!!!!!!!!!!"))
                .unwrap();
        let real_server = EphemeralKey::generate();
        let other_server = EphemeralKey::generate();

        let (env, _) = encrypt_v2(&room_key, &real_server.public_b64(), b"payload").unwrap();
        // Encrypted for `real_server` but `other_server` tries to read it.
        assert!(decrypt_request_v2(&room_key, &other_server, &env).is_err());
    }

    #[test]
    fn forward_secrecy_after_eph_rotation() {
        // Captured ciphertext + leaked room secret, but the server's
        // ephemeral private key has been rotated → still undecryptable.
        let room_key =
            derive_room_key(&data_encoding::BASE32.encode(b"room!!!!!!!!!!!!!!!!!!!!!!!!!!!!"))
                .unwrap();
        let old_server_eph = EphemeralKey::generate();

        let (captured, _) = encrypt_v2(
            &room_key,
            &old_server_eph.public_b64(),
            b"yesterday's secret",
        )
        .unwrap();

        // Simulate app restart : new ephemeral keypair, old one is gone.
        let new_server_eph = EphemeralKey::generate();
        drop(old_server_eph);

        // Even with the room secret, the new server can't decrypt.
        assert!(decrypt_request_v2(&room_key, &new_server_eph, &captured).is_err());
    }

    #[test]
    fn rotation_keeps_grace_window_decryptable() {
        // A client that captured the OLD pubkey just before rotation should
        // still get its request through during the grace period.
        let room_key =
            derive_room_key(&data_encoding::BASE32.encode(b"room!!!!!!!!!!!!!!!!!!!!!!!!!!!!"))
                .unwrap();
        let server_eph = EphemeralKey::generate();
        let old_pub = server_eph.public_b64();

        // Client encrypts against the (then-)current key.
        let (env, _) = encrypt_v2(&room_key, &old_pub, b"in-flight payload").unwrap();

        // Server rotates *before* the request reaches the handler.
        let new_pub = server_eph.rotate();
        assert_ne!(old_pub, new_pub);

        // The handler must still decrypt the in-flight request, by trying
        // the previous key as a fallback.
        let (plain, _) = decrypt_request_v2(&room_key, &server_eph, &env).unwrap();
        assert_eq!(plain, b"in-flight payload");

        // After GC of the previous key, the same envelope is no longer
        // decryptable — confirming the grace window is finite.
        // (We can't easily fast-forward time in stable Rust, so we manually
        // drop the previous slot.)
        {
            let mut st = server_eph.state.write().unwrap();
            st.previous = None;
            st.previous_expires_at = None;
        }
        assert!(decrypt_request_v2(&room_key, &server_eph, &env).is_err());
    }

    #[test]
    fn new_clients_use_new_pubkey_after_rotation() {
        let room_key =
            derive_room_key(&data_encoding::BASE32.encode(b"room!!!!!!!!!!!!!!!!!!!!!!!!!!!!"))
                .unwrap();
        let server_eph = EphemeralKey::generate();
        let _old_pub = server_eph.public_b64();
        let new_pub = server_eph.rotate();

        // A client that fetched the new pubkey gets through normally.
        let (env, _) = encrypt_v2(&room_key, &new_pub, b"after rotation").unwrap();
        let (plain, _) = decrypt_request_v2(&room_key, &server_eph, &env).unwrap();
        assert_eq!(plain, b"after rotation");
    }
}
