//! JSON contracts shared by the iOS app, desktop gateway, and server routes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct DeviceRegisterRequest {
    pub device_type: String,
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub os_type: Option<String>,
    pub push_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceRegisterResponse {
    pub registered: bool,
    pub server_time: i64,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeRequest {
    pub device_id: String,
}

#[derive(Debug, Serialize)]
pub struct ChallengeResponse {
    pub challenge_id: String,
    pub challenge: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct VerifyRequest {
    pub device_id: String,
    pub challenge_id: String,
    pub timestamp: i64,
    pub nonce: String,
    pub request_digest: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyResponse {
    pub verified: bool,
    pub session_token: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PairRequest {
    pub agent_id: String,
    pub agent_pairing_public_key: String,
}

#[derive(Debug, Serialize)]
pub struct PairRequestResponse {
    pub pair_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct PairConfirmRequest {
    pub device_id: String,
    pub device_name: String,
    pub public_key: String,
    pub os_type: Option<String>,
    pub push_token: Option<String>,
    pub agent_id: String,
    pub pair_token: String,
    pub agent_pairing_public_key: String,
    pub encrypted_handshake: String,
    pub signature: String,
}

#[derive(Debug, Serialize)]
pub struct PairConfirmResponse {
    pub binding_id: String,
    pub status: String,
    pub agent_ack: Value,
}

#[derive(Debug, Deserialize)]
pub struct PairRevokeRequest {
    pub device_id: String,
    pub agent_id: String,
}

#[derive(Debug, Serialize)]
pub struct PairRevokeResponse {
    pub revoked: bool,
}

#[derive(Debug, Serialize)]
pub struct PairBindingRevokeResponse {
    pub binding_id: String,
    pub revoked: bool,
}

#[derive(Debug, Deserialize)]
pub struct PushTokenUpdateRequest {
    pub device_id: String,
    pub push_token: String,
}

#[derive(Debug, Serialize)]
pub struct PushTokenUpdateResponse {
    pub updated: bool,
}

#[derive(Debug, Deserialize)]
pub struct TransferEnsureRequest {
    pub source_device_id: String,
    pub target_device_id: String,
    pub direction: String,
    pub encrypted_size_bytes: usize,
}

#[derive(Debug, Serialize)]
pub struct TransferEnsureResponse {
    pub transfer_id: String,
    pub expires_at: i64,
    pub needs_upload: bool,
}

#[derive(Debug, Serialize)]
pub struct TransferUploadResponse {
    pub uploaded: bool,
    pub expires_at: i64,
}

#[derive(Debug, Deserialize)]
pub struct TransferAckRequest {
    pub receiver_device_id: String,
}

#[derive(Debug, Serialize)]
pub struct TransferAckResponse {
    pub acknowledged: bool,
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
}
