//! HTTP control-plane client for niuma-server.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use reqwest::{Client, Response, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{crypto, identity::AgentIdentity};

#[derive(Debug, Clone)]
pub struct NiumaServerClient {
    base_url: Url,
    client: Client,
}

#[derive(Debug)]
struct HttpStatusError {
    operation: &'static str,
    status: StatusCode,
    detail: Option<String>,
}

impl fmt::Display for HttpStatusError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.detail.as_deref().filter(|detail| !detail.is_empty()) {
            Some(detail) => write!(
                formatter,
                "{} failed with {}: {}",
                self.operation, self.status, detail
            ),
            None => write!(formatter, "{} failed with {}", self.operation, self.status),
        }
    }
}

impl std::error::Error for HttpStatusError {}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    #[serde(default)]
    detail: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ChallengeResponse {
    pub challenge_id: String,
    pub challenge: String,
}

#[derive(Debug, Deserialize)]
pub struct VerifyResponse {
    pub verified: bool,
    pub session_token: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PairToken {
    pub pair_token: String,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransferEnsureResponse {
    pub transfer_id: String,
    pub expires_at: i64,
    pub needs_upload: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransferUploadResponse {
    pub uploaded: bool,
    pub expires_at: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransferAckResponse {
    pub acknowledged: bool,
}

impl NiumaServerClient {
    pub fn new(base_url: &str) -> Result<Self> {
        Ok(Self {
            base_url: normalized_base_url(base_url)?,
            client: Client::new(),
        })
    }

    pub fn base_url(&self) -> &Url {
        &self.base_url
    }

    /// Check that the configured server accepts HTTP traffic.
    pub async fn health(&self) -> Result<()> {
        let response = self.client.get(self.url("/healthz")?).send().await?;
        ensure_success(response.status(), "/healthz").await
    }

    pub async fn register_agent(&self, identity: &AgentIdentity) -> Result<()> {
        #[derive(Serialize)]
        struct Register<'a> {
            device_type: &'static str,
            device_id: &'a str,
            device_name: &'a str,
            public_key: &'a str,
            os_type: &'a str,
        }
        let response = self
            .client
            .post(self.url("/devices/register")?)
            .json(&Register {
                device_type: "agent",
                device_id: &identity.agent_id,
                device_name: &identity.device_name,
                public_key: &identity.signing_public_key,
                os_type: &identity.os_type,
            })
            .send()
            .await?;
        ensure_success(response.status(), "/devices/register").await
    }

    pub async fn authenticate_agent(&self, identity: &AgentIdentity) -> Result<String> {
        let challenge = self.issue_challenge(&identity.agent_id).await?;
        let timestamp = unix_timestamp();
        let nonce = crypto::random_urlsafe_token(12);
        let digest = auth_digest(
            &identity.agent_id,
            &challenge.challenge_id,
            &challenge.challenge,
            timestamp,
            &nonce,
        );
        let signature = crypto::sign_ed25519(&identity.signing_private_key, &digest)?;
        #[derive(Serialize)]
        struct Verify<'a> {
            device_id: &'a str,
            challenge_id: &'a str,
            timestamp: i64,
            nonce: &'a str,
            request_digest: &'a str,
            signature: &'a str,
        }
        let response = self
            .client
            .post(self.url("/auth/verify")?)
            .json(&Verify {
                device_id: &identity.agent_id,
                challenge_id: &challenge.challenge_id,
                timestamp,
                nonce: &nonce,
                request_digest: &digest,
                signature: &signature,
            })
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("POST /auth/verify failed with {status}");
        }
        let payload: VerifyResponse = response.json().await?;
        if payload.verified {
            payload
                .session_token
                .context("server returned verified=true without session_token")
        } else {
            anyhow::bail!("agent authentication failed")
        }
    }

    pub async fn request_pair_token(
        &self,
        identity: &AgentIdentity,
        session_token: &str,
        agent_pairing_public_key: &str,
    ) -> Result<PairToken> {
        #[derive(Serialize)]
        struct PairRequest<'a> {
            agent_id: &'a str,
            agent_pairing_public_key: &'a str,
        }
        let response = self
            .client
            .post(self.url("/pair/request")?)
            .header("X-Session-Token", session_token)
            .json(&PairRequest {
                agent_id: &identity.agent_id,
                agent_pairing_public_key,
            })
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_status_error(response, "POST /pair/request").await);
        }
        Ok(response.json().await?)
    }

    /// Ensure a content-addressed transfer manifest exists on niuma-server.
    pub async fn ensure_transfer(
        &self,
        transfer_id: &str,
        source_device_id: &str,
        target_device_id: &str,
        direction: &str,
        encrypted_size_bytes: usize,
        session_token: &str,
    ) -> Result<TransferEnsureResponse> {
        #[derive(Serialize)]
        struct Ensure<'a> {
            source_device_id: &'a str,
            target_device_id: &'a str,
            direction: &'a str,
            encrypted_size_bytes: usize,
        }
        let response = self
            .client
            .post(self.url(&format!("/transfers/{transfer_id}/ensure"))?)
            .header("X-Session-Token", session_token)
            .json(&Ensure {
                source_device_id,
                target_device_id,
                direction,
                encrypted_size_bytes,
            })
            .send()
            .await?;
        ensure_success(response.status(), "/transfers/{transfer_id}/ensure").await?;
        Ok(response.json().await?)
    }

    /// Upload a complete transfer body from this desktop agent.
    pub async fn upload_transfer(
        &self,
        transfer_id: &str,
        body: Vec<u8>,
        device_id: &str,
        session_token: &str,
    ) -> Result<TransferUploadResponse> {
        let response = self
            .client
            .put(self.url(&format!("/transfers/{transfer_id}"))?)
            .header("X-Session-Token", session_token)
            .header("X-Device-ID", device_id)
            .header("Content-Type", "application/octet-stream")
            .body(body)
            .send()
            .await?;
        ensure_success(response.status(), "/transfers/{transfer_id}").await?;
        Ok(response.json().await?)
    }

    /// Download one transfer payload for this desktop agent.
    pub async fn download_transfer(
        &self,
        transfer_id: &str,
        device_id: &str,
        session_token: &str,
    ) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(self.url(&format!("/transfers/{transfer_id}"))?)
            .header("X-Session-Token", session_token)
            .query(&[("device_id", device_id)])
            .send()
            .await?;
        ensure_success(response.status(), "/transfers/{transfer_id}").await?;
        Ok(response.bytes().await?.to_vec())
    }

    /// Refresh transfer TTL after the desktop has stored the payload locally.
    pub async fn ack_transfer(
        &self,
        transfer_id: &str,
        receiver_device_id: &str,
        session_token: &str,
    ) -> Result<TransferAckResponse> {
        #[derive(Serialize)]
        struct Ack<'a> {
            receiver_device_id: &'a str,
        }
        let response = self
            .client
            .post(self.url(&format!("/transfers/{transfer_id}/ack"))?)
            .header("X-Session-Token", session_token)
            .json(&Ack { receiver_device_id })
            .send()
            .await?;
        ensure_success(response.status(), "/transfers/{transfer_id}/ack").await?;
        Ok(response.json().await?)
    }

    async fn issue_challenge(&self, device_id: &str) -> Result<ChallengeResponse> {
        #[derive(Serialize)]
        struct Challenge<'a> {
            device_id: &'a str,
        }
        let response = self
            .client
            .post(self.url("/auth/challenge")?)
            .json(&Challenge { device_id })
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            anyhow::bail!("POST /auth/challenge failed with {status}");
        }
        Ok(response.json().await?)
    }

    fn url(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path.trim_start_matches('/'))
            .with_context(|| format!("failed to build server URL for {path}"))
    }
}

