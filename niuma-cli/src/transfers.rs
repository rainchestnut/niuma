//! Local and server-backed file transfer helpers.
//!
//! The server stores temporary payloads by content hash. The Rust gateway keeps
//! its local copy under `~/.niuma/transfers`, then hands Codex real filesystem
//! paths instead of opaque `transfer_id` text.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::warn;

use crate::identity::AgentIdentity;
use crate::paths;
use crate::server::NiumaServerClient;
use crate::{bindings, crypto};

const LONG_TEXT_TRANSFER_THRESHOLD_BYTES: usize = 200 * 1024;
const LONG_TEXT_PREVIEW_CHARS: usize = 100;
const LONG_TEXT_MIME_TYPE: &str = "text/plain; charset=utf-8";

#[derive(Debug, Clone)]
pub struct TransferContext {
    pub server: NiumaServerClient,
    pub session_token: Arc<RwLock<String>>,
    pub agent_id: String,
    pub identity: AgentIdentity,
    #[cfg(test)]
    pub test_bindings: std::collections::HashMap<String, bindings::PairedDeviceBinding>,
    pub store: TransferStore,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TransferReady {
    pub transfer_id: String,
    pub direction: String,
    pub source_device_id: String,
    pub target_device_id: String,
}

#[derive(Debug, Clone)]
pub struct TransferStore {
    inbound_dir: PathBuf,
    outbound_dir: PathBuf,
}

impl TransferStore {
    /// Open the gateway transfer store under `~/.niuma/transfers`.
    pub fn open() -> Result<Self> {
        let root = paths::transfers_dir()?;
        let inbound_dir = root.join("inbound");
        let outbound_dir = root.join("outbound");
        std::fs::create_dir_all(&inbound_dir)?;
        std::fs::create_dir_all(&outbound_dir)?;
        Ok(Self {
            inbound_dir,
            outbound_dir,
        })
    }

    /// Save an inbound payload after verifying the content-addressed id.
    pub fn save_inbound_ciphertext(&self, transfer_id: &str, body: &[u8]) -> Result<PathBuf> {
        let transfer_hash = safe_hash_id(transfer_id)?;
        anyhow::ensure!(
            sha256_hex(body) == transfer_hash,
            "inbound transfer_id must match payload sha256"
        );
        let directory = self.inbound_dir.join(safe_path_component(&transfer_hash)?);
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("payload.bin");
        std::fs::write(&path, body)?;
        Ok(path)
    }

    /// Save an outbound payload before or after upload to niuma-server.
    pub fn save_outbound_ciphertext(&self, transfer_id: &str, body: &[u8]) -> Result<PathBuf> {
        let transfer_hash = safe_hash_id(transfer_id)?;
        anyhow::ensure!(
            sha256_hex(body) == transfer_hash,
            "outbound transfer_id must match payload sha256"
        );
        let directory = self.outbound_dir.join(safe_path_component(&transfer_hash)?);
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("payload.bin");
        std::fs::write(&path, body)?;
        Ok(path)
    }

    /// Return whether an inbound transfer payload is already cached.
    pub fn has_inbound_payload(&self, transfer_id: &str) -> bool {
        let Ok(transfer_hash) = safe_hash_id(transfer_id) else {
            return false;
        };
        let path = self
            .inbound_dir
            .join(safe_path_component(&transfer_hash).unwrap_or_default())
            .join("payload.bin");
        let Ok(body) = std::fs::read(path) else {
            return false;
        };
        sha256_hex(&body) == transfer_hash
    }

    /// Read an inbound encrypted transfer body after validating its content hash.
    pub fn inbound_ciphertext(&self, transfer_id: &str) -> Result<Vec<u8>> {
        let transfer_hash = safe_hash_id(transfer_id)?;
        let directory = self.inbound_dir.join(safe_path_component(&transfer_hash)?);
        let payload_path = directory.join("payload.bin");
        let body = std::fs::read(&payload_path)
            .with_context(|| format!("transfer payload not found: {transfer_id}"))?;
        anyhow::ensure!(
            sha256_hex(&body) == transfer_hash,
            "inbound transfer ciphertext sha256 mismatch"
        );
        Ok(body)
    }

