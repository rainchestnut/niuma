//! APNs delivery for payload-blind task progress notifications.
//!
//! The server never decrypts the Gateway-provided business locator. APNs only
//! receives a generic visible alert plus opaque ciphertext for the iOS app to
//! decrypt after the user opens the notification.

use std::{fs, sync::Arc};

use anyhow::{Context, Result, bail};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::Client;
use serde::Serialize;
use tracing::{info, warn};

use crate::config::Settings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushSendOutcome {
    Sent,
    Disabled,
}

#[derive(Clone)]
pub struct ApnsPushService {
    client: Option<Arc<ApnsClient>>,
}

struct ApnsClient {
    http: Client,
    key_id: String,
    team_id: String,
    topic: String,
    endpoint: String,
    signing_key: EncodingKey,
}

#[derive(Serialize)]
struct ApnsClaims<'a> {
    iss: &'a str,
    iat: usize,
}

#[derive(Serialize)]
struct TaskProgressApnsPayload<'a> {
    aps: ApsPayload<'a>,
    kind: &'a str,
    agent_id: &'a str,
    ciphertext: &'a str,
}

#[derive(Serialize)]
struct ApsPayload<'a> {
    alert: ApsAlert<'a>,
    sound: &'a str,
    category: &'a str,
    #[serde(rename = "thread-id")]
    thread_id: &'a str,
}

#[derive(Serialize)]
struct ApsAlert<'a> {
    title: &'a str,
    body: &'a str,
}

impl ApnsPushService {
    /// Build an APNs service from deployment config, leaving local development disabled by default.
    pub fn new(settings: &Settings) -> Result<Self> {
        let Some(key_id) = settings.apns_key_id.clone() else {
            return Ok(Self { client: None });
        };
        let Some(team_id) = settings.apns_team_id.clone() else {
            warn!("NIUMA_APNS_KEY_ID is set but NIUMA_APNS_TEAM_ID is missing; APNs disabled");
            return Ok(Self { client: None });
        };
        let Some(topic) = settings.apns_topic.clone() else {
            warn!("NIUMA_APNS_KEY_ID is set but NIUMA_APNS_TOPIC is missing; APNs disabled");
            return Ok(Self { client: None });
        };
        let Some(key_pem) = apns_key_pem(settings)? else {
            warn!("NIUMA_APNS_KEY_ID is set but no APNs signing key was configured; APNs disabled");
            return Ok(Self { client: None });
        };
        let endpoint = match settings.apns_environment.as_str() {
            "production" => "https://api.push.apple.com",
            "sandbox" | "development" => "https://api.sandbox.push.apple.com",
            other => bail!("NIUMA_APNS_ENVIRONMENT must be sandbox or production, got {other}"),
        }
        .to_string();
        let signing_key = EncodingKey::from_ec_pem(key_pem.as_bytes())
            .context("failed to load APNs ES256 signing key")?;
        Ok(Self {
            client: Some(Arc::new(ApnsClient {
                http: Client::builder().build()?,
                key_id,
                team_id,
                topic,
                endpoint,
                signing_key,
            })),
        })
    }

    /// Send a generic task-progress alert with an opaque encrypted business locator.
    pub async fn send_task_progress(
        &self,
        device_token: &str,
        agent_id: &str,
        ciphertext: &str,
    ) -> Result<PushSendOutcome> {
        let Some(client) = self.client.as_ref() else {
            return Ok(PushSendOutcome::Disabled);
        };
        client
            .send_task_progress(device_token, agent_id, ciphertext)
            .await?;
        Ok(PushSendOutcome::Sent)
    }
}

impl ApnsClient {
    async fn send_task_progress(
        &self,
        device_token: &str,
        agent_id: &str,
        ciphertext: &str,
    ) -> Result<()> {
        let token = self.jwt()?;
        let url = format!("{}/3/device/{}", self.endpoint, device_token);
        let payload = TaskProgressApnsPayload {
            aps: ApsPayload {
                alert: ApsAlert {
                    title: "Niuma",
                    body: "任务有新的进度",
                },
                sound: "default",
                category: "task_progress",
                thread_id: "niuma-task-progress",
            },
            kind: "task_progress",
            agent_id,
            ciphertext,
        };
        let response = self
            .http
            .post(url)
            .bearer_auth(token)
            .header("apns-topic", &self.topic)
            .header("apns-push-type", "alert")
            .header("apns-priority", "10")
            .json(&payload)
            .send()
            .await
            .context("APNs request failed")?;
        if response.status().is_success() {
            info!("apns_task_progress_sent");
            return Ok(());
        }
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<unreadable APNs error body>".to_string());
        bail!("APNs task progress request failed with {status}: {body}")
    }

    fn jwt(&self) -> Result<String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.key_id.clone());
        encode(
            &header,
            &ApnsClaims {
                iss: &self.team_id,
                iat: chrono::Utc::now().timestamp() as usize,
            },
            &self.signing_key,
        )
        .context("failed to sign APNs provider token")
    }
}

fn apns_key_pem(settings: &Settings) -> Result<Option<String>> {
    if let Some(pem) = settings.apns_auth_key_pem.as_ref() {
        return Ok(Some(pem.clone()));
    }
    let Some(path) = settings.apns_auth_key_path.as_ref() else {
        return Ok(None);
    };
    fs::read_to_string(path)
        .with_context(|| format!("failed to read APNs key {}", path.display()))
        .map(Some)
}
