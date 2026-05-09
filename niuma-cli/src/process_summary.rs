//! Compact mobile summaries for Codex tool-call process items.
//!
//! The gateway keeps raw app-server tool output out of the mobile chat body.
//! Only stable metadata, diagnostics, and renderable attachments are projected
//! to iOS; display labels and localized status text belong to the client.

use serde_json::{Value, json};

pub(crate) fn process_summary_payload(item: &Value) -> Option<String> {
    let tool_call = mcp_tool_call_value(item)?;
    let result_text = mcp_tool_result_text(tool_call);
    let warnings = diagnostic_messages(tool_call, &result_text, "warnings");
    let errors = diagnostic_messages(tool_call, &result_text, "errors");
    let warning_count = diagnostic_count(tool_call, &result_text, "warnings", warnings.len());
    let error_count = diagnostic_count(tool_call, &result_text, "errors", errors.len());
    let status = process_status(tool_call);
    let tool_key = process_tool_key(tool_call);
    let diagnostics = diagnostic_payloads(warnings, errors);

    serde_json::to_string(&json!({
        "kind": "process_summary",
        "tool_key": tool_key,
        "status": status,
        "warning_count": warning_count,
        "error_count": error_count,
        "diagnostics": diagnostics,
    }))
    .ok()
}

pub(crate) fn mcp_tool_call_value(item: &Value) -> Option<&Value> {
    if item.get("type").and_then(Value::as_str) == Some("mcpToolCall") {
        return Some(item);
    }
    item.get("item")
        .filter(|nested| nested.get("type").and_then(Value::as_str) == Some("mcpToolCall"))
}

fn mcp_tool_result_text(tool_call: &Value) -> String {
    let Some(result) = tool_call.get("result") else {
        return String::new();
    };
    let mut texts = Vec::new();
    if let Some(content) = result.get("content").and_then(Value::as_array) {
        for block in content {
            if block.get("type").and_then(Value::as_str) == Some("text") {
                if let Some(text) = string_field(block, "text") {
                    texts.push(text);
                }
            }
        }
    }
    dedupe_texts(texts).join("\n")
}

fn process_tool_key(tool_call: &Value) -> String {
    string_field(tool_call, "tool")
        .or_else(|| string_field(tool_call, "name"))
        .or_else(|| string_field(tool_call, "method"))
        .unwrap_or_else(|| "unknown".to_string())
}

fn process_status(tool_call: &Value) -> Option<String> {
    let result = tool_call.get("result");
    if tool_call_did_error(tool_call) {
        return Some("failed".to_string());
    }

    let raw = string_field(tool_call, "status")
        .or_else(|| result.and_then(|value| string_field(value, "status")))
        .or_else(|| {
            result
                .and_then(|value| value.get("structuredContent"))
                .and_then(|value| value.get("summary"))
                .and_then(|value| string_field(value, "status"))
        });
    raw.map(|status| match status.to_ascii_lowercase().as_str() {
        "succeeded" | "success" | "completed" => "succeeded".to_string(),
        "failed" | "error" => "failed".to_string(),
        other => other.to_string(),
    })
}

fn tool_call_did_error(tool_call: &Value) -> bool {
    let result = tool_call.get("result");
    tool_call
        .get("didError")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || result
            .and_then(|value| value.get("didError"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || result
            .and_then(|value| value.get("structuredContent"))
            .and_then(|value| value.get("didError"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn diagnostic_messages(tool_call: &Value, result_text: &str, key: &str) -> Vec<String> {
    let mut messages = structured_diagnostic_messages(tool_call, key);
    if messages.is_empty() {
        messages = section_diagnostic_messages(result_text, key);
    }
    if messages.is_empty() && key == "errors" && tool_call_did_error(tool_call) {
        messages = leading_error_messages(result_text);
    }
    dedupe_texts(messages)
        .into_iter()
        .take(3)
        .collect::<Vec<_>>()
}

fn diagnostic_payloads(warnings: Vec<String>, errors: Vec<String>) -> Vec<Value> {
    errors
        .into_iter()
        .take(3)
        .map(|message| json!({ "severity": "error", "message": message }))
        .chain(
            warnings
                .into_iter()
                .take(3)
                .map(|message| json!({ "severity": "warning", "message": message })),
        )
        .collect()
}

fn structured_diagnostic_messages(tool_call: &Value, key: &str) -> Vec<String> {
    let diagnostics = tool_call
        .get("result")
        .and_then(|value| value.get("structuredContent"))
        .and_then(|value| value.get("diagnostics"))
        .or_else(|| {
            tool_call
                .get("result")
                .and_then(|value| value.get("diagnostics"))
        });
    let Some(items) = diagnostics
        .and_then(|value| value.get(key))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    items.iter().filter_map(diagnostic_message).collect()
}

fn diagnostic_message(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str().and_then(non_empty) {
        return Some(text);
    }
    let message = string_field(value, "message")?;
    if let Some(location) = string_field(value, "location") {
        Some(format!("{message} ({location})"))
    } else {
        Some(message)
    }
}

fn section_diagnostic_messages(result_text: &str, key: &str) -> Vec<String> {
    let section = if key == "warnings" {
        "Warnings ("
    } else {
        "Errors ("
    };
    let other_section = if key == "warnings" {
        "Errors ("
    } else {
        "Warnings ("
    };
    let mut messages = Vec::new();
    let mut in_section = false;

    for line in result_text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(section) {
            in_section = true;
            if let Some(after_colon) = trimmed.split_once(':').map(|(_, tail)| tail.trim()) {
                if !after_colon.is_empty() {
                    messages.push(clean_diagnostic_line(after_colon));
                }
            }
            continue;
        }
        if trimmed.starts_with(other_section) {
            in_section = false;
            continue;
        }
        if !in_section || trimmed.is_empty() {
            continue;
        }
        messages.push(clean_diagnostic_line(trimmed));
    }

    messages
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect()
}

fn leading_error_messages(result_text: &str) -> Vec<String> {
    result_text
        .lines()
        .map(str::trim)
        .filter(|line| {
            line.starts_with("Error:")
                || line.starts_with("Error ")
                || line.starts_with("❌")
                || line.starts_with("✗")
        })
        .map(clean_diagnostic_line)
        .filter(|line| !line.is_empty())
        .collect()
}

fn clean_diagnostic_line(line: &str) -> String {
    line.trim_start_matches(|ch: char| !ch.is_alphanumeric() && ch != '/' && ch != '[' && ch != '(')
        .trim()
        .to_string()
}

fn diagnostic_count(
    tool_call: &Value,
    result_text: &str,
    key: &str,
    fallback_count: usize,
) -> usize {
    structured_diagnostic_messages(tool_call, key)
        .len()
        .max(section_count(result_text, key).unwrap_or(0))
        .max(fallback_count)
}

fn section_count(result_text: &str, key: &str) -> Option<usize> {
    let section = if key == "warnings" {
        "Warnings ("
    } else {
        "Errors ("
    };
    result_text.lines().find_map(|line| {
        let trimmed = line.trim();
        let rest = trimmed.strip_prefix(section)?;
        let count = rest.split(')').next()?;
        count.parse::<usize>().ok()
    })
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload.get(key).and_then(Value::as_str).and_then(non_empty)
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn dedupe_texts(texts: Vec<String>) -> Vec<String> {
    let mut deduped = Vec::new();
    for text in texts {
        if !deduped.iter().any(|existing| existing == &text) {
            deduped.push(text);
        }
    }
    deduped
}