    /// Expose decrypted transfer bytes as a Codex-readable local file.
    pub fn materialize_inbound_file(
        &self,
        transfer_id: &str,
        file_name: &str,
        plaintext: &[u8],
    ) -> Result<PathBuf> {
        let transfer_hash = safe_hash_id(transfer_id)?;
        let directory = self.inbound_dir.join(safe_path_component(&transfer_hash)?);
        let files_dir = directory.join("files");
        std::fs::create_dir_all(&files_dir)?;
        let file_path = files_dir.join(safe_filename(file_name)?);
        std::fs::write(&file_path, plaintext)?;
        Ok(file_path)
    }
}

impl TransferContext {
    /// Download an iOS-originated transfer if needed and return a local file path.
    pub async fn materialize_inbound_file(
        &self,
        transfer_id: &str,
        source_device_id: &str,
        file_name: &str,
    ) -> Result<PathBuf> {
        if !self.store.has_inbound_payload(transfer_id) {
            let session_token = self.session_token.read().await.clone();
            let body = self
                .server
                .download_transfer(transfer_id, &self.agent_id, &session_token)
                .await?;
            self.store.save_inbound_ciphertext(transfer_id, &body)?;
            let ack = self
                .server
                .ack_transfer(transfer_id, &self.agent_id, &session_token)
                .await?;
            if !ack.acknowledged {
                warn!("server did not acknowledge inbound transfer ack transfer_id={transfer_id}");
            }
        }
        let ciphertext = self.store.inbound_ciphertext(transfer_id)?;
        let plaintext = self.decrypt_transfer_body(source_device_id, &ciphertext)?;
        self.store
            .materialize_inbound_file(transfer_id, file_name, &plaintext)
    }

    /// Proactively cache a server `transfer_ready` payload for later task input mapping.
    pub async fn handle_transfer_ready(&self, ready: &TransferReady) -> Result<()> {
        if ready.direction != "ios_to_agent" || ready.target_device_id != self.agent_id {
            return Ok(());
        }
        anyhow::ensure!(
            !ready.source_device_id.is_empty(),
            "transfer_ready missing source_device_id"
        );
        self.materialize_inbound_file(
            &ready.transfer_id,
            &ready.source_device_id,
            &ready.transfer_id,
        )
        .await?;
        Ok(())
    }

    /// Upload inline file bytes from Codex output and return compact `file_ref`.
    pub async fn upload_inline_file_part(
        &self,
        target_device_id: &str,
        part: &Value,
    ) -> Result<Value> {
        let body =
            file_bytes_from_part(part).context("inline file part missing decodable data_url")?;
        let (transfer_id, body_len) = self.upload_outbound_body(target_device_id, body).await?;
        Ok(file_transfer_ref(part, &transfer_id, body_len as i64))
    }

    /// Upload a generated text attachment and return its transfer-backed `file_ref`.
    pub async fn upload_text_attachment(
        &self,
        target_device_id: &str,
        body: String,
        file_name: &str,
    ) -> Result<Value> {
        let (transfer_id, body_len) = self
            .upload_outbound_body(target_device_id, body.into_bytes())
            .await?;
        Ok(text_transfer_ref(file_name, &transfer_id, body_len as i64))
    }

    /// Upload a JSON bundle and return the content-addressed transfer id.
    pub async fn upload_json_attachment(
        &self,
        target_device_id: &str,
        body: String,
    ) -> Result<(String, usize)> {
        self.upload_outbound_body(target_device_id, body.into_bytes())
            .await
    }

