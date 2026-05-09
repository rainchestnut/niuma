//! Mobile task and thread replay projection for Codex app-server.
//!
//! This is the Rust replacement for the Python plugin's main Codex bridge:
//! mobile payloads are converted into Codex-native input items, and Codex
//! thread history is projected back into the server's existing `task_update`
//! wire events.

use anyhow::{Context, Result};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::OnceLock;
use url::Url;

use crate::codex_app_server::CodexAppServerClient;
use crate::diff_summary;
use crate::identity::AgentIdentity;
use crate::metadata::CodexWorkspaceStore;
use crate::thread_status::normalize_thread_status;
use crate::transfers::{
    TransferContext, encode_content_parts_payload, file_bytes_from_part, sha256_hex,
};
use crate::{bindings, crypto};

const CONVERSATION_PROJECT_ID: &str = "__conversation__";

#[derive(Debug, Clone, Deserialize)]
pub struct TaskStartInbound {
    pub device_id: String,
    pub agent_id: String,
    pub project_id: String,
    pub thread_id: Option<String>,
    pub ciphertext: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub approval_policy: Option<String>,
    pub approvals_reviewer: Option<String>,
    pub sandbox_mode: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResumeThreadInbound {
    pub device_id: String,
    pub thread_id: String,
    pub cursor: i64,
    pub checkpoint: Option<String>,
}

#[derive(Debug, Clone)]
struct ThreadContext {
    thread_id: String,
    project_id: Option<String>,
    cwd: Option<String>,
    title: String,
    status: String,
    updated_at: Option<f64>,
}

#[derive(Debug, Clone)]
struct ThreadEntry {
    entry_id: String,
    role: String,
    text: String,
    item_type: String,
    phase: Option<String>,
}

#[derive(Debug, Clone)]
struct ThreadEvent {
    thread_id: String,
    seq: i64,
    ciphertext: String,
    checkpoint: Option<String>,
    role: String,
    item_type: String,
    phase: Option<String>,
    project_id: Option<String>,
    entry_id: Option<String>,
    created_at: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TaskProjectRoute {
    Workspace { project_id: String, cwd: String },
    // Codex app-server treats an omitted cwd as a projectless conversation.
    Conversation,
}

impl TaskProjectRoute {
    fn project_id(&self) -> &str {
        match self {
            Self::Workspace { project_id, .. } => project_id,
            Self::Conversation => CONVERSATION_PROJECT_ID,
        }
    }

    fn cwd(&self) -> Option<&str> {
        match self {
            Self::Workspace { cwd, .. } => Some(cwd),
            Self::Conversation => None,
        }
    }
}

pub struct TaskRuntime {
    identity: AgentIdentity,
    app_server: CodexAppServerClient,
    workspace_store: CodexWorkspaceStore,
    transfer_context: Option<TransferContext>,
}

impl TaskRuntime {
    /// Create a task runtime around the long-lived Codex app-server client.
    pub fn new(
        identity: AgentIdentity,
        app_server: CodexAppServerClient,
        transfer_context: Option<TransferContext>,
    ) -> Self {
        Self {
            identity,
            app_server,
            workspace_store: CodexWorkspaceStore::new(),
            transfer_context,
        }
    }

    /// Start or resume a Codex thread, submit the mobile turn, and emit sync rows.
    pub async fn start_task_messages(&self, message: TaskStartInbound) -> Result<Vec<Value>> {
        let route = self.project_route_for_task(&message.project_id)?;
        let cwd = route.cwd();
        let requested_thread_id = message.thread_id.as_deref();
        let thread_payload = match requested_thread_id {
            Some(thread_id) => match self
                .app_server
                .resume_thread_payload(
                    thread_id,
                    cwd,
                    message.approval_policy.as_deref(),
                    message.approvals_reviewer.as_deref(),
                    message.sandbox_mode.as_deref(),
                )
                .await
            {
                Ok(thread) => thread,
                Err(_) => {
                    self.app_server
                        .start_thread_payload(
                            cwd,
                            message.approval_policy.as_deref(),
                            message.approvals_reviewer.as_deref(),
                            message.sandbox_mode.as_deref(),
                        )
                        .await?
                }
            },
            None => {
                self.app_server
                    .start_thread_payload(
                        cwd,
                        message.approval_policy.as_deref(),
                        message.approvals_reviewer.as_deref(),
                        message.sandbox_mode.as_deref(),
                    )
                    .await?
            }
        };
        let thread_id = string_field(&thread_payload, "id").context("Codex thread missing id")?;
        let project_id = route.project_id().to_string();
        let plaintext = self.decrypt_mobile_task_payload(&message)?;
        let input_items = self
            .decode_mobile_payload_to_codex_input(&message.device_id, &plaintext)
            .await;
        let turn_payload = self
            .app_server
            .start_turn_payload(
                &thread_id,
                input_items,
                message.model.as_deref(),
                message.effort.as_deref(),
                message.approval_policy.as_deref(),
                message.approvals_reviewer.as_deref(),
                message.sandbox_mode.as_deref(),
            )
            .await?;
        let turn_id = string_field(&turn_payload, "id").context("Codex turn missing id")?;
        let replay_payload = self
            .app_server
            .read_thread_payload(&thread_id, true)
            .await
            .unwrap_or(thread_payload);
        let context = thread_context(&replay_payload, Some(project_id.clone()));
        let mut messages = metadata_messages_for_thread(&context);
        if should_emit_started_user_event(requested_thread_id) {
            let turn_checkpoint = format!("turn:{turn_id}");
            let mut user_event = replay_events_from_thread(&replay_payload, 0, &context)
                .into_iter()
                .find(|event| {
                    event.role == "user" && event.checkpoint.as_deref() == Some(&turn_checkpoint)
                })
                .unwrap_or_else(|| fallback_started_turn_event(&context, &turn_id, &plaintext));
            user_event.ciphertext = plaintext;
            messages.push(user_event.to_wire(&message.device_id));
        }
        Ok(messages)
    }

