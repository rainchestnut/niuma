//! Local paired-device key store under ~/.niuma.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDeviceBinding {
    pub binding_id: String,
    pub device_id: String,
    pub agent_id: String,
    pub ios_encryption_public_key: String,
    pub paired_at: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct BindingFile {
    bindings: BTreeMap<String, PairedDeviceBinding>,
}

impl BindingFile {
    fn upsert(&mut self, binding: PairedDeviceBinding) {
        self.bindings.retain(|_, existing| {
            existing.device_id != binding.device_id || existing.agent_id != binding.agent_id
        });
        self.bindings.insert(binding.binding_id.clone(), binding);
    }

    fn binding_by_id(&self, binding_id: &str) -> Option<PairedDeviceBinding> {
        self.bindings.get(binding_id).cloned()
    }

    fn newest_for_device(&self, device_id: &str) -> Option<PairedDeviceBinding> {
        self.bindings
            .values()
            .filter(|binding| binding.device_id == device_id)
            .max_by_key(|binding| binding.paired_at)
            .cloned()
    }

    fn remove_binding(&mut self, binding_id: &str) -> Option<PairedDeviceBinding> {
        self.bindings.remove(binding_id)
    }
}

/// Persist or replace one mobile binding after the server confirms pairing.
pub fn save_binding(binding: PairedDeviceBinding) -> Result<()> {
    let path = bindings_path()?;
    let mut file = read_bindings(&path)?;
    file.upsert(binding);
    write_bindings(&path, &file)
}

/// Find the active binding material needed to decrypt or encrypt mobile payloads.
pub fn binding_for_device(device_id: &str) -> Result<Option<PairedDeviceBinding>> {
    let path = bindings_path()?;
    let file = read_bindings(&path)?;
    Ok(file.newest_for_device(device_id))
}

/// Find one local binding by its stable server-issued binding id.
pub fn binding_for_id(binding_id: &str) -> Result<Option<PairedDeviceBinding>> {
    let path = bindings_path()?;
    let file = read_bindings(&path)?;
    Ok(file.binding_by_id(binding_id))
}

/// List locally known mobile bindings for dashboard diagnostics.
pub fn list_bindings() -> Result<Vec<PairedDeviceBinding>> {
    let path = bindings_path()?;
    let file = read_bindings(&path)?;
    let mut bindings = file.bindings.values().cloned().collect::<Vec<_>>();
    bindings.sort_by(|left, right| right.paired_at.cmp(&left.paired_at));
    Ok(bindings)
}

/// Remove a local dashboard binding after niuma-server has accepted revocation.
pub fn delete_binding(binding_id: &str) -> Result<Option<PairedDeviceBinding>> {
    let path = bindings_path()?;
    let mut file = read_bindings(&path)?;
    let removed = file.remove_binding(binding_id);
    if removed.is_some() {
        write_bindings(&path, &file)?;
    }
    Ok(removed)
}

fn read_bindings(path: &PathBuf) -> Result<BindingFile> {
    if !path.exists() {
        return Ok(BindingFile::default());
    }
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    Ok(serde_json::from_str(&text)
        .with_context(|| format!("failed to parse {}", path.display()))?)
}

fn write_bindings(path: &PathBuf, file: &BindingFile) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let body = serde_json::to_vec_pretty(file)?;
    let mut options = OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        // Pair bindings contain mobile long-term encryption public keys and
        // should follow the same owner-only policy as desktop identity.
        options.mode(0o600);
    }
    let mut handle = options
        .open(path)
        .with_context(|| format!("failed to write {}", path.display()))?;
    handle.write_all(&body)?;
    handle.write_all(b"\n")?;
    Ok(())
}

fn bindings_path() -> Result<PathBuf> {
    Ok(paths::identity_dir()?.join("bindings.json"))
}

#[cfg(test)]
mod tests {
    use super::{BindingFile, PairedDeviceBinding};

    #[test]
    fn upsert_replaces_existing_device_agent_binding() {
        let mut file = BindingFile::default();
        file.upsert(binding("old", "ios-1", "agent-1", 10));
        file.upsert(binding("new", "ios-1", "agent-1", 20));
        file.upsert(binding("other", "ios-2", "agent-1", 15));

        assert_eq!(file.bindings.len(), 2);
        assert!(!file.bindings.contains_key("old"));
        assert_eq!(
            file.newest_for_device("ios-1")
                .expect("updated binding")
                .binding_id,
            "new"
        );
    }

    #[test]
    fn newest_for_device_tolerates_legacy_duplicate_bindings() {
        let mut file = BindingFile::default();
        file.bindings
            .insert("old".to_string(), binding("old", "ios-1", "agent-1", 10));
        file.bindings
            .insert("new".to_string(), binding("new", "ios-1", "agent-1", 20));

        assert_eq!(
            file.newest_for_device("ios-1")
                .expect("latest legacy binding")
                .binding_id,
            "new"
        );
    }

    #[test]
    fn remove_binding_deletes_only_requested_binding() {
        let mut file = BindingFile::default();
        file.upsert(binding("target", "ios-1", "agent-1", 10));
        file.upsert(binding("other", "ios-2", "agent-1", 20));

        let removed = file
            .remove_binding("target")
            .expect("target binding should be removed");

        assert_eq!(removed.binding_id, "target");
        assert!(!file.bindings.contains_key("target"));
        assert!(file.bindings.contains_key("other"));
        assert!(file.remove_binding("missing").is_none());
    }

    fn binding(
        binding_id: &str,
        device_id: &str,
        agent_id: &str,
        paired_at: i64,
    ) -> PairedDeviceBinding {
        PairedDeviceBinding {
            binding_id: binding_id.to_string(),
            device_id: device_id.to_string(),
            agent_id: agent_id.to_string(),
            ios_encryption_public_key: "x25519:test".to_string(),
            paired_at,
        }
    }
}
