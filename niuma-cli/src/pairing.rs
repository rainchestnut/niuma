//! Pairing payload generation and refresh state.

use std::collections::HashMap;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::{
    crypto,
    identity::AgentIdentity,
    server::{NiumaServerClient, PairToken},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingPayload {
    pub agent_id: String,
    pub agent_name: String,
    pub pair_token: String,
    pub expires_at: i64,
    pub agent_signing_public_key: String,
    pub agent_encryption_public_key: String,
    pub agent_pairing_public_key: String,
    pub agent_signing_key_fingerprint: String,
    pub agent_encryption_key_fingerprint: String,
    pub signature: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PairingRuntimeState {
    pub payload: Option<PairingPayload>,
    pub last_error: Option<String>,
    #[serde(skip)]
    pub secrets: HashMap<String, PairingSecret>,
}

#[derive(Debug, Clone)]
pub struct PairingSecret {
    pub pair_token: String,
    pub pairing_private_key: String,
    pub pairing_public_key: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct PairingMaterial {
    pub payload: PairingPayload,
    pub secret: PairingSecret,
}

impl PairingPayload {
    pub fn to_qr_json(&self) -> Result<String> {
        Ok(serde_json::to_string(self)?)
    }
}

/// Fetch a fresh pair token and build a signed QR-compatible payload.
pub async fn refresh_payload(
    server: &NiumaServerClient,
    identity: &AgentIdentity,
    session_token: &str,
) -> Result<PairingMaterial> {
    let (pairing_private_key, pairing_public_key) = crypto::generate_x25519_keypair();
    let token = server
        .request_pair_token(identity, session_token, &pairing_public_key)
        .await?;
    build_payload(identity, token, pairing_private_key, pairing_public_key)
}

pub fn build_payload(
    identity: &AgentIdentity,
    token: PairToken,
    pairing_private_key: String,
    pairing_public_key: String,
) -> Result<PairingMaterial> {
    let digest = payload_digest(
        &identity.agent_id,
        &identity.device_name,
        &token.pair_token,
        token.expires_at,
        &identity.signing_public_key,
        &identity.encryption_public_key,
        &pairing_public_key,
    );
    let signature = crypto::sign_ed25519(&identity.signing_private_key, &digest)?;
    let payload = PairingPayload {
        agent_id: identity.agent_id.clone(),
        agent_name: identity.device_name.clone(),
        pair_token: token.pair_token,
        expires_at: token.expires_at,
        agent_signing_public_key: identity.signing_public_key.clone(),
        agent_encryption_public_key: identity.encryption_public_key.clone(),
        agent_pairing_public_key: pairing_public_key.clone(),
        agent_signing_key_fingerprint: identity.signing_key_fingerprint.clone(),
        agent_encryption_key_fingerprint: identity.encryption_key_fingerprint.clone(),
        signature,
    };
    let secret = PairingSecret {
        pair_token: payload.pair_token.clone(),
        pairing_private_key,
        pairing_public_key,
        expires_at: payload.expires_at,
    };
    Ok(PairingMaterial { payload, secret })
}

fn payload_digest(
    agent_id: &str,
    agent_name: &str,
    pair_token: &str,
    expires_at: i64,
    signing_public_key: &str,
    encryption_public_key: &str,
    pairing_public_key: &str,
) -> String {
    crypto::sha256_hex(format!(
        "{agent_id}:{agent_name}:{pair_token}:{expires_at}:{signing_public_key}:{encryption_public_key}:{pairing_public_key}"
    ))
}