    /// Replay thread entries after the mobile cursor.
    pub async fn resume_thread_messages(&self, message: ResumeThreadInbound) -> Result<Vec<Value>> {
        let thread_payload = self
            .app_server
            .resume_thread_payload(&message.thread_id, None, None, None, None)
            .await
            .unwrap_or_else(|_| json!({ "id": message.thread_id }));
        let replay_payload = self
            .app_server
            .read_thread_payload(&message.thread_id, true)
            .await
            .unwrap_or(thread_payload);
        let context = thread_context(&replay_payload, None);
        Ok(
            replay_events_from_thread(&replay_payload, message.cursor, &context)
                .into_iter()
                .map(|event| event.to_wire(&message.device_id))
                .collect(),
        )
    }

    fn project_route_for_task(&self, project_id: &str) -> Result<TaskProjectRoute> {
        let project = if project_id == CONVERSATION_PROJECT_ID {
            None
        } else {
            self.workspace_store.project_for_id(project_id)
        };
        task_project_route(project_id, project)
    }

    fn decrypt_mobile_task_payload(&self, message: &TaskStartInbound) -> Result<String> {
        let binding = bindings::binding_for_device(&message.device_id)?
            .with_context(|| format!("missing pair binding for device {}", message.device_id))?;
        let aad = task_start_aad(message);
        let plaintext = crypto::decrypt_payload(
            &self.identity.encryption_private_key,
            &binding.ios_encryption_public_key,
            &binding.binding_id,
            crypto::PayloadDirection::IosToAgent,
            &message.ciphertext,
            &aad,
        )?;
        String::from_utf8(plaintext).context("task_start plaintext is not utf-8")
    }

    async fn decode_mobile_payload_to_codex_input(
        &self,
        source_device_id: &str,
        payload: &str,
    ) -> Vec<Value> {
        self.content_parts_to_codex_input(source_device_id, payload)
            .await
            .unwrap_or_else(|| {
                vec![json!({
                    "type": "text",
                    "text": payload,
                })]
            })
    }

    async fn content_parts_to_codex_input(
        &self,
        source_device_id: &str,
        payload: &str,
    ) -> Option<Vec<Value>> {
        let decoded: Value = serde_json::from_str(payload).ok()?;
        let parts = decoded.get("content_parts")?.as_array()?;
        let mut input_items = Vec::new();
        let mut attachment_lines = Vec::new();
        for part in parts {
            let part_type = string_field(part, "type").or_else(|| string_field(part, "kind"));
            match part_type.as_deref() {
                Some("text") => {
                    if let Some(text) = string_field(part, "text") {
                        input_items.push(json!({ "type": "text", "text": text }));
                    }
                }
                Some("file_ref") => {
                    if let Some(item) = self.file_part_to_codex_input(part, source_device_id).await
                    {
                        input_items.push(item);
                    } else {
                        attachment_lines.push(file_part_attachment_line(part));
                    }
                }
                _ => {}
            }
        }
        if !attachment_lines.is_empty() {
            input_items.push(json!({
                "type": "text",
                "text": format!("Files mentioned by user:\n{}", attachment_lines.join("\n")),
            }));
        }
        if input_items.is_empty() {
            None
        } else {
            Some(input_items)
        }
    }

