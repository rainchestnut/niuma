//! Cryptographic encoding and signing helpers shared by pairing and auth.

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use chacha20poly1305::{
    ChaCha20Poly1305, Nonce,
    aead::{Aead, KeyInit, Payload},
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use signature::Signer;
use x25519_dalek::{PublicKey as X25519PublicKey, StaticSecret};

pub const ED25519_PREFIX: &str = "ed25519:";
pub const X25519_PREFIX: &str = "x25519:";
const PAYLOAD_ENVELOPE_VERSION: u8 = 1;
const PAYLOAD_ENVELOPE_ALG: &str = "X25519-HKDF-SHA256-CHACHA20POLY1305";

pub fn encode_prefixed(prefix: &str, raw: &[u8]) -> String {
    format!("{prefix}{}", STANDARD.encode(raw))
}

pub fn decode_prefixed(value: &str, expected_prefix: &str) -> Result<Vec<u8>> {
    let encoded = value.strip_prefix(expected_prefix).unwrap_or(value);
    STANDARD
        .decode(encoded)
        .with_context(|| format!("invalid {expected_prefix} base64 material"))
}

pub fn generate_ed25519_keypair() -> (String, String) {
    let signing_key = SigningKey::generate(&mut OsRng);
    let public_key = signing_key.verifying_key();
    (
        encode_prefixed(ED25519_PREFIX, &signing_key.to_bytes()),
        encode_prefixed(ED25519_PREFIX, public_key.as_bytes()),
    )
}

pub fn generate_x25519_keypair() -> (String, String) {
    let secret = StaticSecret::random_from_rng(OsRng);
    let public = X25519PublicKey::from(&secret);
    (
        encode_prefixed(X25519_PREFIX, secret.to_bytes().as_ref()),
        encode_prefixed(X25519_PREFIX, public.as_bytes()),
    )
}

pub fn sign_ed25519(private_key: &str, message: &str) -> Result<String> {
    let raw = decode_prefixed(private_key, ED25519_PREFIX)?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("ed25519 private key must be 32 bytes"))?;
    let key = SigningKey::from_bytes(&bytes);
    let signature = key.sign(message.as_bytes());
    Ok(encode_prefixed(ED25519_PREFIX, &signature.to_bytes()))
}

pub fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(value.as_ref()))
}

#[derive(Debug, Copy, Clone)]
pub enum PayloadDirection {
    IosToAgent,
    AgentToIos,
}

impl PayloadDirection {
    fn as_str(self) -> &'static str {
        match self {
            Self::IosToAgent => "ios_to_agent",
            Self::AgentToIos => "agent_to_ios",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct PayloadEnvelope {
    version: u8,
    alg: String,
    kid: String,
    nonce: String,
    ciphertext: String,
}

pub fn payload_aad(fields: &[(&str, String)]) -> Vec<u8> {
    fields
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("\n")
        .into_bytes()
}

pub fn encrypt_payload(
    private_key: &str,
    peer_public_key: &str,
    binding_id: &str,
    direction: PayloadDirection,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<String> {
    let key = derive_payload_key(private_key, peer_public_key, binding_id, direction)?;
    let mut nonce = [0_u8; 12];
    OsRng.fill_bytes(&mut nonce);
    let cipher = ChaCha20Poly1305::new_from_slice(&key)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("payload encryption failed"))?;
    let envelope = PayloadEnvelope {
        version: PAYLOAD_ENVELOPE_VERSION,
        alg: PAYLOAD_ENVELOPE_ALG.to_string(),
        kid: binding_id.to_string(),
        nonce: STANDARD.encode(nonce),
        ciphertext: STANDARD.encode(ciphertext),
    };
    Ok(serde_json::to_string(&envelope)?)
}

pub fn decrypt_payload(
    private_key: &str,
    peer_public_key: &str,
    binding_id: &str,
    direction: PayloadDirection,
    envelope_json: &str,
    aad: &[u8],
) -> Result<Vec<u8>> {
    let envelope: PayloadEnvelope =
        serde_json::from_str(envelope_json).context("payload envelope must be JSON")?;
    if envelope.version != PAYLOAD_ENVELOPE_VERSION || envelope.alg != PAYLOAD_ENVELOPE_ALG {
        anyhow::bail!("unsupported payload envelope");
    }
    if envelope.kid != binding_id {
        anyhow::bail!("payload envelope binding mismatch");
    }
    let nonce = STANDARD
        .decode(envelope.nonce)
        .context("invalid payload nonce")?;
    if nonce.len() != 12 {
        anyhow::bail!("payload nonce must be 12 bytes");
    }
    let ciphertext = STANDARD
        .decode(envelope.ciphertext)
        .context("invalid payload ciphertext")?;
    let key = derive_payload_key(private_key, peer_public_key, binding_id, direction)?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)?;
    cipher
        .decrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: ciphertext.as_ref(),
                aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("payload decrypt failed"))
}

