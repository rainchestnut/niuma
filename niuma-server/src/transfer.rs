//! Content-addressed temporary file transfer relay.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{config::Settings, crypto};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferManifest {
    pub transfer_id: String,
    pub source_device_id: String,
    pub target_device_id: String,
    pub direction: String,
    pub encrypted_size_bytes: usize,
    pub expires_at: i64,
}

#[derive(Debug, Clone)]
pub struct TransferEnsureResult {
    pub manifest: TransferManifest,
    pub needs_upload: bool,
}

#[derive(Debug, Clone)]
pub struct TransferStore {
    root: PathBuf,
    ttl_seconds: i64,
    max_encrypted_bytes: usize,
}

impl TransferStore {
    /// Open the temporary payload relay rooted at the configured directory.
    pub fn new(settings: &Settings) -> Result<Self> {
        std::fs::create_dir_all(&settings.transfer_storage_dir).with_context(|| {
            format!(
                "failed to create {}",
                settings.transfer_storage_dir.display()
            )
        })?;
        Ok(Self {
            root: settings.transfer_storage_dir.clone(),
            ttl_seconds: settings.transfer_ttl_seconds,
            max_encrypted_bytes: settings.transfer_max_encrypted_bytes,
        })
    }

    pub async fn ensure_transfer(
        &self,
        transfer_id: &str,
        source_device_id: &str,
        target_device_id: &str,
        direction: &str,
        encrypted_size_bytes: usize,
    ) -> Result<TransferEnsureResult> {
        validate_sha256(transfer_id)?;
        if encrypted_size_bytes == 0 {
            bail!("encrypted_size_bytes must be positive");
        }
        if encrypted_size_bytes > self.max_encrypted_bytes {
            bail!("transfer exceeds maximum size");
        }
        if !matches!(direction, "ios_to_agent" | "agent_to_ios") {
            bail!("direction must be ios_to_agent or agent_to_ios");
        }

        let transfer_id = transfer_id.to_ascii_lowercase();
        let mut existing_completed = false;
        if let Ok(existing) = self.read_manifest(&transfer_id).await {
            if existing.expires_at > now_ts() {
                existing_completed = existing.encrypted_size_bytes == encrypted_size_bytes
                    && self.payload_is_complete(&existing).await;
            }
        }
        let manifest = TransferManifest {
            transfer_id: transfer_id.clone(),
            source_device_id: source_device_id.to_string(),
            target_device_id: target_device_id.to_string(),
            direction: direction.to_string(),
            encrypted_size_bytes,
            expires_at: now_ts() + self.ttl_seconds,
        };
        fs::create_dir_all(self.transfer_dir(&transfer_id)).await?;
        self.write_manifest(&manifest).await?;
        Ok(TransferEnsureResult {
            manifest,
            needs_upload: !existing_completed,
        })
    }

    pub async fn write_payload(&self, transfer_id: &str, body: &[u8]) -> Result<TransferManifest> {
        let mut manifest = self.read_manifest(transfer_id).await?;
        self.ensure_not_expired(&manifest).await?;
        if body.is_empty() {
            bail!("transfer body is empty");
        }
        if body.len() > self.max_encrypted_bytes {
            bail!("transfer exceeds maximum size");
        }
        if body.len() != manifest.encrypted_size_bytes {
            bail!("encrypted_size_bytes mismatch");
        }
        if crypto::sha256_hex(body) != manifest.transfer_id {
            bail!("transfer_id hash mismatch");
        }
        fs::write(self.payload_path(transfer_id), body).await?;
        manifest.expires_at = now_ts() + self.ttl_seconds;
        self.write_manifest(&manifest).await?;
        Ok(manifest)
    }

    pub async fn read_payload(&self, transfer_id: &str) -> Result<Vec<u8>> {
        let manifest = self.read_manifest(transfer_id).await?;
        self.ensure_not_expired(&manifest).await?;
        if !self.payload_is_complete(&manifest).await {
            let _ = fs::remove_dir_all(self.transfer_dir(transfer_id)).await;
            bail!("transfer payload not found");
        }
        Ok(fs::read(self.payload_path(transfer_id)).await?)
    }

    pub async fn read_manifest(&self, transfer_id: &str) -> Result<TransferManifest> {
        validate_sha256(transfer_id)?;
        let data = fs::read(self.manifest_path(transfer_id))
            .await
            .context("transfer not found")?;
        Ok(serde_json::from_slice(&data)?)
    }