    async fn file_part_to_codex_input(
        &self,
        part: &Value,
        source_device_id: &str,
    ) -> Option<Value> {
        let path = if let Some(path) =
            string_field(part, "local_path").or_else(|| string_field(part, "path"))
        {
            path
        } else if let (Some(context), Some(transfer_id)) = (
            self.transfer_context.as_ref(),
            string_field(part, "transfer_id"),
        ) {
            let name = string_field(part, "file_name")
                .or_else(|| string_field(part, "alt"))
                .unwrap_or_else(|| transfer_id.clone());
            context
                .materialize_inbound_file(&transfer_id, source_device_id, &name)
                .await
                .ok()?
                .to_string_lossy()
                .into_owned()
        } else {
            return None;
        };

        if file_part_type(part) == "image" {
            let mut item = json!({
                "type": "localImage",
                "path": path,
            });
            if let Some(file_name) = string_field(part, "file_name") {
                item["file_name"] = json!(file_name);
            }
            if let Some(alt) = string_field(part, "alt") {
                item["alt"] = json!(alt);
            }
            return Some(item);
        }
        Some(json!({
            "type": "text",
            "text": format!("Attached file available at local path:\n{path}"),
        }))
    }
}

fn task_project_route(
    project_id: &str,
    project: Option<crate::metadata::ProjectSummary>,
) -> Result<TaskProjectRoute> {
    if project_id == CONVERSATION_PROJECT_ID {
        return Ok(TaskProjectRoute::Conversation);
    }
    let project = project.with_context(|| format!("unknown project_id={project_id}"))?;
    let cwd = project
        .cwd
        .with_context(|| format!("project_id={project_id} has no workspace cwd"))?;
    Ok(TaskProjectRoute::Workspace {
        project_id: project.project_id,
        cwd,
    })
}

pub fn task_start_aad(message: &TaskStartInbound) -> Vec<u8> {
    crypto::payload_aad(&[
        ("kind", "task_start".to_string()),
        ("device_id", message.device_id.clone()),
        ("agent_id", message.agent_id.clone()),
        ("project_id", message.project_id.clone()),
        ("thread_id", message.thread_id.clone().unwrap_or_default()),
    ])
}

impl ThreadEvent {
    fn to_wire(self, device_id: &str) -> Value {
        json!({
            "kind": "task_update",
            "device_id": device_id,
            "thread_id": self.thread_id,
            "seq": self.seq,
            "ciphertext": self.ciphertext,
            "checkpoint": self.checkpoint,
            "role": self.role,
            "type": self.item_type,
            "phase": self.phase,
            "project_id": self.project_id,
            "entry_id": self.entry_id,
            "created_at": self.created_at,
        })
    }
}

fn metadata_messages_for_thread(context: &ThreadContext) -> Vec<Value> {
    let Some(project_id) = context.project_id.as_ref() else {
        return Vec::new();
    };
    vec![json!({
        "kind": "thread_sync",
        "thread_id": context.thread_id,
        "project_id": project_id,
        "title": context.title,
        "status": context.status,
        "last_checkpoint_seen": null,
        "updated_at": context.updated_at,
    })]
}

fn thread_context(payload: &Value, project_id: Option<String>) -> ThreadContext {
    let thread_id = string_field(payload, "id").unwrap_or_else(|| "unknown-thread".to_string());
    ThreadContext {
        title: string_field(payload, "name")
            .or_else(|| string_field(payload, "preview"))
            .unwrap_or_else(|| thread_id.clone()),
        status: normalize_thread_status(payload.get("status"), bool_field(payload, "archived")),
        updated_at: number_field(payload, "updatedAt"),
        project_id,
        cwd: string_field(payload, "cwd"),
        thread_id,
    }
}

fn replay_events_from_thread(
    thread: &Value,
    cursor: i64,
    context: &ThreadContext,
) -> Vec<ThreadEvent> {
    let mut events = Vec::new();
    let mut seq = 0;
    let turns = thread
        .get("turns")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for turn in turns {
        let turn_id = string_field(&turn, "id").unwrap_or_else(|| "turn".to_string());
        let mut entries = extract_turn_entries(&turn, context.cwd.as_deref());
        if entries.is_empty() {
            if let Some(preview) = string_field(&turn, "preview") {
                entries.push(ThreadEntry {
                    entry_id: turn_id.clone(),
                    role: "user".to_string(),
                    text: preview,
                    item_type: "userMessage".to_string(),
                    phase: None,
                });
            }
        }
        let created_at = number_field(&turn, "startedAt").or(context.updated_at);
        for entry in entries {
            seq += 1;
            if seq <= cursor {
                continue;
            }
            events.push(ThreadEvent {
                thread_id: context.thread_id.clone(),
                seq,
                ciphertext: entry.text,
                checkpoint: Some(format!("turn:{turn_id}")),
                role: entry.role,
                item_type: entry.item_type,
                phase: entry.phase,
                project_id: context.project_id.clone(),
                entry_id: Some(entry.entry_id),
                created_at,
            });
        }
    }
    events
}

fn extract_turn_entries(turn: &Value, cwd: Option<&str>) -> Vec<ThreadEntry> {
    let mut entries = Vec::new();
    let turn_id = string_field(turn, "id").unwrap_or_else(|| "turn".to_string());
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let final_file_change_group = final_file_change_group(&items);
    for (item_index, item) in items.iter().enumerate() {
        if let Some((start, end, final_index)) = final_file_change_group {
            if item_index == start {
                if let Some(part) = diff_summary::file_change_summary_part(
                    &turn_id,
                    &final_answer_entry_id(&items[final_index], &turn_id, final_index),
                    &items[start..end],
                    cwd,
                ) {
                    entries.push(ThreadEntry {
                        entry_id: final_file_change_entry_id(&turn_id, &items[start..end]),
                        role: "assistant".to_string(),
                        text: encode_content_parts_payload(&[part]),
                        item_type: "fileChange".to_string(),
                        phase: Some("final_answer_file_change".to_string()),
                    });
                }
            }
            if (start..end).contains(&item_index) {
                continue;
            }
        }
        let text = item_mobile_ciphertext(item);
        if text.is_empty() {
            continue;
        }
        let role = item_role(item);
        let offset = entries.len();
        entries.push(ThreadEntry {
            entry_id: item_entry_id(item, &turn_id, offset),
            role,
            text,
            item_type: item_type(item),
            phase: item_phase(item),
        });
    }
    if let Some(input_text) = turn_input_text(turn, &entries) {
        if !entries
            .iter()
            .any(|entry| entry.role == "user" && same_text(&entry.text, &input_text))
        {
            entries.insert(
                0,
                ThreadEntry {
                    entry_id: fallback_input_entry_id(&turn_id),
                    role: "user".to_string(),
                    text: input_text,
                    item_type: "userMessage".to_string(),
                    phase: None,
                },
            );
        }
    }
    entries
}

fn final_file_change_group(items: &[Value]) -> Option<(usize, usize, usize)> {
    let final_index = items.iter().rposition(is_final_answer_item)?;
    let last_change_index = items[..final_index]
        .iter()
        .rposition(is_completed_file_change)?;
    let mut start = last_change_index;
    while start > 0 && is_completed_file_change(&items[start - 1]) {
        start -= 1;
    }
    let mut end = last_change_index + 1;
    while end < final_index && is_completed_file_change(&items[end]) {
        end += 1;
    }
    Some((start, end, final_index))
}

fn is_final_answer_item(item: &Value) -> bool {
    item_type(item) == "agentMessage" && item_phase(item).as_deref() == Some("final_answer")
}

fn is_completed_file_change(item: &Value) -> bool {
    item_type(item) == "fileChange"
        && string_field(item, "status").as_deref() == Some("completed")
        && item
            .get("changes")
            .and_then(Value::as_array)
            .is_some_and(|changes| !changes.is_empty())
}

fn final_answer_entry_id(item: &Value, turn_id: &str, item_index: usize) -> String {
    item_entry_id(item, turn_id, item_index)
}

fn final_file_change_entry_id(turn_id: &str, items: &[Value]) -> String {
    let ids = items
        .iter()
        .filter_map(|item| string_field(item, "id").or_else(|| string_field(item, "itemId")))
        .collect::<Vec<_>>();
    if ids.is_empty() {
        format!("{turn_id}-file-change-summary")
    } else {
        format!("{turn_id}-file-change-summary-{}", ids.join("-"))
    }
}

fn fallback_started_turn_event(
    context: &ThreadContext,
    turn_id: &str,
    mobile_payload: &str,
) -> ThreadEvent {
    ThreadEvent {
        thread_id: context.thread_id.clone(),
        seq: 1,
        ciphertext: mobile_payload.to_string(),
        checkpoint: Some(format!("turn:{turn_id}")),
        role: "user".to_string(),
        item_type: "userMessage".to_string(),
        phase: None,
        project_id: context.project_id.clone(),
        entry_id: Some(turn_id.to_string()),
        created_at: context.updated_at,
    }
}

fn should_emit_started_user_event(requested_thread_id: Option<&str>) -> bool {
    requested_thread_id.is_none()
}

fn file_part_attachment_line(part: &Value) -> String {
    let name = string_field(part, "file_name")
        .or_else(|| string_field(part, "alt"))
        .or_else(|| string_field(part, "transfer_id"))
        .unwrap_or_else(|| "attachment".to_string());
    let mut details = vec![format!("file_type={}", file_part_type(part))];
    if let Some(mime_type) = string_field(part, "mime_type") {
        details.push(mime_type);
    }
    if let Some(size_bytes) = number_field(part, "size_bytes") {
        details.push(format!("{} bytes", size_bytes as i64));
    }
    if let Some(transfer_id) = string_field(part, "transfer_id") {
        details.push(format!("transfer_id={transfer_id}"));
    }
    format!("- {name} ({})", details.join(", "))
}

fn file_part_type(part: &Value) -> String {
    if let Some(file_type) = string_field(part, "file_type") {
        return file_type;
    }
    match string_field(part, "mime_type") {
        Some(mime_type) if mime_type.starts_with("image/") => "image".to_string(),
        Some(mime_type) if mime_type.starts_with("video/") => "video".to_string(),
        _ => "file".to_string(),
    }
}

fn turn_input_text(turn: &Value, entries: &[ThreadEntry]) -> Option<String> {
    if let Some(entry) = entries
        .iter()
        .find(|entry| entry.role == "user" && !entry.text.trim().is_empty())
    {
        return Some(entry.text.trim().to_string());
    }
    for key in ["input", "input_items", "inputItems"] {
        if let Some(text) = input_value_text(turn.get(key)) {
            return Some(text);
        }
    }
    None
}

fn input_value_text(value: Option<&Value>) -> Option<String> {
    let value = value?;
    match value {
        Value::String(value) => non_empty(value),
        Value::Array(items) => {
            let lines: Vec<String> = items
                .iter()
                .filter_map(input_item_display_text)
                .filter(|line| !line.is_empty())
                .collect();
            non_empty(&lines.join("\n"))
        }
        Value::Object(map) => {
            if let Some(Value::Array(items)) = map.get("items").or_else(|| map.get("content")) {
                let lines: Vec<String> = items.iter().filter_map(input_item_display_text).collect();
                return non_empty(&lines.join("\n"));
            }
            non_empty(&item_text(value))
        }
        _ => None,
    }
}

fn input_item_display_text(value: &Value) -> Option<String> {
    let item_type = string_field(value, "type").unwrap_or_default();
    if item_type == "text" {
        return string_field(value, "text");
    }
    if matches!(
        item_type.as_str(),
        "image" | "input_image" | "localImage" | "local_image"
    ) {
        let name = string_field(value, "file_name")
            .or_else(|| string_field(value, "alt"))
            .unwrap_or_else(|| "image".to_string());
        return Some(format!("[图片: {name}]"));
    }
    None
}

fn item_mobile_ciphertext(item: &Value) -> String {
    let text = item_text(item);
    let mut parts = content_parts_from_text(&text);
    collect_item_files(item, &mut parts);
    collect_mcp_tool_result_files(item, &mut parts);
    if parts.iter().any(crate::transfers::is_inline_file_part) {
        encode_content_parts_payload(&parts)
    } else {
        text
    }
}

fn content_parts_from_text(text: &str) -> Vec<Value> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let Ok(pattern) =
        Regex::new(r"!\[(?P<alt>[^\]]*)\]\((?P<url>(?:data:image/[^)]+|file://[^)]+|/[^)]+))\)")
    else {
        return vec![json!({ "type": "text", "text": text })];
    };
    let mut parts = Vec::new();
    let mut cursor = 0;
    for capture in pattern.captures_iter(text) {
        let Some(full) = capture.get(0) else {
            continue;
        };
        let prefix = text[cursor..full.start()].trim();
        if !prefix.is_empty() {
            parts.push(json!({ "type": "text", "text": prefix }));
        }
        let url = capture
            .name("url")
            .map(|value| value.as_str())
            .unwrap_or("");
        let alt = capture.name("alt").map(|value| value.as_str());
        match file_part_from_markdown_url(url, alt) {
            Some(part) => parts.push(part),
            None => parts.push(json!({ "type": "text", "text": full.as_str() })),
        }
        cursor = full.end();
    }
    let suffix = text[cursor..].trim();
    if !suffix.is_empty() {
        parts.push(json!({ "type": "text", "text": suffix }));
    }
    if parts.is_empty() && text.trim_start().starts_with("data:image/") {
        if let Some(part) = file_part_from_data_url(text.trim(), None, None) {
            parts.push(part);
        }
    }
    if parts.is_empty() {
        parts.push(json!({ "type": "text", "text": text }));
    }
    parts
}

fn item_text(item: &Value) -> String {
    let mut texts = Vec::new();
    collect_item_text(item, &mut texts);
    dedupe_texts(texts).join("\n")
}

fn collect_item_files(value: &Value, parts: &mut Vec<Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_item_files(item, parts);
            }
        }
        Value::Object(map) => {
            let item_type = map.get("type").and_then(Value::as_str).unwrap_or_default();
            if matches!(item_type, "localImage" | "local_image") {
                if let Some(path_value) = map
                    .get("path")
                    .or_else(|| map.get("file_path"))
                    .or_else(|| map.get("url"))
                    .and_then(Value::as_str)
                {
                    if let Some(path) = local_markdown_image_path(path_value).or_else(|| {
                        Some(std::path::PathBuf::from(path_value)).filter(|path| path.is_file())
                    }) {
                        if let Some(part) =
                            file_part_from_local_path(&path, string_field(value, "alt").as_deref())
                        {
                            parts.push(part);
                            return;
                        }
                    }
                }
            }
            if matches!(item_type, "input_image" | "image" | "output_image") {
                let data_url = map
                    .get("image_url")
                    .or_else(|| map.get("url"))
                    .or_else(|| map.get("data_url"))
                    .and_then(Value::as_str);
                if let Some(data_url) = data_url.filter(|value| value.starts_with("data:image/")) {
                    if let Some(part) = file_part_from_data_url(
                        data_url,
                        string_field(value, "file_name")
                            .or_else(|| string_field(value, "filename"))
                            .or_else(|| string_field(value, "name"))
                            .as_deref(),
                        string_field(value, "alt").as_deref(),
                    ) {
                        parts.push(part);
                        return;
                    }
                }
                let data = map.get("data").and_then(Value::as_str);
                let mime_type = map
                    .get("mimeType")
                    .or_else(|| map.get("mime_type"))
                    .and_then(Value::as_str);
                if let (Some(data), Some(mime_type)) = (data, mime_type) {
                    if mime_type.starts_with("image/") {
                        let data_url = format!("data:{mime_type};base64,{data}");
                        if let Some(part) = file_part_from_data_url(
                            &data_url,
                            None,
                            string_field(value, "alt").as_deref(),
                        ) {
                            parts.push(part);
                            return;
                        }
                    }
                }
            }
            for key in ["item", "content", "details", "result", "results"] {
                if let Some(nested @ (Value::Object(_) | Value::Array(_))) = map.get(key) {
                    collect_item_files(nested, parts);
                }
            }
        }
        _ => {}
    }
}

