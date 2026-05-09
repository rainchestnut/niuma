//! Codex app-server JSON-RPC client over stdio.
//!
//! The gateway owns one app-server process for its whole lifetime. Mobile
//! refresh requests reuse this client instead of spawning short-lived Codex
//! commands, which keeps the desktop source of truth inside Codex itself.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::time::{Duration, timeout};
use tracing::{debug, info, warn};

use crate::paths;

type PendingResponse = std::result::Result<Value, String>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelState {
    pub current_model: Option<String>,
    pub available_models: Vec<String>,
}

#[derive(Clone)]
pub struct CodexAppServerClient {
    inner: Arc<CodexAppServerInner>,
}

struct CodexAppServerInner {
    child: StdMutex<Child>,
    stdin: Mutex<ChildStdin>,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<PendingResponse>>>>,
    notifications: broadcast::Sender<Value>,
    next_id: AtomicU64,
}

impl CodexAppServerClient {
    /// Start Codex app-server and complete the JSON-RPC initialize handshake.
    pub async fn start(command: &[String]) -> Result<Self> {
        let (binary, args) = command
            .split_first()
            .context("codex app-server command is empty")?;
        info!(
            binary = %binary,
            arg_count = args.len(),
            "codex_app_server_start"
        );
        let mut child = Command::new(binary)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("failed to spawn {}", command.join(" ")))?;
        let stdin = child.stdin.take().context("app-server stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("app-server stdout unavailable")?;
        let stderr = child
            .stderr
            .take()
            .context("app-server stderr unavailable")?;
        spawn_stderr_logger(stderr);

        let (notifications, _) = broadcast::channel(256);
        let client = Self {
            inner: Arc::new(CodexAppServerInner {
                child: StdMutex::new(child),
                stdin: Mutex::new(stdin),
                pending: Arc::new(Mutex::new(HashMap::new())),
                notifications,
                next_id: AtomicU64::new(1),
            }),
        };
        spawn_stdout_reader(
            stdout,
            client.inner.pending.clone(),
            client.inner.notifications.clone(),
        );
        client.initialize().await?;
        let log_path = paths::runtime_dir()?.join("codex_app_server.initialized");
        std::fs::write(log_path, command.join(" "))?;
        info!("codex_app_server_initialized");
        Ok(client)
    }

    /// Subscribe to app-server notifications and requests emitted on stdout.
    pub fn subscribe_notifications(&self) -> broadcast::Receiver<Value> {
        self.inner.notifications.subscribe()
    }