    /// Store and upload one desktop-originated transfer body by content hash.
    async fn upload_outbound_body(
        &self,
        target_device_id: &str,
        body: Vec<u8>,
    ) -> Result<(String, usize)> {
        let encrypted_body = self.encrypt_transfer_body(target_device_id, &body)?;
        let transfer_id = sha256_hex(&encrypted_body);
        let body_len = encrypted_body.len();
        let session_token = self.session_token.read().await.clone();
        let ensured = self
            .server
            .ensure_transfer(
                &transfer_id,
                &self.agent_id,
                target_device_id,
                "agent_to_ios",
                body_len,
                &session_token,
            )
            .await?;
        anyhow::ensure!(
            ensured.transfer_id == transfer_id,
            "server returned a mismatched transfer_id"
        );
        let _expires_at = ensured.expires_at;
        self.store
            .save_outbound_ciphertext(&transfer_id, &encrypted_body)?;
        if ensured.needs_upload {
            let uploaded = self
                .server
                .upload_transfer(&transfer_id, encrypted_body, &self.agent_id, &session_token)
                .await?;
            let _expires_at = uploaded.expires_at;
            if !uploaded.uploaded {
                warn!("server returned uploaded=false for transfer_id={transfer_id}");
            }
        }
        Ok((transfer_id, body_len))
    }

    fn encrypt_transfer_body(&self, target_device_id: &str, body: &[u8]) -> Result<Vec<u8>> {
        let binding = self.binding_for_device(target_device_id)?;
        let aad = transfer_aad("agent_to_ios", &self.agent_id, target_device_id);
        crypto::encrypt_payload(
            &self.identity.encryption_private_key,
            &binding.ios_encryption_public_key,
            &binding.binding_id,
            crypto::PayloadDirection::AgentToIos,
            body,
            &aad,
        )
        .map(|envelope| envelope.into_bytes())
    }

    fn decrypt_transfer_body(&self, source_device_id: &str, ciphertext: &[u8]) -> Result<Vec<u8>> {
        let binding = self.binding_for_device(source_device_id)?;
        let envelope = std::str::from_utf8(ciphertext).context("transfer envelope is not utf-8")?;
        let aad = transfer_aad("ios_to_agent", source_device_id, &self.agent_id);
        crypto::decrypt_payload(
            &self.identity.encryption_private_key,
            &binding.ios_encryption_public_key,
            &binding.binding_id,
            crypto::PayloadDirection::IosToAgent,
            envelope,
            &aad,
        )
    }

    fn binding_for_device(&self, device_id: &str) -> Result<bindings::PairedDeviceBinding> {
        #[cfg(test)]
        if let Some(binding) = self.test_bindings.get(device_id) {
            return Ok(binding.clone());
        }
        bindings::binding_for_device(device_id)?
            .with_context(|| format!("missing pair binding for device {device_id}"))
    }
}

pub fn transfer_aad(direction: &str, source_device_id: &str, target_device_id: &str) -> Vec<u8> {
    crypto::payload_aad(&[
        ("kind", "transfer".to_string()),
        ("direction", direction.to_string()),
        ("source_device_id", source_device_id.to_string()),
        ("target_device_id", target_device_id.to_string()),
    ])
}

/// Normalize outbound `task_update` payloads before they are sent to iOS.
///
/// Inline files are uploaded as transfer-backed refs first. The resulting
/// message is then capped by moving oversized text into a text-file transfer,
/// keeping only a short preview in the WebSocket frame.
pub async fn normalize_task_update_transfers(
    mut payload: Value,
    context: Option<&TransferContext>,
) -> Value {
    if payload.get("kind").and_then(Value::as_str) != Some("task_update") {
        return payload;
    }
    let Some(context) = context else {
        return payload;
    };
    let Some(device_id) = payload
        .get("device_id")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return payload;
    };
    let Some(ciphertext) = payload
        .get("ciphertext")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return payload;
    };
    if let Some(parts) = decode_content_parts_payload(&ciphertext) {
        let mut rewritten = Vec::new();
        let mut changed = false;
        for part in parts {
            if is_file_change_summary_part(&part) {
                match upload_file_change_bundle(context, &device_id, part.clone()).await {
                    Ok(ref_part) => {
                        rewritten.push(ref_part);
                        changed = true;
                    }
                    Err(err) => {
                        warn!(
                            "file change diff bundle transfer failed target_device_id={device_id}: {err:#}"
                        );
                        rewritten.push(file_change_summary_without_bundle(part));
                        changed = true;
                    }
                }
                continue;
            }
            if is_inline_file_part(&part) {
                match context.upload_inline_file_part(&device_id, &part).await {
                    Ok(ref_part) => rewritten.push(ref_part),
                    Err(err) => {
                        warn!("inline file transfer failed target_device_id={device_id}: {err:#}");
                        rewritten.push(file_placeholder_ref(&part));
                    }
                }
                changed = true;
                continue;
            }
            rewritten.push(part);
        }
        if changed {
            payload["ciphertext"] = json!(encode_content_parts_payload(&rewritten));
        }
    }

    let Some(ciphertext) = payload
        .get("ciphertext")
        .and_then(Value::as_str)
        .map(str::to_string)
    else {
        return payload;
    };
    if ciphertext.len() <= LONG_TEXT_TRANSFER_THRESHOLD_BYTES {
        return payload;
    }
    externalize_long_task_text(payload, context, &device_id, &ciphertext).await
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

