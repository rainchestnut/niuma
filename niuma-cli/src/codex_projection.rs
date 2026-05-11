//! Projection from Codex app-server thread payloads into mobile thread rows.
//!
//! This module is the only place that understands Codex app-server turn/item
//! shapes. `tasks.rs` asks it for thread metadata and replay events, then only
//! forwards those events to the server wire protocol.

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use regex::Regex;
use serde_json::{Value, json};
use std::sync::OnceLock;
use url::Url;

use crate::diff_summary;
use crate::file_access::FileReadOutcome;
use crate::process_summary::{mcp_tool_call_value, process_summary_payload};
use crate::thread_status::normalize_thread_status;
use crate::transfers::{encode_content_parts_payload, file_bytes_from_part, sha256_hex};

#[derive(Debug, Clone)]
pub(crate) struct ThreadContext {
    pub(crate) thread_id: String,
    pub(crate) project_id: Option<String>,
    pub(crate) cwd: Option<String>,
    pub(crate) title: String,
    pub(crate) status: String,
    pub(crate) updated_at: Option<f64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ThreadEntry {
    pub(crate) entry_id: String,
    pub(crate) role: String,
    pub(crate) text: String,
    pub(crate) item_type: String,
    pub(crate) phase: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ThreadEvent {
    pub(crate) thread_id: String,
    pub(crate) seq: i64,
    pub(crate) ciphertext: String,
    pub(crate) checkpoint: Option<String>,
    pub(crate) role: String,
    pub(crate) item_type: String,
    pub(crate) phase: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) entry_id: Option<String>,
    pub(crate) created_at: Option<f64>,
}

impl ThreadEvent {
    pub(crate) fn into_wire(self, device_id: &str) -> Value {
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

pub(crate) fn metadata_messages_for_thread(context: &ThreadContext) -> Vec<Value> {
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

pub(crate) fn thread_context(payload: &Value, project_id: Option<String>) -> ThreadContext {
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

pub(crate) fn replay_events_from_thread(
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
        if entries.is_empty()
            && let Some(preview) = string_field(&turn, "preview")
        {
            entries.push(ThreadEntry {
                entry_id: turn_id.clone(),
                role: "user".to_string(),
                text: preview,
                item_type: "userMessage".to_string(),
                phase: None,
            });
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

pub(crate) fn extract_turn_entries(turn: &Value, cwd: Option<&str>) -> Vec<ThreadEntry> {
    let mut entries = Vec::new();
    let turn_id = string_field(turn, "id").unwrap_or_else(|| "turn".to_string());
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let final_file_change_summary = final_file_change_summary(&items);
    for (item_index, item) in items.iter().enumerate() {
        if final_file_change_summary
            .as_ref()
            .is_some_and(|summary| summary.completed_indices.contains(&item_index))
        {
            continue;
        }
        let text = item_mobile_ciphertext(item);
        if text.is_empty() {
            continue;
        }
        let role = item_role(item);
        let offset = entries.len();
        let entry_id = item_entry_id(item, &turn_id, offset);
        entries.push(ThreadEntry {
            entry_id: entry_id.clone(),
            role,
            text,
            item_type: item_type(item),
            phase: item_phase(item),
        });
        if let Some(summary) = final_file_change_summary.as_ref()
            && item_index == summary.final_index
            && let Some(part) =
                diff_summary::file_change_summary_part(&turn_id, &entry_id, &summary.items, cwd)
        {
            entries.push(ThreadEntry {
                entry_id: final_file_change_entry_id(&turn_id, &summary.items),
                role: "assistant".to_string(),
                text: encode_content_parts_payload(&[part]),
                item_type: "fileChange".to_string(),
                phase: Some("final_answer_file_change".to_string()),
            });
        }
    }
    if let Some(input_text) = turn_input_text(turn, &entries)
        && !entries
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
    entries
}

pub(crate) fn fallback_started_turn_event(
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

pub(crate) fn should_emit_started_user_event(requested_thread_id: Option<&str>) -> bool {
    requested_thread_id.is_none()
}

pub(crate) fn file_part_attachment_line(part: &Value) -> String {
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

pub(crate) fn file_part_type(part: &Value) -> String {
    if let Some(file_type) = string_field(part, "file_type") {
        return file_type;
    }
    match string_field(part, "mime_type") {
        Some(mime_type) if mime_type.starts_with("image/") => "image".to_string(),
        Some(mime_type) if mime_type.starts_with("video/") => "video".to_string(),
        _ => "file".to_string(),
    }
}

pub(crate) fn item_mobile_ciphertext(item: &Value) -> String {
    if let Some(summary) = process_summary_payload(item) {
        let mut parts = vec![json!({ "type": "text", "text": summary })];
        collect_mcp_tool_result_files(item, &mut parts);
        return encode_content_parts_payload(&parts);
    }

    let text = item_text(item);
    let mut parts = content_parts_from_text(&text);
    collect_item_files(item, &mut parts);
    collect_mcp_tool_result_files(item, &mut parts);
    if should_encode_generated_parts(&parts) {
        encode_content_parts_payload(&parts)
    } else {
        text
    }
}

pub(crate) fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).and_then(non_empty)
}

struct FinalFileChangeSummary {
    final_index: usize,
    completed_indices: Vec<usize>,
    items: Vec<Value>,
}

/// Collect all completed file changes associated with the turn's final answer.
///
/// Codex app-server can interleave build, test, and tool progress messages
/// between fileChange items. Mobile should receive one final turn summary, not
/// only the last contiguous fileChange block.
fn final_file_change_summary(items: &[Value]) -> Option<FinalFileChangeSummary> {
    let final_index = items.iter().rposition(is_final_answer_item)?;
    let completed_indices = items[..final_index]
        .iter()
        .enumerate()
        .filter_map(|(index, item)| is_completed_file_change(item).then_some(index))
        .collect::<Vec<_>>();
    if completed_indices.is_empty() {
        return None;
    }
    let items = completed_indices
        .iter()
        .map(|index| items[*index].clone())
        .collect();
    Some(FinalFileChangeSummary {
        final_index,
        completed_indices,
        items,
    })
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
            Some(LocalFileProjection::TransferPart(part)) => parts.push(part),
            Some(LocalFileProjection::PathOnlyNote(part)) => parts.push(part),
            None => parts.push(json!({ "type": "text", "text": full.as_str() })),
        }
        cursor = full.end();
    }
    let suffix = text[cursor..].trim();
    if !suffix.is_empty() {
        parts.push(json!({ "type": "text", "text": suffix }));
    }
    if parts.is_empty()
        && text.trim_start().starts_with("data:image/")
        && let Some(part) = file_part_from_data_url(text.trim(), None, None)
    {
        parts.push(part);
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
            if matches!(item_type, "localImage" | "local_image")
                && let Some(path_value) = map
                    .get("path")
                    .or_else(|| map.get("file_path"))
                    .or_else(|| map.get("url"))
                    .and_then(Value::as_str)
                && let Some(path) = local_path_from_ref(path_value)
                && let Some(part) =
                    file_part_from_local_path(&path, string_field(value, "alt").as_deref())
            {
                match part {
                    LocalFileProjection::TransferPart(part)
                    | LocalFileProjection::PathOnlyNote(part) => parts.push(part),
                }
                return;
            }
            if matches!(item_type, "input_image" | "image" | "output_image") {
                let data_url = map
                    .get("image_url")
                    .or_else(|| map.get("url"))
                    .or_else(|| map.get("data_url"))
                    .and_then(Value::as_str);
                if let Some(data_url) = data_url.filter(|value| value.starts_with("data:image/"))
                    && let Some(part) = file_part_from_data_url(
                        data_url,
                        string_field(value, "file_name")
                            .or_else(|| string_field(value, "filename"))
                            .or_else(|| string_field(value, "name"))
                            .as_deref(),
                        string_field(value, "alt").as_deref(),
                    )
                {
                    parts.push(part);
                    return;
                }
                let data = map.get("data").and_then(Value::as_str);
                let mime_type = map
                    .get("mimeType")
                    .or_else(|| map.get("mime_type"))
                    .and_then(Value::as_str);
                if let (Some(data), Some(mime_type)) = (data, mime_type)
                    && mime_type.starts_with("image/")
                {
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
            for key in ["item", "content", "details", "result", "results"] {
                if let Some(nested @ (Value::Object(_) | Value::Array(_))) = map.get(key) {
                    collect_item_files(nested, parts);
                }
            }
        }
        _ => {}
    }
}

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
        let Some(path) = local_path_from_ref(raw_ref) else {
            continue;
        };
        if let Some(part) = file_part_from_local_path(&path, None) {
            match part {
                LocalFileProjection::TransferPart(part) => push_unique_file_part(parts, part),
                LocalFileProjection::PathOnlyNote(part) => parts.push(part),
            }
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

fn should_encode_generated_parts(parts: &[Value]) -> bool {
    parts
        .iter()
        .any(|part| crate::transfers::is_inline_file_part(part) || is_local_path_note_part(part))
}

fn is_local_path_note_part(part: &Value) -> bool {
    part.get("source").and_then(Value::as_str) == Some("local_file_path_only")
}

fn collect_item_text(value: &Value, texts: &mut Vec<String>) {
    match value {
        Value::String(text) if !text.starts_with("data:image/") && !text.trim().is_empty() => {
            texts.push(text.trim().to_string());
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

enum LocalFileProjection {
    TransferPart(Value),
    PathOnlyNote(Value),
}

fn file_part_from_markdown_url(raw_url: &str, alt: Option<&str>) -> Option<LocalFileProjection> {
    let url = raw_url.trim().trim_start_matches('<').trim_end_matches('>');
    if url.starts_with("data:image/") {
        return file_part_from_data_url(url, None, alt).map(LocalFileProjection::TransferPart);
    }
    let path = local_path_from_ref(url)?;
    file_part_from_local_path(&path, alt)
}

fn local_path_from_ref(raw_url: &str) -> Option<std::path::PathBuf> {
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
    path.is_absolute().then_some(path)
}

fn file_part_from_local_path(
    path: &std::path::Path,
    alt: Option<&str>,
) -> Option<LocalFileProjection> {
    let mime_type = local_image_mime_type(path)?;
    if !is_mobile_renderable_image_mime(&mime_type) {
        return None;
    }
    let body = match crate::file_access::read_file_for_transfer(path) {
        FileReadOutcome::Bytes(body) => body,
        FileReadOutcome::PathOnly { path, reason } => {
            return Some(LocalFileProjection::PathOnlyNote(
                local_file_path_note_part(&path, &reason, alt),
            ));
        }
    };
    let data_url = format!("data:{mime_type};base64,{}", STANDARD.encode(&body));
    file_part_from_data_url(
        &data_url,
        path.file_name().and_then(|name| name.to_str()),
        alt,
    )
    .map(LocalFileProjection::TransferPart)
    .or_else(|| {
        Some(LocalFileProjection::PathOnlyNote(
            local_file_path_note_part(&path.to_string_lossy(), "文件不是有效图片", alt),
        ))
    })
}

fn local_file_path_note_part(path: &str, reason: &str, alt: Option<&str>) -> Value {
    let label = alt
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("本地文件");
    json!({
        "type": "text",
        "source": "local_file_path_only",
        "text": format!("{label}未传输：{path}\n原因：{reason}"),
    })
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

    #[test]
    fn final_file_change_summary_merges_non_contiguous_changes() {
        let turn = json!({
            "id": "turn-1",
            "items": [
                {
                    "id": "change-a",
                    "type": "fileChange",
                    "status": "completed",
                    "changes": [{
                        "path": "/repo/a.rs",
                        "kind": { "type": "update" },
                        "diff": "@@ -1 +1 @@\n-old-a\n+new-a\n"
                    }]
                },
                {
                    "id": "progress",
                    "type": "agentMessage",
                    "text": "running build"
                },
                {
                    "id": "change-b",
                    "type": "fileChange",
                    "status": "completed",
                    "changes": [{
                        "path": "/repo/b.rs",
                        "kind": { "type": "update" },
                        "diff": "@@ -1 +1 @@\n-old-b\n+new-b\n"
                    }]
                },
                {
                    "id": "final",
                    "type": "agentMessage",
                    "phase": "final_answer",
                    "text": "done"
                }
            ]
        });

        let entries = extract_turn_entries(&turn, Some("/repo"));
        let file_summaries = entries
            .iter()
            .filter(|entry| entry.item_type == "fileChange")
            .collect::<Vec<_>>();

        assert_eq!(file_summaries.len(), 1);
        assert!(file_summaries[0].entry_id.contains("change-a"));
        assert!(file_summaries[0].entry_id.contains("change-b"));

        let payload: Value = serde_json::from_str(&file_summaries[0].text).unwrap();
        let part = &payload["content_parts"][0];
        assert_eq!(part["type"], "file_change_summary");
        assert_eq!(part["files"], 2);
        assert_eq!(part["additions"], 2);
        assert_eq!(part["deletions"], 2);
    }
}