fn derive_payload_key(
    private_key: &str,
    peer_public_key: &str,
    binding_id: &str,
    direction: PayloadDirection,
) -> Result<[u8; 32]> {
    let private_raw = decode_prefixed(private_key, X25519_PREFIX)?;
    let public_raw = decode_prefixed(peer_public_key, X25519_PREFIX)?;
    let private_bytes: [u8; 32] = private_raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("x25519 private key must be 32 bytes"))?;
    let public_bytes: [u8; 32] = public_raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("x25519 public key must be 32 bytes"))?;
    let shared =
        StaticSecret::from(private_bytes).diffie_hellman(&X25519PublicKey::from(public_bytes));
    let hk = Hkdf::<Sha256>::new(Some(binding_id.as_bytes()), shared.as_bytes());
    let mut key = [0_u8; 32];
    hk.expand(
        format!("niuma e2ee v1:{}", direction.as_str()).as_bytes(),
        &mut key,
    )
    .map_err(|_| anyhow::anyhow!("failed to derive payload key"))?;
    Ok(key)
}

pub fn fingerprint_public_key(public_key: &str, prefix: &str) -> Result<String> {
    let raw = decode_prefixed(public_key, prefix)?;
    Ok(sha256_hex(raw)[..16].to_string())
}

pub fn random_urlsafe_token(bytes: usize) -> String {
    let mut raw = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

/// Decrypt the iOS pairing handshake with a one-time X25519 private key.
pub fn decrypt_pairing_handshake(
    private_key: &str,
    peer_public_key: &str,
    nonce: &str,
    ciphertext: &str,
    pair_token: &str,
) -> Result<Vec<u8>> {
    let private_raw = decode_prefixed(private_key, X25519_PREFIX)?;
    let public_raw = decode_prefixed(peer_public_key, X25519_PREFIX)?;
    let private_bytes: [u8; 32] = private_raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("x25519 private key must be 32 bytes"))?;
    let public_bytes: [u8; 32] = public_raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("x25519 public key must be 32 bytes"))?;
    let shared =
        StaticSecret::from(private_bytes).diffie_hellman(&X25519PublicKey::from(public_bytes));
    let hk = Hkdf::<Sha256>::new(Some(pair_token.as_bytes()), shared.as_bytes());
    let mut key = [0_u8; 32];
    hk.expand(b"niuma pairing handshake", &mut key)
        .map_err(|_| anyhow::anyhow!("failed to derive pairing key"))?;
    let nonce_bytes = STANDARD.decode(nonce).context("invalid pairing nonce")?;
    if nonce_bytes.len() != 12 {
        anyhow::bail!("pairing nonce must be 12 bytes")
    }
    let ciphertext_bytes = STANDARD
        .decode(ciphertext)
        .context("invalid pairing ciphertext")?;
    let cipher = ChaCha20Poly1305::new_from_slice(&key)?;
    cipher
        .decrypt(Nonce::from_slice(&nonce_bytes), ciphertext_bytes.as_ref())
        .map_err(|_| anyhow::anyhow!("pairing handshake decrypt failed"))
}

/// Parse a prefixed Ed25519 public key to prove stored identity material is usable.
pub fn validate_ed25519_public_key(public_key: &str) -> Result<()> {
    let raw = decode_prefixed(public_key, ED25519_PREFIX)?;
    let bytes: [u8; 32] = raw
        .try_into()
        .map_err(|_| anyhow::anyhow!("ed25519 public key must be 32 bytes"))?;
    VerifyingKey::from_bytes(&bytes).context("invalid ed25519 public key")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_envelope_round_trips_with_bound_aad() {
        let (agent_private, agent_public) = generate_x25519_keypair();
        let (ios_private, ios_public) = generate_x25519_keypair();
        let aad = payload_aad(&[
            ("kind", "task_start".to_string()),
            ("device_id", "ios-device".to_string()),
            ("agent_id", "agent-device".to_string()),
        ]);

        let envelope = encrypt_payload(
            &ios_private,
            &agent_public,
            "binding-id",
            PayloadDirection::IosToAgent,
            b"hello encrypted payload",
            &aad,
        )
        .expect("encrypt payload");
        let plaintext = decrypt_payload(
            &agent_private,
            &ios_public,
            "binding-id",
            PayloadDirection::IosToAgent,
            &envelope,
            &aad,
        )
        .expect("decrypt payload");
        assert_eq!(plaintext, b"hello encrypted payload");

        let bad_aad = payload_aad(&[("kind", "task_update".to_string())]);
        assert!(
            decrypt_payload(
                &agent_private,
                &ios_public,
                "binding-id",
                PayloadDirection::IosToAgent,
                &envelope,
                &bad_aad,
            )
            .is_err()
        );
    }
}