pub fn file_bytes_from_part(part: &Value) -> Option<Vec<u8>> {
    let data_url = part.get("data_url")?.as_str()?;
    let comma = data_url.find(',')?;
    let header = data_url[..comma].to_ascii_lowercase();
    if !header.contains(";base64") {
        return None;
    }
    STANDARD
        .decode(data_url[comma + 1..].split_whitespace().collect::<String>())
        .ok()
}

pub fn encode_content_parts_payload(parts: &[Value]) -> String {
    let payload = json!({ "content_parts": parts });
    serde_json::to_string(&payload).unwrap_or_default()
}

pub fn decode_content_parts_payload(ciphertext: &str) -> Option<Vec<Value>> {
    let payload: Value = serde_json::from_str(ciphertext).ok()?;
    payload.get("content_parts")?.as_array().cloned()
}

pub fn is_inline_file_part(part: &Value) -> bool {
    part.get("type").and_then(Value::as_str) == Some("file_ref")
        && part.get("data_url").and_then(Value::as_str).is_some()
}

fn is_file_change_summary_part(part: &Value) -> bool {
    part.get("type").and_then(Value::as_str) == Some("file_change_summary")
        && part.get("diff_bundle").is_some()
}

async fn upload_file_change_bundle(
    context: &TransferContext,
    device_id: &str,
    mut part: Value,
) -> Result<Value> {
    let bundle = part
        .get("diff_bundle")
        .cloned()
        .context("file_change_summary missing diff_bundle")?;
    let body = serde_json::to_string_pretty(&bundle)?;
    let (transfer_id, size_bytes) = context.upload_json_attachment(device_id, body).await?;
    part["transfer_id"] = json!(transfer_id);
    part["size_bytes"] = json!(size_bytes as i64);
    part["mime_type"] = json!("application/json");
    part["file_name"] = json!("diff-bundle.json");
    if let Some(map) = part.as_object_mut() {
        map.remove("diff_bundle");
    }
    Ok(part)
}

fn file_change_summary_without_bundle(mut part: Value) -> Value {
    if let Some(map) = part.as_object_mut() {
        map.remove("diff_bundle");
    }
    part
}

pub fn file_placeholder_ref(part: &Value) -> Value {
    let mut ref_part = json!({
        "type": "file_ref",
        "file_type": normalized_file_type(part),
        "mime_type": string_field(part, "mime_type").unwrap_or_else(|| "application/octet-stream".to_string()),
    });
    for key in ["file_name", "size_bytes", "alt", "transfer_id"] {
        if let Some(value) = part.get(key) {
            ref_part[key] = value.clone();
        }
    }
    ref_part
}

