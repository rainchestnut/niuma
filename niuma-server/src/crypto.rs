//! Ed25519 signature and SHA-256 digest helpers for the existing protocol.

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, VerifyingKey};
use rand::RngCore;
use sha2::{Digest, Sha256};
use signature::Verifier;

pub fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(value.as_ref()))
}

pub fn random_token(bytes: usize) -> String {
    let mut raw = vec![0_u8; bytes];
    rand::rngs::OsRng.fill_bytes(&mut raw);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw)
}

pub fn auth_digest(
    device_id: &str,
    challenge_id: &str,
    challenge: &str,
    timestamp: i64,
    nonce: &str,
) -> String {
    sha256_hex(format!(
        "{device_id}:{challenge_id}:{challenge}:{timestamp}:{nonce}"
    ))
}

pub fn pair_digest(
    device_id: &str,
    agent_id: &str,
    pair_token: &str,
    agent_pairing_public_key: &str,
    encrypted_handshake: &str,
) -> String {
    sha256_hex(format!(
        "{device_id}:{agent_id}:{pair_token}:{agent_pairing_public_key}:{encrypted_handshake}"
    ))
}

pub fn pair_ack_digest(
    binding_id: &str,
    device_id: &str,
    agent_id: &str,
    pair_token: &str,
    handshake_hash: &str,
    ack_status: &str,
) -> String {
    sha256_hex(format!(
        "{binding_id}:{device_id}:{agent_id}:{pair_token}:{handshake_hash}:{ack_status}"
    ))
}

pub fn task_start_digest(
    device_id: &str,
    agent_id: &str,
    project_id: &str,
    thread_id: Option<&str>,
    ciphertext: &str,
) -> String {
    sha256_hex(format!(
        "{}:{}:{}:{}:{}",
        device_id,
        agent_id,
        project_id,
        thread_id.unwrap_or_default(),
        ciphertext
    ))
}

pub fn verify_ed25519(public_key: &str, message: &str, signature: &str) -> bool {
    verify_ed25519_inner(public_key, message, signature).is_ok()
}

fn verify_ed25519_inner(public_key: &str, message: &str, signature: &str) -> Result<()> {
    let public_key = decode_prefixed(public_key, "ed25519:")?;
    let signature = decode_prefixed(signature, "ed25519:")?;
    let key_bytes: [u8; 32] = public_key
        .try_into()
        .map_err(|_| anyhow::anyhow!("ed25519 public key must be 32 bytes"))?;
    let signature_bytes: [u8; 64] = signature
        .try_into()
        .map_err(|_| anyhow::anyhow!("ed25519 signature must be 64 bytes"))?;
    let key = VerifyingKey::from_bytes(&key_bytes).context("invalid ed25519 public key")?;
    let signature = Signature::from_bytes(&signature_bytes);
    key.verify(message.as_bytes(), &signature)
        .context("signature invalid")
}

fn decode_prefixed(value: &str, prefix: &str) -> Result<Vec<u8>> {
    let encoded = value.strip_prefix(prefix).unwrap_or(value);
    STANDARD
        .decode(encoded)
        .with_context(|| format!("invalid {prefix}base64 material"))
}

#[cfg(test)]
mod tests {
    use super::{auth_digest, pair_ack_digest, pair_digest, task_start_digest};

    #[test]
    fn digest_shapes_match_existing_protocol() {
        assert_eq!(
            auth_digest("d", "c-id", "c", 12, "n"),
            "063b89ff097687a51082f142747f9333d97cb6c3ef0674257ee77db9b300dd88"
        );
        assert_eq!(
            pair_digest("ios", "agent", "pair", "x25519:key", "opaque"),
            "14f1b97228f19cf6fa3068fc9159ba8fbc78d2f64992db66a90b1ad5f63a8427"
        );
        assert_eq!(
            pair_ack_digest("b", "ios", "agent", "pair", "hash", "accepted"),
            "2ad27b3861f9d94fa81c8905b4541df61eaece43e456da71ed883479217f4782"
        );
        assert_eq!(
            task_start_digest("ios", "agent", "p", None, "payload"),
            "1f66ca84c3e4452c5a5d9d17e5fe2dd464ce5616c9ec88b04aa96031147eb277"
        );
    }
}