/// Extract renderable artifacts from Codex app-server MCP tool-call results.
///
/// Tool outputs such as XcodeBuildMCP screenshots are exposed by app-server as
/// `mcpToolCall.result.content[].text`. Limiting bare-path detection to that
/// shape avoids scanning normal assistant text, where file paths are often just
/// logs or code examples.
fn collect_mcp_tool_result_files(item: &Value, parts: &mut Vec<Value>) {
    let Some(tool_call) = mcp_tool_call_value(item) else {
        return;
    };
    let Some(result) = tool_call.get("result") else {
        return;
    };

    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    collect_tool_text_file_parts(text, parts);
                }
            } else {
                collect_item_files(block, parts);
            }
        }
    }

    for key in ["structuredContent", "_meta"] {
        if let Some(value @ (Value::Object(_) | Value::Array(_))) = result.get(key) {
            collect_item_files(value, parts);
        }
    }
}

fn mcp_tool_call_value(item: &Value) -> Option<&Value> {
    if item.get("type").and_then(Value::as_str) == Some("mcpToolCall") {
        return Some(item);
    }
    item.get("item")
        .filter(|nested| nested.get("type").and_then(Value::as_str) == Some("mcpToolCall"))
}

fn collect_tool_text_file_parts(text: &str, parts: &mut Vec<Value>) {
    for part in content_parts_from_text(text) {
        if crate::transfers::is_inline_file_part(&part) {
            push_unique_file_part(parts, part);
        }
    }

    for capture in tool_text_image_ref_pattern().captures_iter(text) {
        let Some(raw_ref) = capture.name("ref").map(|value| value.as_str()) else {
            continue;
        };
        let Some(path) = local_markdown_image_path(raw_ref) else {
            continue;
        };
        if let Some(part) = file_part_from_local_path(&path, None) {
            push_unique_file_part(parts, part);
        }
    }
}