/// Normalize server base URLs so reverse-proxy prefixes behave as directories.
///
/// `url::Url::join` treats a base path without a trailing slash as a file path,
/// so `https://host/niuma-server` joined with `/healthz` would otherwise target
/// `https://host/healthz`. Appending the slash keeps `/niuma-server/*` routes
/// intact while preserving root URLs such as `http://127.0.0.1:8000`.
fn normalized_base_url(base_url: &str) -> Result<Url> {
    let mut url = Url::parse(base_url).context("invalid niuma-server URL")?;
    match url.scheme() {
        "http" | "https" => {}
        _ => anyhow::bail!("niuma-server URL must use http or https"),
    }

    let path = url.path();
    if path != "/" && !path.ends_with('/') {
        url.set_path(&format!("{path}/"));
    }
    Ok(url)
}

pub fn auth_digest(
    device_id: &str,
    challenge_id: &str,
    challenge: &str,
    timestamp: i64,
    nonce: &str,
) -> String {
    crypto::sha256_hex(format!(
        "{device_id}:{challenge_id}:{challenge}:{timestamp}:{nonce}"
    ))
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

async fn ensure_success(status: StatusCode, path: &str) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        anyhow::bail!("request {path} failed with {status}")
    }
}

async fn http_status_error(response: Response, operation: &'static str) -> anyhow::Error {
    let status = response.status();
    let detail = response
        .json::<ErrorResponse>()
        .await
        .ok()
        .and_then(|body| body.detail);
    HttpStatusError {
        operation,
        status,
        detail,
    }
    .into()
}

pub fn is_unauthorized_response(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<HttpStatusError>()
            .is_some_and(|http_error| http_error.status == StatusCode::UNAUTHORIZED)
    })
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::{HttpStatusError, NiumaServerClient, is_unauthorized_response};

    #[test]
    fn server_client_treats_proxy_prefix_without_slash_as_directory() {
        let client = NiumaServerClient::new("https://example.invalid/niuma-server")
            .expect("server URL should be valid");

        assert_eq!(
            client.base_url().as_str(),
            "https://example.invalid/niuma-server/"
        );
        assert_eq!(
            client.url("/healthz").expect("health URL").as_str(),
            "https://example.invalid/niuma-server/healthz"
        );
    }

    #[test]
    fn server_client_keeps_root_server_routes() {
        let client =
            NiumaServerClient::new("http://127.0.0.1:8000").expect("server URL should be valid");

        assert_eq!(
            client.url("/healthz").expect("health URL").as_str(),
            "http://127.0.0.1:8000/healthz"
        );
    }

    #[test]
    fn unauthorized_status_errors_are_detected_through_context() {
        let error = anyhow::Error::new(HttpStatusError {
            operation: "POST /pair/request",
            status: StatusCode::UNAUTHORIZED,
            detail: Some("invalid session token".to_string()),
        })
        .context("pair token refresh failed");

        assert!(is_unauthorized_response(&error));
    }
}