    pub async fn ack_transfer(&self, transfer_id: &str) -> Result<()> {
        let mut manifest = self.read_manifest(transfer_id).await?;
        self.ensure_not_expired(&manifest).await?;
        manifest.expires_at = now_ts() + self.ttl_seconds;
        self.write_manifest(&manifest).await
    }

    pub async fn cleanup_expired(&self) {
        let Ok(mut entries) = fs::read_dir(&self.root).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let manifest_path = path.join("manifest.json");
            let remove = match fs::read(&manifest_path).await {
                Ok(data) => serde_json::from_slice::<TransferManifest>(&data)
                    .map(|manifest| manifest.expires_at <= now_ts())
                    .unwrap_or(true),
                Err(_) => true,
            };
            if remove {
                let _ = fs::remove_dir_all(path).await;
            }
        }
    }

    async fn payload_is_complete(&self, manifest: &TransferManifest) -> bool {
        let path = self.payload_path(&manifest.transfer_id);
        let Ok(metadata) = fs::metadata(&path).await else {
            return false;
        };
        if metadata.len() as usize != manifest.encrypted_size_bytes {
            return false;
        }
        fs::read(path)
            .await
            .map(|body| crypto::sha256_hex(body) == manifest.transfer_id)
            .unwrap_or(false)
    }

    async fn write_manifest(&self, manifest: &TransferManifest) -> Result<()> {
        fs::write(
            self.manifest_path(&manifest.transfer_id),
            serde_json::to_vec(manifest)?,
        )
        .await?;
        Ok(())
    }

    async fn ensure_not_expired(&self, manifest: &TransferManifest) -> Result<()> {
        if manifest.expires_at <= now_ts() {
            let _ = fs::remove_dir_all(self.transfer_dir(&manifest.transfer_id)).await;
            bail!("transfer expired");
        }
        Ok(())
    }

    fn transfer_dir(&self, transfer_id: &str) -> PathBuf {
        safe_transfer_path(&self.root, transfer_id)
    }

    fn manifest_path(&self, transfer_id: &str) -> PathBuf {
        self.transfer_dir(transfer_id).join("manifest.json")
    }

    fn payload_path(&self, transfer_id: &str) -> PathBuf {
        self.transfer_dir(transfer_id).join("payload.bin")
    }
}

fn safe_transfer_path(root: &Path, transfer_id: &str) -> PathBuf {
    root.join(transfer_id)
}

fn validate_sha256(value: &str) -> Result<()> {
    if value.len() != 64 || !value.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("transfer_id must be a hex sha256 digest");
    }
    Ok(())
}

fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::TransferStore;
    use crate::crypto;

    fn test_store() -> TransferStore {
        let root = std::env::temp_dir().join(format!("niuma-transfer-{}", crypto::random_token(8)));
        std::fs::create_dir_all(&root).expect("create temp transfer dir");
        TransferStore {
            root,
            ttl_seconds: 60,
            max_encrypted_bytes: 1024,
        }
    }

    #[tokio::test]
    async fn completed_transfer_is_reused_by_hash() {
        let store = test_store();
        let payload = b"encrypted image payload".to_vec();
        let transfer_id = crypto::sha256_hex(&payload);

        let first = store
            .ensure_transfer(
                &transfer_id,
                "ios-device",
                "agent-device",
                "ios_to_agent",
                payload.len(),
            )
            .await
            .expect("ensure first manifest");
        assert!(first.needs_upload);

        store
            .write_payload(&transfer_id, &payload)
            .await
            .expect("write complete payload");
        let second = store
            .ensure_transfer(
                &transfer_id,
                "ios-device",
                "agent-device",
                "ios_to_agent",
                payload.len(),
            )
            .await
            .expect("reuse complete transfer");
        assert!(!second.needs_upload);
        assert_eq!(store.read_payload(&transfer_id).await.unwrap(), payload);
    }

    #[tokio::test]
    async fn upload_rejects_body_that_does_not_match_hash_id() {
        let store = test_store();
        let transfer_id = crypto::sha256_hex(b"expected payload");
        store
            .ensure_transfer(&transfer_id, "ios", "agent", "ios_to_agent", 14)
            .await
            .expect("ensure manifest");

        let error = store
            .write_payload(&transfer_id, b"actual payload")
            .await
            .expect_err("hash mismatch should fail");
        assert!(error.to_string().contains("hash mismatch"));
    }
}