fn tool_text_image_ref_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?P<ref>file://[^\s<>\]\)]+?\.(?i:png|jpe?g|gif|webp)|/[^\s<>\]\)]+?\.(?i:png|jpe?g|gif|webp))",
        )
        .expect("valid MCP tool artifact path pattern")
    })
}

fn push_unique_file_part(parts: &mut Vec<Value>, part: Value) {
    let transfer_id = part.get("transfer_id").and_then(Value::as_str);
    let data_url = part.get("data_url").and_then(Value::as_str);
    let already_present = parts.iter().any(|existing| {
        let same_transfer_id = transfer_id.is_some()
            && existing.get("transfer_id").and_then(Value::as_str) == transfer_id;
        let same_data_url =
            data_url.is_some() && existing.get("data_url").and_then(Value::as_str) == data_url;
        same_transfer_id || same_data_url
    });
    if !already_present {
        parts.push(part);
    }
}

fn collect_item_text(value: &Value, texts: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            if !text.starts_with("data:image/") && !text.trim().is_empty() {
                texts.push(text.trim().to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_item_text(item, texts);
            }
        }
        Value::Object(map) => {
            for key in [
                "text",
                "message",
                "preview",
                "summary",
                "input_text",
                "output_text",
                "output",
                "error",
                "description",
            ] {
                if let Some(value) = map.get(key) {
                    collect_item_text(value, texts);
                }
            }
            for key in ["item", "content", "details", "result", "results"] {
                if let Some(value @ (Value::Object(_) | Value::Array(_))) = map.get(key) {
                    collect_item_text(value, texts);
                }
            }
        }
        _ => {}
    }
}

