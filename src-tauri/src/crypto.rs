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
//! TOTP authentication (header `X-PartaGPU-TOTP`) stays as before and
//! provides anti-replay; encryption layers confidentiality + integrity on
//! top.
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
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::sync::Arc;
use x25519_dalek::{PublicKey, StaticSecret};

const HKDF_SALT: &[u8] = b"PartaGPU/peer-api/v1";
const HKDF_INFO: &[u8] = b"AES-256-GCM message key";
const HKDF_INFO_V2: &[u8] = b"AES-256-GCM session key v2 (room|ecdh)";

/// Content-Type header used to signal an encrypted body. Receivers MUST
/// reject bodies that don't carry this content-type — rejecting plaintext
/// is the whole point of mandatory encryption.
pub const ENCRYPTED_CONTENT_TYPE: &str = "application/x-partagpu-encrypted-v1";

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

/// In-memory ephemeral X25519 keypair. Generated once at app startup and
/// dropped on shutdown — never written to disk. This is what gives v2
/// envelopes their forward-secrecy property : an attacker who later steals
/// the room secret still can't decrypt past traffic because the secret
/// half of this keypair is gone.
#[derive(Clone)]
pub struct EphemeralKey {
    inner: Arc<StaticSecret>,
}

impl EphemeralKey {
    /// Generate a fresh keypair from the OS RNG. Call once at app start.
    pub fn generate() -> Self {
        let mut rng = rand::rngs::OsRng;
        Self {
            inner: Arc::new(StaticSecret::random_from_rng(&mut rng)),
        }
    }

    /// Public key, ready to be advertised over mDNS / health endpoint.
    pub fn public_b64(&self) -> String {
        let pk = PublicKey::from(self.inner.as_ref());
        data_encoding::BASE64.encode(pk.as_bytes())
    }

    /// Compute the Diffie-Hellman shared secret with the peer's public key.
    pub fn dh(&self, peer_pub_b64: &str) -> Result<[u8; 32], String> {
        let raw = data_encoding::BASE64
            .decode(peer_pub_b64.as_bytes())
            .map_err(|e| format!("eph_pk base64 invalide : {e}"))?;
        if raw.len() != 32 {
            return Err(format!("eph_pk doit faire 32 octets, recu {}", raw.len()));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&raw);
        let pk = PublicKey::from(arr);
        Ok(self.inner.diffie_hellman(&pk).to_bytes())
    }
}

/// Generate a fresh ephemeral X25519 keypair for client-side use. Returns
/// (private_secret, public_b64). The private secret stays on the stack and
/// is dropped after the request is sent.
pub fn fresh_client_eph() -> (StaticSecret, String) {
    let mut rng = rand::rngs::OsRng;
    let secret = StaticSecret::random_from_rng(&mut rng);
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
pub fn derive_room_key(secret_base32: &str) -> Result<[u8; 32], String> {
    let secret_bytes = data_encoding::BASE32
        .decode(secret_base32.as_bytes())
        .map_err(|e| format!("secret salle invalide (base32) : {e}"))?;
    let hk = Hkdf::<Sha256>::new(Some(HKDF_SALT), &secret_bytes);
    let mut key = [0u8; 32];
    hk.expand(HKDF_INFO, &mut key)
        .map_err(|e| format!("HKDF expand : {e}"))?;
    Ok(key)
}

/// Encrypt `plaintext` with the v=1 room-key envelope (no forward secrecy).
/// Kept for backward compat with older peers ; prefer `encrypt_v2`.
pub fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Envelope, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("chiffrement AES-GCM : {e}"))?;
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
) -> Result<(Envelope, [u8; 32]), String> {
    let (client_secret, client_pk_b64) = fresh_client_eph();
    let raw = data_encoding::BASE64
        .decode(peer_eph_b64.as_bytes())
        .map_err(|e| format!("peer_eph_pk base64 invalide : {e}"))?;
    if raw.len() != 32 {
        return Err(format!(
            "peer_eph_pk doit faire 32 octets, recu {}",
            raw.len()
        ));
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
        .map_err(|e| format!("chiffrement AES-GCM : {e}"))?;

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
) -> Result<Envelope, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(session_key));
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ct = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("chiffrement AES-GCM : {e}"))?;
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
pub fn decrypt(key: &[u8; 32], env: &Envelope) -> Result<Vec<u8>, String> {
    decrypt_inner(key, env)
}