fn file_transfer_ref(part: &Value, transfer_id: &str, fallback_size: i64) -> Value {
    let mut ref_part = file_placeholder_ref(part);
    ref_part["transfer_id"] = json!(transfer_id);
    if ref_part.get("size_bytes").and_then(Value::as_i64).is_none() {
        ref_part["size_bytes"] = json!(fallback_size);
    }
    if let Some(map) = ref_part.as_object_mut() {
        map.remove("data_url");
    }
    ref_part
}

async fn externalize_long_task_text(
    mut payload: Value,
    context: &TransferContext,
    device_id: &str,
    ciphertext: &str,
) -> Value {
    let Some((body, parts)) = long_text_replacement_payload(ciphertext) else {
        return payload;
    };
    let transfer_id = sha256_hex(body.as_bytes());
    let file_name = long_text_file_name(&payload, &transfer_id);
    match context
        .upload_text_attachment(device_id, body, &file_name)
        .await
    {
        Ok(file_part) => {
            payload["ciphertext"] = json!(encode_content_parts_payload(&append_text_file_part(
                parts, file_part,
            )));
            payload
        }
        Err(err) => {
            warn!("long text transfer failed target_device_id={device_id}: {err:#}");
            payload["ciphertext"] = json!(encode_content_parts_payload(&[json!({
                "type": "text",
                "text": format!(
                    "{}\n\n[完整内容上传失败，已截断以保护实时连接。]",
                    preview_text(ciphertext)
                ),
            })]));
            payload
        }
    }
}

fn long_text_replacement_payload(ciphertext: &str) -> Option<(String, Vec<Value>)> {
    if let Some(parts) = decode_content_parts_payload(ciphertext) {
        let body = text_from_content_parts(&parts);
        if body.trim().is_empty() {
            return None;
        }
        let replacement = long_text_replacement_parts(&body, parts);
        return Some((body, replacement));
    }
    Some((
        ciphertext.to_string(),
        vec![json!({
            "type": "text",
            "text": preview_text(ciphertext),
        })],
    ))
}

fn append_text_file_part(mut parts: Vec<Value>, file_part: Value) -> Vec<Value> {
    parts.push(file_part);
    parts
}

fn long_text_replacement_parts(body: &str, parts: Vec<Value>) -> Vec<Value> {
    let mut replacement = vec![json!({
        "type": "text",
        "text": preview_text(body),
    })];
    replacement.extend(parts.into_iter().filter(|part| {
        part.get("type").and_then(Value::as_str) != Some("text")
            && part.get("text").and_then(Value::as_str).is_none()
    }));
    replacement
}

