//! Persistent desktop identity for server auth and end-to-end encryption.

use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{crypto, paths};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentIdentity {
    pub agent_id: String,
    pub device_name: String,
    pub os_type: String,
    pub signing_private_key: String,
    pub signing_public_key: String,
    pub signing_key_fingerprint: String,
    pub encryption_private_key: String,
    pub encryption_public_key: String,
    pub encryption_key_fingerprint: String,
}

impl AgentIdentity {
    /// Load the installed gateway identity or create a new one under ~/.niuma.
    pub fn load_or_create(device_name: &str) -> Result<Self> {
        let path = identity_path()?;
        if path.exists() {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read {}", path.display()))?;
            let identity: Self = serde_json::from_str(&text)
                .with_context(|| format!("failed to parse {}", path.display()))?;
            crypto::validate_ed25519_public_key(&identity.signing_public_key)?;
            return Ok(identity);
        }

        let (signing_private_key, signing_public_key) = crypto::generate_ed25519_keypair();
        let (encryption_private_key, encryption_public_key) = crypto::generate_x25519_keypair();
        let identity = Self {
            agent_id: format!("agent_{}", Uuid::new_v4().simple()),
            device_name: device_name.to_string(),
            os_type: "darwin".to_string(),
            signing_key_fingerprint: crypto::fingerprint_public_key(
                &signing_public_key,
                crypto::ED25519_PREFIX,
            )?,
            encryption_key_fingerprint: crypto::fingerprint_public_key(
                &encryption_public_key,
                crypto::X25519_PREFIX,
            )?,
            signing_private_key,
            signing_public_key,
            encryption_private_key,
            encryption_public_key,
        };
        write_identity(&path, &identity)?;
        Ok(identity)
    }
}

fn identity_path() -> Result<PathBuf> {
    Ok(paths::identity_dir()?.join("agent.json"))
}

fn write_identity(path: &PathBuf, identity: &AgentIdentity) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let body = serde_json::to_vec_pretty(identity)?;
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        // Private signing and encryption keys live in this file, so write with
        // user-only permissions instead of relying on the process umask.
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    file.write_all(&body)?;
    file.write_all(b"\n")?;
    Ok(())
}