fn dedupe_texts(texts: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    texts
        .into_iter()
        .filter(|text| seen.insert(text.clone()))
        .collect()
}

fn item_role(item: &Value) -> String {
    let hints = item_type_hints(item).join(" ").to_lowercase();
    if hints.contains("user") {
        return "user".to_string();
    }
    if hints.contains("approval") {
        return "approval".to_string();
    }
    if [
        "system", "tool", "command", "reason", "thinking", "analysis",
    ]
    .iter()
    .any(|hint| hints.contains(hint))
    {
        return "system".to_string();
    }
    string_field(item, "role").unwrap_or_else(|| "assistant".to_string())
}

fn item_type(item: &Value) -> String {
    string_field(item, "type")
        .or_else(|| {
            item.get("item")
                .and_then(|nested| string_field(nested, "type"))
        })
        .unwrap_or_else(|| "message".to_string())
}

fn item_phase(item: &Value) -> Option<String> {
    string_field(item, "phase").or_else(|| {
        item.get("item")
            .and_then(|nested| string_field(nested, "phase"))
    })
}

fn item_entry_id(item: &Value, turn_id: &str, offset: usize) -> String {
    match string_field(item, "id").or_else(|| string_field(item, "itemId")) {
        Some(item_id) => format!("{turn_id}-{item_id}"),
        None => format!("{turn_id}-{offset}"),
    }
}

fn fallback_input_entry_id(turn_id: &str) -> String {
    format!("{turn_id}-input")
}

fn item_type_hints(item: &Value) -> Vec<String> {
    let mut hints = Vec::new();
    if let Some(value) = string_field(item, "type") {
        hints.push(value);
    }
    if let Some(value) = item
        .get("item")
        .and_then(|nested| string_field(nested, "type"))
    {
        hints.push(value);
    }
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for part in content {
            if let Some(value) = string_field(part, "type") {
                hints.push(value);
            }
        }
    }
    hints
}

fn file_part_from_markdown_url(raw_url: &str, alt: Option<&str>) -> Option<Value> {
    let url = raw_url.trim().trim_start_matches('<').trim_end_matches('>');
    if url.starts_with("data:image/") {
        return file_part_from_data_url(url, None, alt);
    }
    let path = local_markdown_image_path(url)?;
    file_part_from_local_path(&path, alt)
}

fn local_markdown_image_path(raw_url: &str) -> Option<std::path::PathBuf> {
    let path_text = if raw_url.starts_with("file://") {
        let parsed = Url::parse(raw_url).ok()?;
        if parsed.scheme() != "file" || !matches!(parsed.host_str(), None | Some("localhost")) {
            return None;
        }
        parsed.to_file_path().ok()?.to_string_lossy().into_owned()
    } else if raw_url.starts_with('/') {
        raw_url.to_string()
    } else {
        return None;
    };
    let path = std::path::PathBuf::from(path_text);
    path.is_file().then_some(path)
}

fn file_part_from_local_path(path: &std::path::Path, alt: Option<&str>) -> Option<Value> {
    let mime_type = local_image_mime_type(path)?;
    if !is_mobile_renderable_image_mime(&mime_type) {
        return None;
    }
    let body = std::fs::read(path).ok()?;
    let data_url = format!("data:{mime_type};base64,{}", STANDARD.encode(&body));
    file_part_from_data_url(
        &data_url,
        path.file_name().and_then(|name| name.to_str()),
        alt,
    )
}