    /// Send a JSON-RPC request and wait for its response.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        let request_id = format!("req-{}", self.inner.next_id.fetch_add(1, Ordering::SeqCst));
        let (sender, receiver) = oneshot::channel();
        self.inner
            .pending
            .lock()
            .await
            .insert(request_id.clone(), sender);
        let payload = json!({
            "id": request_id.clone(),
            "method": method,
            "params": params,
        });
        info!(
            app_server_request_id = %request_id,
            method,
            payload_bytes = payload.to_string().len(),
            "codex_app_server_request_out"
        );
        if let Err(err) = self.write_json_line(&payload).await {
            self.inner.pending.lock().await.remove(&request_id);
            warn!(
                app_server_request_id = %request_id,
                method,
                "codex_app_server_request_write_failed: {err:#}"
            );
            return Err(err);
        }
        let response = match timeout(Duration::from_secs(60), receiver).await {
            Ok(Ok(response)) => response,
            Ok(Err(_)) => {
                warn!(
                    app_server_request_id = %request_id,
                    method,
                    "codex_app_server_response_channel_closed"
                );
                return Err(anyhow!("app-server response channel closed: {method}"));
            }
            Err(_) => {
                self.inner.pending.lock().await.remove(&request_id);
                warn!(
                    app_server_request_id = %request_id,
                    method,
                    "codex_app_server_request_timeout"
                );
                return Err(anyhow!("app-server request timed out: {method}"));
            }
        };
        match response {
            Ok(response) => {
                info!(
                    app_server_request_id = %request_id,
                    method,
                    "codex_app_server_request_done"
                );
                Ok(response)
            }
            Err(message) => {
                warn!(
                    app_server_request_id = %request_id,
                    method,
                    error = %message,
                    "codex_app_server_request_failed"
                );
                Err(anyhow!(message))
            }
        }
    }

    /// Send a JSON-RPC notification without waiting for a response.
    pub async fn notify(&self, method: &str, params: Option<Value>) -> Result<()> {
        let mut payload = json!({ "method": method });
        if let Some(params) = params {
            payload["params"] = params;
        }
        info!(
            method,
            payload_bytes = payload.to_string().len(),
            "codex_app_server_notification_out"
        );
        self.write_json_line(&payload).await
    }

    /// Resolve an app-server request with a successful result.
    pub async fn respond(&self, request_id: Value, result: Value) -> Result<()> {
        info!(
            app_server_request_id = %request_id_string(&request_id),
            "codex_app_server_response_out"
        );
        self.write_json_line(&json!({
            "id": request_id,
            "result": result,
        }))
        .await
    }

    /// Resolve an app-server request with a JSON-RPC error.
    pub async fn respond_error(&self, request_id: Value, code: i64, message: &str) -> Result<()> {
        warn!(
            app_server_request_id = %request_id_string(&request_id),
            code,
            "codex_app_server_error_response_out"
        );
        self.write_json_line(&json!({
            "id": request_id,
            "error": {
                "code": code,
                "message": message,
            },
        }))
        .await
    }

    /// Return raw thread payloads from Codex's `thread/list` endpoint.
    pub async fn list_thread_payloads(&self, cwd: &str, archived: bool) -> Result<Vec<Value>> {
        let result = self
            .request(
                "thread/list",
                json!({
                    "cwd": cwd,
                    "archived": archived,
                }),
            )
            .await?;
        Ok(result
            .get("data")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Return one raw thread payload from Codex's `thread/read` endpoint.
    pub async fn read_thread_payload(&self, thread_id: &str, include_turns: bool) -> Result<Value> {
        let result = self
            .request(
                "thread/read",
                json!({
                    "threadId": thread_id,
                    "includeTurns": include_turns,
                }),
            )
            .await?;
        result
            .get("thread")
            .cloned()
            .context("thread/read response missing thread")
    }

    /// Start a new Codex thread, optionally binding it to a workspace cwd.
    pub async fn start_thread_payload(
        &self,
        cwd: Option<&str>,
        approval_policy: Option<&str>,
        approvals_reviewer: Option<&str>,
        sandbox_mode: Option<&str>,
    ) -> Result<Value> {
        let params = thread_start_params(cwd, approval_policy, approvals_reviewer, sandbox_mode);
        let result = self.request("thread/start", params).await?;
        result
            .get("thread")
            .cloned()
            .context("thread/start response missing thread")
    }

    /// Resume an existing Codex thread before starting a new turn or replay.
    pub async fn resume_thread_payload(
        &self,
        thread_id: &str,
        cwd: Option<&str>,
        approval_policy: Option<&str>,
        approvals_reviewer: Option<&str>,
        sandbox_mode: Option<&str>,
    ) -> Result<Value> {
        let params = thread_resume_params(
            thread_id,
            cwd,
            approval_policy,
            approvals_reviewer,
            sandbox_mode,
        );
        let result = self.request("thread/resume", params).await?;
        result
            .get("thread")
            .cloned()
            .context("thread/resume response missing thread")
    }

    /// Archive a Codex thread through the app-server source of truth.
    pub async fn archive_thread(&self, thread_id: &str) -> Result<()> {
        self.request(
            "thread/archive",
            json!({
                "threadId": thread_id,
            }),
        )
        .await?;
        Ok(())
    }

    /// Start a user turn with Codex-native input items.
    pub async fn start_turn_payload(
        &self,
        thread_id: &str,
        input_items: Vec<Value>,
        model: Option<&str>,
        effort: Option<&str>,
        approval_policy: Option<&str>,
        approvals_reviewer: Option<&str>,
        sandbox_mode: Option<&str>,
    ) -> Result<Value> {
        let params = turn_start_params(
            thread_id,
            input_items,
            model,
            effort,
            approval_policy,
            approvals_reviewer,
            sandbox_mode,
        );
        let result = self.request("turn/start", params).await?;
        result
            .get("turn")
            .cloned()
            .context("turn/start response missing turn")
    }

    /// Ask Codex for model metadata when the app-server exposes it.
    pub async fn model_state(&self) -> ModelState {
        for method in ["model/list", "models/list"] {
            let Ok(result) = self.request(method, json!({})).await else {
                continue;
            };
            let available_models = extract_model_ids(&result);
            if !available_models.is_empty() {
                let current_model = string_field(&result, "model")
                    .or_else(|| string_field(&result, "currentModel"));
                return ModelState {
                    current_model,
                    available_models,
                };
            }
        }
        ModelState {
            current_model: None,
            available_models: Vec::new(),
        }
    }

    async fn initialize(&self) -> Result<()> {
        self.request(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "niuma-cli",
                    "title": "Niuma Desktop Gateway",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "capabilities": {
                    "suppressedNotifications": [],
                },
            }),
        )
        .await?;
        self.notify("initialized", None).await
    }

    async fn write_json_line(&self, value: &Value) -> Result<()> {
        let mut stdin = self.inner.stdin.lock().await;
        stdin
            .write_all(serde_json::to_string(value)?.as_bytes())
            .await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        Ok(())
    }
}