fn text_from_content_parts(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(|part| {
            if part.get("type").and_then(Value::as_str) == Some("text") {
                return part.get("text").and_then(Value::as_str);
            }
            None
        })
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn preview_text(text: &str) -> String {
    let preview = text
        .chars()
        .take(LONG_TEXT_PREVIEW_CHARS)
        .collect::<String>();
    format!("{preview}...")
}

fn long_text_file_name(payload: &Value, transfer_id: &str) -> String {
    let fallback = transfer_id.get(..12).unwrap_or(transfer_id);
    let candidate = payload
        .get("entry_id")
        .or_else(|| payload.get("checkpoint"))
        .or_else(|| payload.get("thread_id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(fallback);
    let safe = safe_path_component(candidate).unwrap_or_else(|_| fallback.to_string());
    format!("message-{safe}.txt")
}

fn text_transfer_ref(file_name: &str, transfer_id: &str, size_bytes: i64) -> Value {
    json!({
        "type": "file_ref",
        "file_type": "file",
        "mime_type": LONG_TEXT_MIME_TYPE,
        "file_name": file_name,
        "size_bytes": size_bytes,
        "transfer_id": transfer_id,
    })
}

pub fn normalized_file_type(part: &Value) -> String {
    if let Some(file_type) = string_field(part, "file_type") {
        return file_type;
    }
    match string_field(part, "mime_type") {
        Some(mime_type) if mime_type.starts_with("image/") => "image".to_string(),
        Some(mime_type) if mime_type.starts_with("video/") => "video".to_string(),
        _ => "file".to_string(),
    }
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_string())
}

fn safe_hash_id(value: &str) -> Result<String> {
    let normalized = value.to_ascii_lowercase();
    anyhow::ensure!(
        normalized.len() == 64 && normalized.chars().all(|char| char.is_ascii_hexdigit()),
        "transfer_id must be a hex sha256 digest"
    );
    Ok(normalized)
}

fn safe_path_component(value: &str) -> Result<String> {
    let cleaned = value
        .chars()
        .map(|char| {
            if char.is_ascii_alphanumeric() || matches!(char, '.' | '_' | '-') {
                char
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches(['.', '-'])
        .chars()
        .take(128)
        .collect::<String>();
    anyhow::ensure!(
        !cleaned.is_empty(),
        "path component is empty after sanitization"
    );
    Ok(cleaned)
}

fn safe_filename(value: &str) -> Result<String> {
    let name = Path::new(value)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file.bin");
    safe_path_component(name).or_else(|_| Ok("file.bin".to_string()))
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    use axum::{
        Json, Router,
        body::Bytes,
        extract::{Path as AxumPath, State},
        routing::{post, put},
    };
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;

    #[derive(Clone, Default)]
    struct UploadState {
        bodies: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    }

    fn test_transfer_store() -> TransferStore {
        let root =
            std::env::temp_dir().join(format!("niuma-cli-transfer-{}", uuid::Uuid::new_v4()));
        let inbound_dir = root.join("inbound");
        let outbound_dir = root.join("outbound");
        std::fs::create_dir_all(&inbound_dir).expect("create inbound dir");
        std::fs::create_dir_all(&outbound_dir).expect("create outbound dir");
        TransferStore {
            inbound_dir,
            outbound_dir,
        }
    }

    async fn test_transfer_context(state: UploadState) -> TransferContext {
        async fn ensure(AxumPath(transfer_id): AxumPath<String>) -> Json<Value> {
            Json(json!({
                "transfer_id": transfer_id,
                "expires_at": 4_102_444_800i64,
                "needs_upload": true,
            }))
        }

        async fn upload(
            AxumPath(transfer_id): AxumPath<String>,
            State(state): State<UploadState>,
            body: Bytes,
        ) -> Json<Value> {
            state
                .bodies
                .lock()
                .expect("lock uploads")
                .insert(transfer_id, body.to_vec());
            Json(json!({
                "uploaded": true,
                "expires_at": 4_102_444_800i64,
            }))
        }

        let app = Router::new()
            .route("/transfers/{transfer_id}/ensure", post(ensure))
            .route("/transfers/{transfer_id}", put(upload))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let address = listener.local_addr().expect("read test server address");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("serve test server");
        });

        let (agent_signing_private_key, agent_signing_public_key) =
            crate::crypto::generate_ed25519_keypair();
        let (agent_encryption_private_key, agent_encryption_public_key) =
            crate::crypto::generate_x25519_keypair();
        let (_, ios_encryption_public_key) = crate::crypto::generate_x25519_keypair();
        let identity = AgentIdentity {
            agent_id: "agent-device".to_string(),
            device_name: "Test Gateway".to_string(),
            os_type: "darwin".to_string(),
            signing_private_key: agent_signing_private_key,
            signing_public_key: agent_signing_public_key,
            signing_key_fingerprint: "signing".to_string(),
            encryption_private_key: agent_encryption_private_key,
            encryption_public_key: agent_encryption_public_key,
            encryption_key_fingerprint: "encryption".to_string(),
        };
        let mut test_bindings = std::collections::HashMap::new();
        test_bindings.insert(
            "ios-device".to_string(),
            bindings::PairedDeviceBinding {
                binding_id: "binding-test".to_string(),
                device_id: "ios-device".to_string(),
                agent_id: "agent-device".to_string(),
                ios_encryption_public_key,
                paired_at: 0,
            },
        );

        TransferContext {
            server: NiumaServerClient::new(&format!("http://{address}"))
                .expect("create server client"),
            session_token: Arc::new(RwLock::new("session-token".to_string())),
            agent_id: "agent-device".to_string(),
            identity,
            test_bindings,
            store: test_transfer_store(),
        }
    }

    #[test]
    fn preview_text_keeps_utf8_boundaries() {
        let preview = preview_text(&format!("{}{}", "你".repeat(120), "tail"));
        assert_eq!(preview.chars().count(), LONG_TEXT_PREVIEW_CHARS + 3);
        assert!(preview.ends_with("..."));
    }

    #[test]
    fn content_parts_payload_uses_mobile_json_contract() {
        let encoded = encode_content_parts_payload(&[json!({
            "type": "text",
            "text": "hello"
        })]);

        assert!(encoded.starts_with("{"));
        assert_eq!(
            decode_content_parts_payload(&encoded)
                .expect("content parts")
                .len(),
            1
        );
        assert!(
            decode_content_parts_payload("eyJjb250ZW50X3BhcnRzIjpbXX0=").is_none(),
            "legacy base64 content-parts wrappers must not be accepted"
        );
    }

    #[tokio::test]
    async fn long_plain_task_update_is_externalized_as_text_file() {
        let uploads = UploadState::default();
        let context = test_transfer_context(uploads.clone()).await;
        let body = format!("{}{}", "日志".repeat(80_000), "\nend");
        assert!(body.len() > LONG_TEXT_TRANSFER_THRESHOLD_BYTES);

        let payload = json!({
            "kind": "task_update",
            "device_id": "ios-device",
            "thread_id": "thread-1",
            "entry_id": "turn:1:item:2",
            "ciphertext": body,
        });
        let normalized = normalize_task_update_transfers(payload, Some(&context)).await;
        let ciphertext = normalized
            .get("ciphertext")
            .and_then(Value::as_str)
            .expect("normalized ciphertext");
        assert!(ciphertext.len() < LONG_TEXT_TRANSFER_THRESHOLD_BYTES);

        let parts = decode_content_parts_payload(ciphertext).expect("content parts payload");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].get("type").and_then(Value::as_str), Some("text"));
        assert!(
            parts[0]
                .get("text")
                .and_then(Value::as_str)
                .expect("preview")
                .ends_with("...")
        );

        let file_part = &parts[1];
        let transfer_id = file_part
            .get("transfer_id")
            .and_then(Value::as_str)
            .expect("transfer id");
        assert_eq!(
            file_part.get("type").and_then(Value::as_str),
            Some("file_ref")
        );
        assert_eq!(
            file_part.get("mime_type").and_then(Value::as_str),
            Some(LONG_TEXT_MIME_TYPE)
        );
        assert_eq!(
            file_part.get("file_name").and_then(Value::as_str),
            Some("message-turn-1-item-2.txt")
        );

        let uploaded = uploads.bodies.lock().expect("lock uploads");
        let uploaded_body = uploaded.get(transfer_id).expect("uploaded text body");
        assert_eq!(sha256_hex(uploaded_body), transfer_id);
        assert_ne!(uploaded_body, body.as_bytes());
        assert!(
            std::str::from_utf8(uploaded_body)
                .expect("encrypted transfer envelope")
                .contains("\"alg\":\"X25519-HKDF-SHA256-CHACHA20POLY1305\"")
        );
    }
}
