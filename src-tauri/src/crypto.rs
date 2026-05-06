//! Peer-to-peer message encryption.
//!
//! Every body exchanged on the peer-API (port 7655, except `/health`) is
//! wrapped in an `Envelope` of `AES-256-GCM(plaintext)` whose key is derived
//! from the room secret via HKDF-SHA256. All members of a PartaGPU room
//! share that secret (it's encoded in the 4-word passphrase) so they can
//! all read each other's messages, but no one outside the room can.
//!
//! TOTP authentication (header `X-PartaGPU-TOTP`) stays as before and
//! provides anti-replay; encryption layers confidentiality + integrity on
//! top.
//!
//! Security properties (assuming the AES-GCM construction holds):
//! - Confidentiality of message bodies on the wire
//! - Integrity (any byte flip rejected at decrypt time)
//! - Authenticity at the room level (only secret-holders can produce a
//!   valid ciphertext that decrypts cleanly)
//!
//! Out of scope: forward secrecy (no per-session key exchange), no
//! protection against an attacker who joins the same room (they have the
//! key by construction — that's the whole point of a "shared room" design).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use hkdf::Hkdf;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const HKDF_SALT: &[u8] = b"PartaGPU/peer-api/v1";
const HKDF_INFO: &[u8] = b"AES-256-GCM message key";

/// Content-Type header used to signal an encrypted body. Receivers MUST
/// reject bodies that don't carry this content-type — rejecting plaintext
/// is the whole point of mandatory encryption.
pub const ENCRYPTED_CONTENT_TYPE: &str = "application/x-partagpu-encrypted-v1";

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Envelope {
    /// Format version, currently 1.
    pub v: u8,
    /// Random 12-byte nonce, base64-encoded.
    pub nonce: String,
    /// AES-256-GCM ciphertext + 16-byte auth tag, base64-encoded.
    pub ct: String,
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

/// Encrypt `plaintext` with a fresh random nonce. The returned envelope is
/// ready to be JSON-serialized and sent over the wire.
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
    })
}

/// Decrypt an envelope produced by `encrypt`. Returns the plaintext bytes
/// or an error describing why the payload is invalid (auth failure, wrong
/// version, malformed base64, etc.).
pub fn decrypt(key: &[u8; 32], env: &Envelope) -> Result<Vec<u8>, String> {
    if env.v != 1 {
        return Err(format!("version d'enveloppe non supportée : {}", env.v));
    }
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
}