impl Drop for CodexAppServerInner {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.start_kill();
        }
    }
}

fn spawn_stderr_logger(stderr: tokio::process::ChildStderr) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            debug!("codex app-server stderr: {line}");
        }
    });
}

fn spawn_stdout_reader(
    stdout: ChildStdout,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<PendingResponse>>>>,
    notifications: broadcast::Sender<Value>,
) {
    tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let payload: Value = match serde_json::from_str(&line) {
                Ok(payload) => payload,
                Err(err) => {
                    warn!(
                        line_bytes = line.len(),
                        "invalid_app_server_json_line: {err}"
                    );
                    continue;
                }
            };
            if let Some(response_id) = response_id(&payload) {
                info!(
                    app_server_request_id = %response_id,
                    has_error = payload.get("error").is_some(),
                    "codex_app_server_response_in"
                );
                if let Some(sender) = pending.lock().await.remove(&response_id) {
                    let _ = sender.send(response_payload(payload));
                    continue;
                }
            }
            let method = payload
                .get("method")
                .and_then(|value| value.as_str())
                .unwrap_or("unknown");
            info!(
                method = %method,
                has_id = payload.get("id").is_some(),
                payload_bytes = line.len(),
                "codex_app_server_notification_in"
            );
            debug!("codex app-server notification received");
            let _ = notifications.send(payload);
        }

        let mut pending = pending.lock().await;
        for (_, sender) in pending.drain() {
            let _ = sender.send(Err("app-server stdout closed".to_string()));
        }
    });
}

fn response_id(payload: &Value) -> Option<String> {
    if payload.get("method").is_some() {
        return None;
    }
    let id = payload.get("id")?;
    id.as_str()
        .map(str::to_string)
        .or_else(|| id.as_u64().map(|value| value.to_string()))
}

fn response_payload(payload: Value) -> PendingResponse {
    if let Some(error) = payload.get("error") {
        return Err(error.to_string());
    }
    Ok(payload.get("result").cloned().unwrap_or(Value::Null))
}

fn request_id_string(request_id: &Value) -> String {
    request_id
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| request_id.to_string())
}

fn extract_model_ids(payload: &Value) -> Vec<String> {
    let raw_models = payload
        .get("models")
        .or_else(|| payload.get("data"))
        .or_else(|| payload.get("availableModels"));
    let Some(raw_models) = raw_models else {
        return Vec::new();
    };
    let mut models = Vec::new();
    collect_model_ids(raw_models, &mut models);
    models
}

fn collect_model_ids(value: &Value, models: &mut Vec<String>) {
    match value {
        Value::String(model) => push_model_id(models, model),
        Value::Array(items) => {
            for item in items {
                collect_model_ids(item, models);
            }
        }
        Value::Object(map) => {
            if let Some(model) = map
                .get("id")
                .or_else(|| map.get("model"))
                .or_else(|| map.get("name"))
                .and_then(Value::as_str)
            {
                push_model_id(models, model);
            } else {
                for value in map.values() {
                    collect_model_ids(value, models);
                }
            }
        }
        _ => {}
    }
}

fn push_model_id(models: &mut Vec<String>, model: &str) {
    if !model.is_empty() && !models.iter().any(|existing| existing == model) {
        models.push(model.to_string());
    }
}

fn turn_start_params(
    thread_id: &str,
    input_items: Vec<Value>,
    model: Option<&str>,
    effort: Option<&str>,
    approval_policy: Option<&str>,
    approvals_reviewer: Option<&str>,
    sandbox_mode: Option<&str>,
) -> Value {
    let mut params = json!({
        "threadId": thread_id,
        "input": input_items,
    });
    if let Some(model) = model {
        params["model"] = json!(model);
    }
    if let Some(effort) = effort {
        params["effort"] = json!(effort);
    }
    apply_approval_overrides(
        &mut params,
        approval_policy,
        approvals_reviewer,
        sandbox_mode,
        SandboxOverrideShape::TurnStart,
    );
    params
}