fn decrypt_inner(key: &[u8; 32], env: &Envelope) -> Result<Vec<u8>, String> {
    let nonce_bytes = data_encoding::BASE64
        .decode(env.nonce.as_bytes())
        .map_err(|e| format!("nonce base64 : {e}"))?;
    if nonce_bytes.len() != 12 {
        return Err(format!(
            "nonce doit faire 12 octets, reçu {}",
            nonce_bytes.len()
        ));
    }
    let ct_bytes = data_encoding::BASE64
        .decode(env.ct.as_bytes())
        .map_err(|e| format!("ciphertext base64 : {e}"))?;
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    cipher
        .decrypt(nonce, ct_bytes.as_ref())
        .map_err(|_| {
            // Don't leak details — could be tag mismatch, key mismatch, or
            // truncated ciphertext. All of them mean "this didn't come from
            // a peer in our room".
            "déchiffrement AES-GCM échoué (clé invalide ou message altéré)"
                .to_string()
        })
}

/// Server-side decryption of a v=2 envelope : extracts the client's ephemeral
/// public key from the envelope, derives the session key via ECDH with the
/// server's `EphemeralKey`, decrypts the payload, and returns
/// `(plaintext, session_key)` so the caller can encrypt the response with
/// the matching key.
pub fn decrypt_request_v2(
    room_key: &[u8; 32],
    server_eph: &EphemeralKey,
    env: &Envelope,
) -> Result<(Vec<u8>, [u8; 32]), String> {
    let client_pk_b64 = env
        .eph_pk
        .as_deref()
        .ok_or_else(|| "envelope v2 sans eph_pk".to_string())?;
    let shared = server_eph.dh(client_pk_b64)?;
    let session_key = derive_session_key(room_key, &shared);
    let plain = decrypt_inner(&session_key, env)?;
    Ok((plain, session_key))
}

/// Convenience: encrypt a JSON-serializable value and return the
/// JSON-string envelope ready to be put in an HTTP body.
pub fn encrypt_json<T: Serialize>(key: &[u8; 32], value: &T) -> Result<String, String> {
    let plain = serde_json::to_vec(value).map_err(|e| format!("JSON sérialisation : {e}"))?;
    let env = encrypt(key, &plain)?;
    serde_json::to_string(&env).map_err(|e| format!("envelope sérialisation : {e}"))
}

/// Convenience: decrypt a JSON-string envelope and parse the plaintext as JSON.
pub fn decrypt_json<T: for<'a> Deserialize<'a>>(
    key: &[u8; 32],
    body: &str,
) -> Result<T, String> {
    let env: Envelope = serde_json::from_str(body)
        .map_err(|e| format!("body n'est pas une enveloppe JSON : {e}"))?;
    let plain = decrypt(key, &env)?;
    serde_json::from_slice(&plain).map_err(|e| format!("JSON intérieur : {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let (env, _) =
            encrypt_v2(&room_key1, &server_eph.public_b64(), b"payload").unwrap();
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

        let (env, _) =
            encrypt_v2(&room_key, &real_server.public_b64(), b"payload").unwrap();
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

        let (captured, _) =
            encrypt_v2(&room_key, &old_server_eph.public_b64(), b"yesterday's secret").unwrap();

        // Simulate app restart : new ephemeral keypair, old one is gone.
        let new_server_eph = EphemeralKey::generate();
        drop(old_server_eph);

        // Even with the room secret, the new server can't decrypt.
        assert!(decrypt_request_v2(&room_key, &new_server_eph, &captured).is_err());
    }
}