fn file_part_from_data_url(
    data_url: &str,
    file_name: Option<&str>,
    alt: Option<&str>,
) -> Option<Value> {
    let body = file_bytes_from_part(&json!({ "data_url": data_url }))?;
    let mime_type = data_url
        .strip_prefix("data:")?
        .split(';')
        .next()
        .filter(|mime| is_mobile_renderable_image_mime(mime))?;
    if !has_image_signature(mime_type, &body) {
        return None;
    }
    let mut part = json!({
        "type": "file_ref",
        "file_type": "image",
        "transfer_id": sha256_hex(&body),
        "data_url": data_url,
        "mime_type": mime_type,
        "size_bytes": body.len(),
    });
    if let Some(file_name) = file_name.filter(|value| !value.trim().is_empty()) {
        part["file_name"] = json!(file_name.trim());
    }
    if let Some(alt) = alt.filter(|value| !value.trim().is_empty()) {
        part["alt"] = json!(alt.trim());
    }
    Some(part)
}

fn local_image_mime_type(path: &std::path::Path) -> Option<String> {
    match path
        .extension()
        .and_then(|extension| extension.to_str())?
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "png" => Some("image/png".to_string()),
        "gif" => Some("image/gif".to_string()),
        "webp" => Some("image/webp".to_string()),
        _ => None,
    }
}

fn is_mobile_renderable_image_mime(mime_type: &str) -> bool {
    matches!(
        mime_type,
        "image/png" | "image/jpeg" | "image/jpg" | "image/gif" | "image/webp"
    )
}

fn has_image_signature(mime_type: &str, body: &[u8]) -> bool {
    match mime_type {
        "image/png" => {
            body.len() >= 24 && body.starts_with(b"\x89PNG\r\n\x1a\n") && &body[12..16] == b"IHDR"
        }
        "image/jpeg" | "image/jpg" => body.len() >= 4 && body.starts_with(b"\xff\xd8\xff"),
        "image/gif" => body.starts_with(b"GIF87a") || body.starts_with(b"GIF89a"),
        "image/webp" => body.len() >= 12 && body.starts_with(b"RIFF") && &body[8..12] == b"WEBP",
        _ => false,
    }
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).and_then(non_empty)
}