fn thread_start_params(
    cwd: Option<&str>,
    approval_policy: Option<&str>,
    approvals_reviewer: Option<&str>,
    sandbox_mode: Option<&str>,
) -> Value {
    let mut params = json!({
        "serviceName": "niuma-cli",
    });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    apply_approval_overrides(
        &mut params,
        approval_policy,
        approvals_reviewer,
        sandbox_mode,
        SandboxOverrideShape::ThreadStartOrResume,
    );
    params
}

fn thread_resume_params(
    thread_id: &str,
    cwd: Option<&str>,
    approval_policy: Option<&str>,
    approvals_reviewer: Option<&str>,
    sandbox_mode: Option<&str>,
) -> Value {
    let mut params = json!({ "threadId": thread_id });
    if let Some(cwd) = cwd {
        params["cwd"] = json!(cwd);
    }
    apply_approval_overrides(
        &mut params,
        approval_policy,
        approvals_reviewer,
        sandbox_mode,
        SandboxOverrideShape::ThreadStartOrResume,
    );
    params
}

#[derive(Debug, Clone, Copy)]
enum SandboxOverrideShape {
    ThreadStartOrResume,
    TurnStart,
}

fn apply_approval_overrides(
    params: &mut Value,
    approval_policy: Option<&str>,
    approvals_reviewer: Option<&str>,
    sandbox_mode: Option<&str>,
    sandbox_shape: SandboxOverrideShape,
) {
    if let Some(approval_policy) = approval_policy {
        params["approvalPolicy"] = json!(approval_policy);
    }
    if let Some(approvals_reviewer) = approvals_reviewer {
        params["approvalsReviewer"] = json!(approvals_reviewer);
    }
    if let Some(sandbox_mode) = sandbox_mode {
        match sandbox_shape {
            SandboxOverrideShape::ThreadStartOrResume => {
                params["sandbox"] = json!(sandbox_mode);
            }
            SandboxOverrideShape::TurnStart => {
                params["sandboxPolicy"] = sandbox_policy_for_mode(sandbox_mode);
            }
        }
    }
}

fn sandbox_policy_for_mode(mode: &str) -> Value {
    match mode {
        "read-only" => json!({ "type": "readOnly" }),
        "workspace-write" => json!({ "type": "workspaceWrite" }),
        "danger-full-access" => json!({ "type": "dangerFullAccess" }),
        other => json!({ "type": other }),
    }
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_start_params_include_model_and_effort_overrides() {
        let params = turn_start_params(
            "thread-1",
            vec![json!({ "type": "text", "text": "hello" })],
            Some("gpt-5.5"),
            Some("xhigh"),
            None,
            None,
            None,
        );

        assert_eq!(params["threadId"], "thread-1");
        assert_eq!(params["model"], "gpt-5.5");
        assert_eq!(params["effort"], "xhigh");
        assert_eq!(params["input"][0]["text"], "hello");
    }

    #[test]
    fn mobile_permission_overrides_map_to_app_server_params() {
        let turn = turn_start_params(
            "thread-1",
            vec![json!({ "type": "text", "text": "hello" })],
            None,
            None,
            Some("on-request"),
            Some("guardian_subagent"),
            Some("workspace-write"),
        );

        assert_eq!(turn["approvalPolicy"], "on-request");
        assert_eq!(turn["approvalsReviewer"], "guardian_subagent");
        assert_eq!(turn["sandboxPolicy"]["type"], "workspaceWrite");

        let thread = thread_start_params(
            Some("/tmp/workspace"),
            Some("never"),
            None,
            Some("danger-full-access"),
        );

        assert_eq!(thread["approvalPolicy"], "never");
        assert_eq!(thread["sandbox"], "danger-full-access");
        assert_eq!(thread["cwd"], "/tmp/workspace");
    }

    #[test]
    fn thread_resume_omits_cwd_when_not_provided() {
        let params = thread_resume_params("thread-1", None, None, None, None);

        assert_eq!(params["threadId"], "thread-1");
        assert!(params.get("cwd").is_none());
    }
}
