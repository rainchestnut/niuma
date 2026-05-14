//! Mobile task orchestration for Codex app-server.
//!
//! This is the Rust replacement for the Python plugin's main Codex bridge:
//! mobile payloads are converted into Codex-native input items, submitted to
//! Codex, and projected thread events are forwarded to the server wire protocol.

use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::codex_app_server::{CodexAppServerClient, TurnStartPayload, TurnSteerPayload};
use crate::codex_projection::{
    file_part_attachment_line, file_part_type, metadata_messages_for_thread,
    replay_events_from_thread, string_field, thread_context,
};
use crate::identity::AgentIdentity;
use crate::metadata::CodexWorkspaceStore;
use crate::transfers::TransferContext;
use crate::{bindings, crypto};

#[cfg(test)]
use crate::codex_projection::{extract_turn_entries, item_mobile_ciphertext};

const CONVERSATION_PROJECT_ID: &str = "__conversation__";

#[derive(Debug, Clone, Deserialize)]
pub struct TaskStartInbound {
    #[serde(default)]
    pub request_id: Option<String>,
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
pub struct TaskSteerInbound {
    #[serde(default)]
    pub request_id: Option<String>,
    pub device_id: String,
    pub agent_id: String,
    pub thread_id: String,
    pub ciphertext: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaskInterruptInbound {
    #[serde(default)]
    pub request_id: Option<String>,
    pub device_id: String,
    pub agent_id: String,
    pub thread_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ResumeThreadInbound {
    #[serde(default)]
    pub request_id: Option<String>,
    pub device_id: String,
    pub thread_id: String,
    pub cursor: i64,
    pub checkpoint: Option<String>,
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

#[derive(Debug)]
pub struct TaskStartMessages {
    pub thread_id: String,
    pub turn_id: String,
    pub device_id: String,
    pub project_id: Option<String>,
    pub messages: Vec<Value>,
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
    pub async fn start_task_messages(
        &self,
        message: TaskStartInbound,
    ) -> Result<TaskStartMessages> {
        let requested_thread_id = message.thread_id.as_deref();
        let (thread_payload, project_id) = match requested_thread_id {
            // Existing Codex threads already carry their own projectless/workspace
            // identity. Passing a cwd on resume can rewrite the desktop app's
            // placement hints, and a failed resume must not silently create a
            // replacement thread under the mobile request's current project.
            Some(thread_id) => {
                let thread = self
                    .app_server
                    .resume_thread_payload(
                        thread_id,
                        None,
                        message.approval_policy.as_deref(),
                        message.approvals_reviewer.as_deref(),
                        message.sandbox_mode.as_deref(),
                    )
                    .await?;
                (thread, None)
            }
            None => {
                let route = self.project_route_for_task(&message.project_id)?;
                let cwd = route.cwd();
                let thread = self
                    .app_server
                    .start_thread_payload(
                        cwd,
                        message.approval_policy.as_deref(),
                        message.approvals_reviewer.as_deref(),
                        message.sandbox_mode.as_deref(),
                    )
                    .await?;
                (thread, Some(route.project_id().to_string()))
            }
        };
        let thread_id = string_field(&thread_payload, "id").context("Codex thread missing id")?;
        let plaintext = self.decrypt_mobile_task_payload(&message)?;
        let input_items = self
            .decode_mobile_payload_to_codex_input(&message.device_id, &plaintext)
            .await;
        let turn_payload = self
            .app_server
            .start_turn_payload(TurnStartPayload {
                thread_id: &thread_id,
                input_items,
                model: message.model.as_deref(),
                effort: message.effort.as_deref(),
                approval_policy: message.approval_policy.as_deref(),
                approvals_reviewer: message.approvals_reviewer.as_deref(),
                sandbox_mode: message.sandbox_mode.as_deref(),
            })
            .await?;
        let turn_id = string_field(&turn_payload, "id").context("Codex turn missing id")?;
        let replay_payload = self
            .app_server
            .read_thread_payload(&thread_id, true)
            .await
            .unwrap_or(thread_payload);
        let context = thread_context(&replay_payload, project_id);
        let mut messages = metadata_messages_for_thread(&context);
        if requested_thread_id.is_none() {
            messages.push(task_start_sync(
                &message.device_id,
                message.request_id.as_deref(),
                &thread_id,
                None,
            ));
        }
        Ok(TaskStartMessages {
            thread_id,
            turn_id,
            device_id: message.device_id,
            project_id: context.project_id,
            messages,
        })
    }

    /// Steer a running Codex turn with mobile-originated additional input.
    pub async fn steer_task(
        &self,
        message: TaskSteerInbound,
        expected_turn_id: &str,
    ) -> Result<()> {
        let plaintext = self.decrypt_mobile_steer_payload(&message)?;
        let input_items = self
            .decode_mobile_payload_to_codex_input(&message.device_id, &plaintext)
            .await;
        self.app_server
            .steer_turn_payload(TurnSteerPayload {
                thread_id: &message.thread_id,
                expected_turn_id,
                input_items,
            })
            .await?;
        Ok(())
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
                .map(|event| event.into_wire(&message.device_id))
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

    fn decrypt_mobile_steer_payload(&self, message: &TaskSteerInbound) -> Result<String> {
        let binding = bindings::binding_for_device(&message.device_id)?
            .with_context(|| format!("missing pair binding for device {}", message.device_id))?;
        let aad = task_steer_aad(message);
        let plaintext = crypto::decrypt_payload(
            &self.identity.encryption_private_key,
            &binding.ios_encryption_public_key,
            &binding.binding_id,
            crypto::PayloadDirection::IosToAgent,
            &message.ciphertext,
            &aad,
        )?;
        String::from_utf8(plaintext).context("task_steer plaintext is not utf-8")
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

/// Acknowledges the mobile request once Codex has assigned the real thread id.
fn task_start_sync(
    device_id: &str,
    request_id: Option<&str>,
    thread_id: &str,
    error: Option<String>,
) -> Value {
    let succeeded = error.is_none();
    json!({
        "kind": "task_start_sync",
        "device_id": device_id,
        "request_id": request_id,
        "thread_id": thread_id,
        "succeeded": succeeded,
        "error": error,
    })
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

pub fn task_steer_aad(message: &TaskSteerInbound) -> Vec<u8> {
    crypto::payload_aad(&[
        ("kind", "task_steer".to_string()),
        ("device_id", message.device_id.clone()),
        ("agent_id", message.agent_id.clone()),
        ("thread_id", message.thread_id.clone()),
    ])
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
    fn task_start_sync_preserves_mobile_request_id() {
        let sync = task_start_sync("device-1", Some("request-1"), "thread-1", None);

        assert_eq!(sync["kind"], "task_start_sync");
        assert_eq!(sync["request_id"], "request-1");
        assert_eq!(sync["thread_id"], "thread-1");
        assert_eq!(sync["succeeded"], true);
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
    fn final_file_changes_become_one_summary_after_final_answer() {
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
        assert_eq!(entries[0].phase.as_deref(), Some("final_answer"));
        assert_eq!(entries[1].item_type, "fileChange");
        assert_eq!(
            entries[1].phase.as_deref(),
            Some("final_answer_file_change")
        );
        let payload: Value =
            serde_json::from_str(&entries[1].text).expect("JSON content-parts payload");
        let summary = &payload["content_parts"][0];
        assert_eq!(summary["type"], "file_change_summary");
        assert_eq!(summary["files"], 1);
        assert_eq!(summary["additions"], 2);
        assert_eq!(summary["deletions"], 1);
        assert_eq!(summary["files_summary"][0]["path"], "design/a.md");
    }

    #[test]
    fn mcp_tool_call_text_path_becomes_image_part_without_prefix_dependency() {
        crate::file_access::configure_test_roots(
            vec![std::env::temp_dir().to_string_lossy().into_owned()],
            crate::file_access::FileAccessStatus::Granted,
        );
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

        let summary: Value = serde_json::from_str(
            parts[0]
                .get("text")
                .and_then(Value::as_str)
                .expect("summary text"),
        )
        .expect("process summary JSON");
        assert_eq!(summary["kind"], "process_summary");
        assert_eq!(summary["tool_key"], "screenshot");
        assert!(summary.get("title").is_none());
        assert!(summary.get("summary").is_none());
        assert!(parts.iter().any(|part| {
            part.get("type").and_then(Value::as_str) == Some("file_ref")
                && part.get("file_type").and_then(Value::as_str) == Some("image")
                && part.get("mime_type").and_then(Value::as_str) == Some("image/jpeg")
                && part.get("file_name").and_then(Value::as_str)
                    == path.file_name().and_then(|name| name.to_str())
        }));
    }

    #[test]
    fn mcp_tool_call_summary_hides_raw_default_tree() {
        let raw = "⚙️ Show Defaults\n\n📁 (default)\n├─ projectPath: /repo/app.xcodeproj\n├─ scheme: niuma\n🚀 Build & Run\n\nWarnings (2):\n⚠ call to main actor-isolated static method";
        let ciphertext = item_mobile_ciphertext(&json!({
            "id": "call-1",
            "type": "mcpToolCall",
            "server": "xcodebuildmcp",
            "tool": "session_show_defaults",
            "status": "completed",
            "result": {
                "content": [
                    { "type": "text", "text": raw }
                ]
            }
        }));

        let payload: Value = serde_json::from_str(&ciphertext).expect("JSON content-parts payload");
        let parts = payload["content_parts"].as_array().expect("content parts");
        let summary_text = parts[0]
            .get("text")
            .and_then(Value::as_str)
            .expect("summary text");
        let summary: Value = serde_json::from_str(summary_text).expect("process summary JSON");

        assert_eq!(summary["kind"], "process_summary");
        assert_eq!(summary["tool_key"], "session_show_defaults");
        assert_eq!(summary["warning_count"], 2);
        assert!(summary.get("title").is_none());
        assert!(summary.get("summary").is_none());
        assert!(!summary_text.contains("projectPath"));
        assert!(!summary_text.contains("app.xcodeproj"));
    }

    #[test]
    fn mcp_tool_call_summary_prefers_structured_diagnostics() {
        let ciphertext = item_mobile_ciphertext(&json!({
            "id": "call-1",
            "type": "mcpToolCall",
            "server": "xcodebuildmcp",
            "tool": "build_run_sim",
            "status": "completed",
            "result": {
                "content": [
                    { "type": "text", "text": "🚀 Build & Run\nWarnings (1):\n⚠ raw warning" }
                ],
                "structuredContent": {
                    "summary": { "status": "SUCCEEDED" },
                    "diagnostics": {
                        "warnings": [
                            {
                                "message": "structured warning",
                                "location": "/repo/App.swift:12"
                            }
                        ],
                        "errors": []
                    }
                }
            }
        }));

        let payload: Value = serde_json::from_str(&ciphertext).expect("JSON content-parts payload");
        let summary_text = payload["content_parts"][0]
            .get("text")
            .and_then(Value::as_str)
            .expect("summary text");
        let summary: Value = serde_json::from_str(summary_text).expect("process summary JSON");

        assert_eq!(summary["tool_key"], "build_run_sim");
        assert_eq!(summary["status"], "succeeded");
        assert_eq!(summary["warning_count"], 1);
        assert_eq!(
            summary["diagnostics"][0]["message"],
            "structured warning (/repo/App.swift:12)"
        );
        assert_eq!(summary["diagnostics"][0]["severity"], "warning");
    }

    #[test]
    fn mcp_tool_call_summary_ignores_ui_hierarchy_error_words() {
        let raw = "📋 Snapshot UI\nAXLabel\": \"niuma-cli/src/tasks.rs 里 error_count\"\nAXLabel\": \"展开态：warning/error 数量\"";
        let ciphertext = item_mobile_ciphertext(&json!({
            "id": "call-1",
            "type": "mcpToolCall",
            "server": "xcodebuildmcp",
            "tool": "snapshot_ui",
            "status": "completed",
            "result": {
                "content": [
                    { "type": "text", "text": raw }
                ]
            }
        }));

        let payload: Value = serde_json::from_str(&ciphertext).expect("JSON content-parts payload");
        let summary_text = payload["content_parts"][0]
            .get("text")
            .and_then(Value::as_str)
            .expect("summary text");
        let summary: Value = serde_json::from_str(summary_text).expect("process summary JSON");

        assert_eq!(summary["tool_key"], "snapshot_ui");
        assert_eq!(summary["status"], "succeeded");
        assert_eq!(summary["warning_count"], 0);
        assert_eq!(summary["error_count"], 0);
        assert!(
            summary["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .is_empty()
        );
    }

    #[test]
    fn mcp_tool_call_summary_ignores_default_tree_warning_words() {
        let raw =
            "⚙️ Show Defaults\n📁 (default)\n├─ suppressWarnings: (not set)\n├─ scheme: niuma";
        let ciphertext = item_mobile_ciphertext(&json!({
            "id": "call-1",
            "type": "mcpToolCall",
            "server": "xcodebuildmcp",
            "tool": "session_show_defaults",
            "status": "completed",
            "result": {
                "content": [
                    { "type": "text", "text": raw }
                ]
            }
        }));

        let payload: Value = serde_json::from_str(&ciphertext).expect("JSON content-parts payload");
        let summary_text = payload["content_parts"][0]
            .get("text")
            .and_then(Value::as_str)
            .expect("summary text");
        let summary: Value = serde_json::from_str(summary_text).expect("process summary JSON");

        assert_eq!(summary["tool_key"], "session_show_defaults");
        assert_eq!(summary["status"], "succeeded");
        assert_eq!(summary["warning_count"], 0);
        assert!(
            summary["diagnostics"]
                .as_array()
                .expect("diagnostics")
                .is_empty()
        );
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