fn bool_field(payload: &Value, key: &str) -> bool {
    payload.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn number_field(payload: &Value, key: &str) -> Option<f64> {
    payload
        .get(key)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
}

fn same_text(left: &str, right: &str) -> bool {
    left.split_whitespace().collect::<Vec<_>>() == right.split_whitespace().collect::<Vec<_>>()
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::ProjectSummary;

    #[test]
    fn conversation_project_route_omits_workspace_cwd() {
        let route = task_project_route(CONVERSATION_PROJECT_ID, None).expect("conversation route");

        assert_eq!(route, TaskProjectRoute::Conversation);
        assert_eq!(route.project_id(), CONVERSATION_PROJECT_ID);
        assert_eq!(route.cwd(), None);
    }

    #[test]
    fn workspace_project_route_uses_project_cwd() {
        let route = task_project_route(
            "workspace-1",
            Some(ProjectSummary {
                project_id: "workspace-1".to_string(),
                project_name: "Project".to_string(),
                cwd: Some("/tmp/project".to_string()),
                updated_at: None,
            }),
        )
        .expect("workspace route");

        assert_eq!(route.project_id(), "workspace-1");
        assert_eq!(route.cwd(), Some("/tmp/project"));
    }

    #[test]
    fn unknown_project_route_errors_without_active_project_fallback() {
        let error = task_project_route("workspace-missing", None).expect_err("unknown project");

        assert!(
            error
                .to_string()
                .contains("unknown project_id=workspace-missing")
        );
    }

    #[test]
    fn workspace_project_route_requires_cwd() {
        let error = task_project_route(
            "workspace-1",
            Some(ProjectSummary {
                project_id: "workspace-1".to_string(),
                project_name: "Project".to_string(),
                cwd: None,
                updated_at: None,
            }),
        )
        .expect_err("missing cwd");

        assert!(
            error
                .to_string()
                .contains("project_id=workspace-1 has no workspace cwd")
        );
    }

    #[test]
    fn builds_file_attachment_fallback_without_local_payload() {
        let line = file_part_attachment_line(&json!({
            "type": "file_ref",
            "file_type": "image",
            "transfer_id": "abc"
        }));
        assert!(line.contains("file_type=image"));
        assert!(line.contains("transfer_id=abc"));
    }

    #[test]
    fn fallback_user_event_uses_decrypted_mobile_payload() {
        let context = ThreadContext {
            thread_id: "thread-1".to_string(),
            project_id: Some("project-1".to_string()),
            cwd: None,
            title: "Thread".to_string(),
            status: "running".to_string(),
            updated_at: Some(1_778_140_000.0),
        };

        let event = fallback_started_turn_event(&context, "turn-1", "plain mobile payload");

        assert_eq!(event.ciphertext, "plain mobile payload");
    }

    #[test]
    fn existing_threads_rely_on_canonical_user_updates() {
        assert!(!should_emit_started_user_event(Some("thread-1")));
        assert!(should_emit_started_user_event(None));
    }

    #[test]
    fn turn_entries_use_turn_scoped_item_ids() {
        let entries = extract_turn_entries(
            &json!({
                "id": "turn-1",
                "items": [
                    {
                        "id": "user-item",
                        "type": "userMessage",
                        "content": [
                            { "type": "input_text", "text": "first user message" },
                            { "type": "input_image", "image_url": "data:image/png;base64,invalid" }
                        ]
                    },
                    {
                        "type": "userMessage",
                        "content": [
                            { "type": "input_text", "text": "second user message" }
                        ]
                    },
                    {
                        "id": "agent-item",
                        "type": "agentMessage",
                        "content": [
                            { "type": "output_text", "text": "assistant message" }
                        ]
                    }
                ]
            }),
            None,
        );
        let entry_ids: Vec<_> = entries
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect();
        assert_eq!(
            entry_ids,
            vec!["turn-1-user-item", "turn-1-1", "turn-1-agent-item"]
        );
    }

    #[test]
    fn replay_image_user_entries_use_json_content_parts_contract() {
        let entries = extract_turn_entries(
            &json!({
                "id": "turn-1",
                "items": [
                    {
                        "id": "user-item",
                        "type": "userMessage",
                        "content": [
                            { "type": "input_text", "text": "这张图片是什么内容？" },
                            {
                                "type": "input_image",
                                "image_url": "data:image/jpeg;base64,/9j/4A==",
                                "file_name": "image-1.jpeg"
                            }
                        ]
                    }
                ]
            }),
            None,
        );

        let payload: Value =
            serde_json::from_str(&entries[0].text).expect("JSON content-parts payload");
        let parts = payload
            .get("content_parts")
            .and_then(Value::as_array)
            .expect("content_parts array");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].get("type").and_then(Value::as_str), Some("text"));
        assert_eq!(
            parts[1].get("file_name").and_then(Value::as_str),
            Some("image-1.jpeg")
        );
    }

    #[test]
    fn final_file_changes_become_one_summary_before_final_answer() {
        let entries = extract_turn_entries(
            &json!({
                "id": "turn-1",
                "items": [
                    {
                        "id": "change-1",
                        "type": "fileChange",
                        "status": "completed",
                        "changes": [{
                            "path": "/repo/design/a.md",
                            "kind": { "type": "update", "move_path": null },
                            "diff": "@@ -1 +1,2 @@\n-old\n+new\n+extra\n"
                        }]
                    },
                    {
                        "id": "final",
                        "type": "agentMessage",
                        "phase": "final_answer",
                        "content": [
                            { "type": "output_text", "text": "done" }
                        ]
                    }
                ]
            }),
            Some("/repo"),
        );

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].item_type, "fileChange");
        assert_eq!(
            entries[0].phase.as_deref(),
            Some("final_answer_file_change")
        );
        let payload: Value =
            serde_json::from_str(&entries[0].text).expect("JSON content-parts payload");
        let summary = &payload["content_parts"][0];
        assert_eq!(summary["type"], "file_change_summary");
        assert_eq!(summary["files"], 1);
        assert_eq!(summary["additions"], 2);
        assert_eq!(summary["deletions"], 1);
        assert_eq!(summary["files_summary"][0]["path"], "design/a.md");
        assert_eq!(entries[1].phase.as_deref(), Some("final_answer"));
    }

    #[test]
    fn mcp_tool_call_text_path_becomes_image_part_without_prefix_dependency() {
        let path = write_test_jpeg("mcp-tool-artifact");
        let tool_text = format!("Created artifact at {} (image/jpeg)", path.display());

        let ciphertext = item_mobile_ciphertext(&json!({
            "id": "call-1",
            "type": "mcpToolCall",
            "server": "xcodebuildmcp",
            "tool": "screenshot",
            "status": "completed",
            "result": {
                "content": [
                    { "type": "text", "text": tool_text }
                ],
                "structuredContent": null,
                "_meta": null
            }
        }));

        let _ = std::fs::remove_file(&path);
        let payload: Value = serde_json::from_str(&ciphertext).expect("JSON content-parts payload");
        let parts = payload
            .get("content_parts")
            .and_then(Value::as_array)
            .expect("content_parts array");

        assert!(parts.iter().any(|part| {
            part.get("type").and_then(Value::as_str) == Some("text")
                && part
                    .get("text")
                    .and_then(Value::as_str)
                    .is_some_and(|text| text.contains("Created artifact"))
        }));
        assert!(parts.iter().any(|part| {
            part.get("type").and_then(Value::as_str) == Some("file_ref")
                && part.get("file_type").and_then(Value::as_str) == Some("image")
                && part.get("mime_type").and_then(Value::as_str) == Some("image/jpeg")
                && part.get("file_name").and_then(Value::as_str)
                    == path.file_name().and_then(|name| name.to_str())
        }));
    }

    #[test]
    fn plain_message_text_path_is_not_scanned_as_artifact() {
        let path = write_test_jpeg("plain-message-path");
        let text = format!("Log mentions a local image path: {}", path.display());

        let ciphertext = item_mobile_ciphertext(&json!({
            "id": "agent-1",
            "type": "agentMessage",
            "content": [
                { "type": "output_text", "text": text }
            ]
        }));

        let _ = std::fs::remove_file(&path);
        assert_eq!(ciphertext, text);
    }

    fn write_test_jpeg(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "niuma-cli-{name}-{}-{}.jpg",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&path, b"\xff\xd8\xff\xe0test image bytes").expect("write test image");
        path
    }
}
