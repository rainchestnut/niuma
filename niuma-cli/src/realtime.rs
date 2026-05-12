//! Agent WebSocket channel for server-routed mobile control messages.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use futures_util::{Sink, SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value, json};
use tokio::sync::{RwLock, broadcast};
use tokio::time::{Duration, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tracing::{info, warn};
use url::Url;

use crate::bindings::{self, PairedDeviceBinding};
use crate::codex_app_server::CodexAppServerClient;
use crate::config::GatewayConfig;
use crate::crypto;
use crate::diff_summary;
use crate::identity::AgentIdentity;
use crate::metadata::{self, CodexMetadataProjector, CodexThreadRecord, CodexWorkspaceStore};
use crate::pairing::PairingRuntimeState;
use crate::server::NiumaServerClient;
use crate::tasks::{
    ResumeThreadInbound, TaskInterruptInbound, TaskRuntime, TaskStartInbound, TaskSteerInbound,
};
use crate::thread_status::normalize_thread_status;
use crate::transfers::{self, TransferContext, TransferReady, TransferStore};

const CONVERSATION_PROJECT_ID: &str = "__conversation__";
const COMPLETED_USER_INPUT_RETENTION: Duration = Duration::from_secs(30 * 60);
const COMPLETED_USER_INPUT_LIMIT: usize = 128;

#[derive(Debug, Clone, Default, Serialize)]
pub struct AgentChannelStatus {
    pub connected: bool,
    pub last_connected_at: Option<i64>,
    pub last_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PairHandshakeMessage {
    request_id: String,
    device_id: String,
    agent_id: String,
    pair_token: String,
    binding_id: String,
    agent_pairing_public_key: String,
    encrypted_handshake: String,
}

#[derive(Debug, Deserialize)]
struct EncryptedHandshakeEnvelope {
    ios_encryption_public_key: String,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Deserialize)]
struct PairHandshakePlaintext {
    device_id: String,
    ios_encryption_public_key: String,
}

#[derive(Debug, Deserialize)]
struct MetadataRefreshMessage {
    request_id: String,
    device_id: String,
}

#[derive(Debug, Deserialize)]
struct BranchChangesRequest {
    request_id: String,
    device_id: String,
    thread_id: String,
    base_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ThreadArchiveRequest {
    request_id: String,
    device_id: String,
    thread_id: String,
}

#[derive(Debug, Deserialize)]
struct ThreadRenameRequest {
    request_id: String,
    device_id: String,
    thread_id: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct PairHandshakeAck {
    kind: &'static str,
    request_id: String,
    device_id: String,
    agent_id: String,
    pair_token: String,
    binding_id: String,
    handshake_hash: String,
    ack_status: String,
    signature: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct ActiveThread {
    device_id: String,
    cursor: i64,
    checkpoint: Option<String>,
    project_id: Option<String>,
    active_turn_id: Option<String>,
    last_pushed_completion: Option<String>,
}

#[derive(Debug, Clone)]
struct PendingApproval {
    approval_id: String,
    request_id: Value,
    method: String,
    thread_id: String,
    params: Value,
}

#[derive(Debug, Clone)]
struct PendingUserInput {
    request_id: String,
    app_server_request_id: Value,
    thread_id: String,
    response_format: UserInputResponseFormat,
}

#[derive(Debug, Clone)]
struct CompletedUserInput {
    thread_id: String,
    completed_at: Instant,
    response_payload: Option<Value>,
}

#[derive(Debug, Clone)]
enum UserInputClaim {
    New,
    AlreadyPending,
    AlreadyCompleted(Option<Value>),
}

/// Identifies how a mobile user-input answer must be translated back to the
/// Codex app-server request that originally blocked the turn.
#[derive(Debug, Clone)]
enum UserInputResponseFormat {
    CodexRequestUserInput,
    McpElicitation(McpElicitationResponseFormat),
}

#[derive(Debug, Clone)]
enum McpElicitationResponseFormat {
    Form { fields: Vec<McpElicitationField> },
    Url,
}

#[derive(Debug, Clone)]
struct McpElicitationField {
    id: String,
    value_kind: McpElicitationValueKind,
    required: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpElicitationValueKind {
    String,
    Number,
    Integer,
    Boolean,
    StringArray,
}

#[derive(Debug, Deserialize)]
struct EncryptedMobilePayload {
    device_id: String,
    ciphertext: String,
}

#[derive(Debug, Deserialize)]
struct ApprovalResponseInbound {
    #[serde(skip)]
    device_id: String,
    approval_id: String,
    decision: String,
    grant_scope: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct UserInputResponseInbound {
    #[serde(skip)]
    device_id: String,
    request_id: String,
    answers: Value,
}

/// Shared state used by one authenticated agent WebSocket connection loop.
struct AgentChannelRuntime {
    identity: AgentIdentity,
    config: GatewayConfig,
    pairing: Arc<RwLock<PairingRuntimeState>>,
    codex_app_server: Option<CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    transfer_context: Option<TransferContext>,
    pending_approvals: Arc<RwLock<HashMap<String, PendingApproval>>>,
    pending_user_inputs: Arc<RwLock<HashMap<String, PendingUserInput>>>,
    completed_user_inputs: Arc<RwLock<HashMap<String, CompletedUserInput>>>,
    pending_task_starts: Arc<RwLock<HashMap<String, VecDeque<TaskStartInbound>>>>,
    channel_status: Arc<RwLock<AgentChannelStatus>>,
}

/// Keep the authenticated agent WebSocket online in the background.
pub fn spawn_agent_channel(
    identity: AgentIdentity,
    config: GatewayConfig,
    session_token: Option<Arc<RwLock<String>>>,
    pairing: Arc<RwLock<PairingRuntimeState>>,
    codex_app_server: Option<CodexAppServerClient>,
    server: Option<NiumaServerClient>,
    channel_status: Arc<RwLock<AgentChannelStatus>>,
) {
    let Some(session_token) = session_token else {
        tokio::spawn(async move {
            channel_status.write().await.last_error =
                Some("gateway is not authenticated".to_string());
        });
        return;
    };
    let transfer_context = server.and_then(|server| {
        Some(TransferContext {
            server,
            session_token: session_token.clone(),
            agent_id: identity.agent_id.clone(),
            identity: identity.clone(),
            #[cfg(test)]
            test_bindings: Default::default(),
            store: TransferStore::open().ok()?,
        })
    });
    let active_threads = Arc::new(RwLock::new(HashMap::new()));
    let pending_approvals = Arc::new(RwLock::new(HashMap::new()));
    let pending_user_inputs = Arc::new(RwLock::new(HashMap::new()));
    let completed_user_inputs = Arc::new(RwLock::new(HashMap::new()));
    let pending_task_starts = Arc::new(RwLock::new(HashMap::new()));
    let runtime = Arc::new(AgentChannelRuntime {
        identity,
        config,
        pairing,
        codex_app_server,
        active_threads,
        transfer_context,
        pending_approvals,
        pending_user_inputs,
        completed_user_inputs,
        pending_task_starts,
        channel_status,
    });
    tokio::spawn(async move {
        loop {
            let current_session_token = session_token.read().await.clone();
            let result = run_agent_channel(&runtime, &current_session_token).await;
            {
                let mut status = runtime.channel_status.write().await;
                status.connected = false;
                status.last_error = Some(match &result {
                    Ok(()) => "agent websocket closed".to_string(),
                    Err(err) => err.to_string(),
                });
            }
            if let Err(err) = result {
                warn!("agent websocket disconnected: {err:#}");
            }
            sleep(Duration::from_secs(2)).await;
        }
    });
}

async fn run_agent_channel(runtime: &AgentChannelRuntime, session_token: &str) -> Result<()> {
    let url = agent_ws_url(
        &runtime.config.server_url,
        &runtime.identity.agent_id,
        session_token,
    )?;
    let (socket, _) = connect_async(url.as_str())
        .await
        .with_context(|| format!("failed to connect agent websocket {url}"))?;
    info!("agent websocket connected");
    {
        let mut status = runtime.channel_status.write().await;
        status.connected = true;
        status.last_connected_at = Some(unix_timestamp());
        status.last_error = None;
    }
    let (mut writer, mut reader) = socket.split();
    let app_server_requests = runtime.codex_app_server.clone();
    let mut notifications = runtime
        .codex_app_server
        .as_ref()
        .map(CodexAppServerClient::subscribe_notifications);
    loop {
        tokio::select! {
            request = next_app_server_request(app_server_requests.as_ref()), if app_server_requests.is_some() => {
                if let Some(request) = request {
                    handle_app_server_event(&mut writer, runtime, request).await?;
                }
            }
            message = reader.next() => {
                let Some(message) = message else {
                    break;
                };
                let message = message?;
                let Message::Text(text) = message else {
                    continue;
                };
                let value: serde_json::Value = serde_json::from_str(&text)?;
                handle_server_payload(&mut writer, runtime, value).await?;
            }
            notification = next_notification(&mut notifications), if notifications.is_some() => {
                if let Some(notification) = notification {
                    handle_app_server_event(&mut writer, runtime, notification).await?;
                }
            }
        }
    }
    Ok(())
}

async fn handle_server_payload<S, E>(
    writer: &mut S,
    runtime: &AgentChannelRuntime,
    value: Value,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let kind = wire_kind(&value).to_string();
    info!(
        kind = %kind,
        request_id = %wire_request_id(&value),
        device_id = %wire_device_id(&value),
        thread_id = %wire_thread_id(&value),
        cursor = ?wire_cursor(&value),
        entry_count = ?wire_entry_count(&value),
        payload_bytes = value.to_string().len(),
        "gateway_ws_in"
    );
    match kind.as_str() {
        "pair_handshake" => {
            let inbound: PairHandshakeMessage = serde_json::from_value(value)?;
            let ack =
                handle_pair_handshake(&runtime.identity, runtime.pairing.clone(), inbound).await;
            send_wire_message(writer, serde_json::to_value(ack)?).await?;
        }
        "metadata_refresh" => {
            let inbound: MetadataRefreshMessage = serde_json::from_value(value)?;
            handle_metadata_refresh(writer, runtime.codex_app_server.clone(), inbound).await?;
        }
        "task_start" => {
            let inbound: TaskStartInbound = serde_json::from_value(value)?;
            handle_task_start(
                writer,
                &runtime.identity,
                runtime.codex_app_server.clone(),
                runtime.active_threads.clone(),
                runtime.pending_task_starts.clone(),
                runtime.transfer_context.clone(),
                inbound,
            )
            .await?;
        }
        "task_steer" => {
            let inbound: TaskSteerInbound = serde_json::from_value(value)?;
            handle_task_steer(
                writer,
                &runtime.identity,
                runtime.codex_app_server.clone(),
                runtime.transfer_context.clone(),
                inbound,
            )
            .await?;
        }
        "task_interrupt" => {
            let inbound: TaskInterruptInbound = serde_json::from_value(value)?;
            handle_task_interrupt(
                writer,
                &runtime.identity,
                runtime.codex_app_server.as_ref(),
                runtime.active_threads.clone(),
                inbound,
            )
            .await?;
        }
        "resume_thread" => {
            let inbound: ResumeThreadInbound = serde_json::from_value(value)?;
            handle_resume_thread(
                writer,
                &runtime.identity,
                runtime.codex_app_server.clone(),
                runtime.active_threads.clone(),
                runtime.transfer_context.clone(),
                inbound,
            )
            .await?;
        }
        "branch_changes_request" => {
            let inbound: BranchChangesRequest = serde_json::from_value(value)?;
            handle_branch_changes_request(
                writer,
                &runtime.identity,
                runtime.codex_app_server.as_ref(),
                runtime.transfer_context.as_ref(),
                inbound,
            )
            .await?;
        }
        "thread_archive_request" => {
            let inbound: ThreadArchiveRequest = serde_json::from_value(value)?;
            handle_thread_archive_request(
                writer,
                runtime.codex_app_server.as_ref(),
                runtime.active_threads.clone(),
                inbound,
            )
            .await?;
        }
        "thread_rename_request" => {
            let inbound: ThreadRenameRequest = serde_json::from_value(value)?;
            handle_thread_rename_request(
                writer,
                runtime.codex_app_server.as_ref(),
                runtime.active_threads.clone(),
                inbound,
            )
            .await?;
        }
        "transfer_ready" => {
            let inbound: TransferReady = serde_json::from_value(value)?;
            handle_transfer_ready(runtime.transfer_context.as_ref(), inbound).await?;
        }
        "approval_response" => {
            let inbound = decrypt_approval_response(&runtime.identity, value)?;
            let response = handle_approval_response(
                runtime.codex_app_server.clone(),
                runtime.pending_approvals.clone(),
                inbound,
            )
            .await?;
            send_wire_message(writer, response).await?;
        }
        "user_input_response" => {
            let inbound = decrypt_user_input_response(&runtime.identity, value)?;
            if let Some(sync) = handle_user_input_response(
                runtime.codex_app_server.clone(),
                runtime.pending_user_inputs.clone(),
                runtime.completed_user_inputs.clone(),
                inbound,
            )
            .await?
            {
                send_wire_message(writer, sync).await?;
            }
        }
        _ if kind == "unknown" => warn!("agent websocket message missing kind"),
        _ => warn!("unsupported agent websocket message kind={kind}"),
    }
    Ok(())
}

async fn handle_task_start<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    codex_app_server: Option<CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    pending_task_starts: Arc<RwLock<HashMap<String, VecDeque<TaskStartInbound>>>>,
    transfer_context: Option<TransferContext>,
    inbound: TaskStartInbound,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let device_id = inbound.device_id.clone();
    let project_id = inbound.project_id.clone();
    let request_id = inbound.request_id.clone();
    info!(
        request_id = %request_id.as_deref().unwrap_or(""),
        device_id = %device_id,
        project_id = %project_id,
        thread_id = ?inbound.thread_id,
        "gateway_task_start_received"
    );
    let fallback_thread_id = inbound
        .thread_id
        .clone()
        .unwrap_or_else(|| "task-start-failed".to_string());
    let Some(app_server) = codex_app_server else {
        send_wire_message(
            writer,
            encrypt_task_update(
                identity,
                task_error_update(
                    &device_id,
                    &fallback_thread_id,
                    "Codex app-server is not connected",
                ),
            )?,
        )
        .await?;
        return Ok(());
    };

    if let Some(thread_id) = inbound.thread_id.clone()
        && should_queue_task_start(&app_server, active_threads.clone(), &thread_id).await
    {
        let queued_count = enqueue_task_start(pending_task_starts, &thread_id, inbound).await;
        send_wire_message(
            writer,
            task_queue_sync(&device_id, &thread_id, queued_count, "queued"),
        )
        .await?;
        return Ok(());
    }

    start_task_now(
        writer,
        identity,
        app_server,
        active_threads,
        transfer_context,
        inbound,
        &fallback_thread_id,
        request_id.as_deref(),
        &project_id,
    )
    .await?;
    Ok(())
}

async fn start_task_now<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    app_server: CodexAppServerClient,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    transfer_context: Option<TransferContext>,
    inbound: TaskStartInbound,
    fallback_thread_id: &str,
    request_id: Option<&str>,
    project_id: &str,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let device_id = inbound.device_id.clone();
    let requested_thread_id = inbound.thread_id.clone();
    match TaskRuntime::new(identity.clone(), app_server, transfer_context.clone())
        .start_task_messages(inbound)
        .await
    {
        Ok(messages) => {
            if let Some(thread_id) = requested_thread_id {
                mark_thread_running(active_threads.clone(), &thread_id, None).await;
            }
            send_wire_messages(
                writer,
                identity,
                messages,
                active_threads,
                transfer_context.as_ref(),
            )
            .await?;
        }
        Err(err) => {
            send_wire_message(
                writer,
                encrypt_task_update(
                    identity,
                    task_error_update(&device_id, fallback_thread_id, &err.to_string()),
                )?,
            )
            .await?;
            warn!(
                request_id = %request_id.unwrap_or(""),
                device_id = %device_id,
                project_id = %project_id,
                thread_id = %fallback_thread_id,
                "task_start_failed: {err:#}"
            );
        }
    }
    Ok(())
}

async fn handle_task_steer<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    codex_app_server: Option<CodexAppServerClient>,
    transfer_context: Option<TransferContext>,
    inbound: TaskSteerInbound,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let request_id = inbound.request_id.clone();
    let device_id = inbound.device_id.clone();
    let thread_id = inbound.thread_id.clone();
    let result = if let Some(app_server) = codex_app_server {
        TaskRuntime::new(identity.clone(), app_server, transfer_context)
            .steer_task(inbound)
            .await
    } else {
        Err(anyhow::anyhow!("Codex app-server is not connected"))
    };
    send_wire_message(
        writer,
        task_action_sync(
            "task_steer_sync",
            &device_id,
            request_id.as_deref(),
            &thread_id,
            result.as_ref().err().map(|error| error.to_string()),
        ),
    )
    .await
}

async fn handle_task_interrupt<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    inbound: TaskInterruptInbound,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let error = if inbound.agent_id != identity.agent_id {
        Some("task_interrupt agent_id does not match this gateway".to_string())
    } else {
        match app_server {
            Some(app_server) => match app_server.interrupt_turn(&inbound.thread_id).await {
                Ok(()) => {
                    mark_thread_idle(active_threads, &inbound.thread_id).await;
                    None
                }
                Err(error) => Some(error.to_string()),
            },
            None => Some("Codex app-server is not connected".to_string()),
        }
    };
    send_wire_message(
        writer,
        task_action_sync(
            "task_interrupt_sync",
            &inbound.device_id,
            inbound.request_id.as_deref(),
            &inbound.thread_id,
            error,
        ),
    )
    .await
}

async fn should_queue_task_start(
    app_server: &CodexAppServerClient,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    thread_id: &str,
) -> bool {
    if active_threads
        .read()
        .await
        .get(thread_id)
        .and_then(|active| active.active_turn_id.as_ref())
        .is_some()
    {
        return true;
    }
    app_server
        .read_thread_payload(thread_id, false)
        .await
        .ok()
        .map(|payload| normalize_thread_status(payload.get("status"), false))
        .is_some_and(|status| status == "running" || status == "waiting_approval")
}

async fn enqueue_task_start(
    pending_task_starts: Arc<RwLock<HashMap<String, VecDeque<TaskStartInbound>>>>,
    thread_id: &str,
    inbound: TaskStartInbound,
) -> usize {
    let mut pending = pending_task_starts.write().await;
    let queue = pending.entry(thread_id.to_string()).or_default();
    queue.push_back(inbound);
    queue.len()
}

async fn mark_thread_running(
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    thread_id: &str,
    turn_id: Option<String>,
) {
    let mut active = active_threads.write().await;
    if let Some(current) = active.get_mut(thread_id) {
        current.active_turn_id = turn_id.or_else(|| Some("mobile-started".to_string()));
    }
}

async fn mark_thread_idle(
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    thread_id: &str,
) {
    let mut active = active_threads.write().await;
    if let Some(current) = active.get_mut(thread_id) {
        current.active_turn_id = None;
    }
}

fn task_queue_sync(device_id: &str, thread_id: &str, queued_count: usize, status: &str) -> Value {
    json!({
        "kind": "task_queue_sync",
        "device_id": device_id,
        "thread_id": thread_id,
        "queued_count": queued_count,
        "status": status,
    })
}

fn task_action_sync(
    kind: &str,
    device_id: &str,
    request_id: Option<&str>,
    thread_id: &str,
    error: Option<String>,
) -> Value {
    let succeeded = error.is_none();
    json!({
        "kind": kind,
        "device_id": device_id,
        "request_id": request_id,
        "thread_id": thread_id,
        "succeeded": succeeded,
        "error": error,
    })
}

async fn handle_resume_thread<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    codex_app_server: Option<CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    transfer_context: Option<TransferContext>,
    inbound: ResumeThreadInbound,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let device_id = inbound.device_id.clone();
    let thread_id = inbound.thread_id.clone();
    let cursor = inbound.cursor;
    let checkpoint = inbound.checkpoint.clone();
    let request_id = inbound.request_id.clone();
    info!(
        request_id = %request_id.as_deref().unwrap_or(""),
        device_id = %device_id,
        thread_id = %thread_id,
        cursor,
        checkpoint_present = checkpoint.is_some(),
        "gateway_resume_thread_start"
    );
    let Some(app_server) = codex_app_server else {
        warn!(
            request_id = %request_id.as_deref().unwrap_or(""),
            device_id = %device_id,
            thread_id = %thread_id,
            cursor,
            "resume_thread_failed_app_server_unavailable"
        );
        send_wire_message(
            writer,
            thread_sync_failed(
                request_id.as_deref(),
                &device_id,
                &thread_id,
                cursor,
                checkpoint.as_deref(),
                "Codex app-server is not connected",
            ),
        )
        .await?;
        return Ok(());
    };
    match TaskRuntime::new(identity.clone(), app_server, transfer_context.clone())
        .resume_thread_messages(inbound)
        .await
    {
        Ok(messages) => {
            let entry_count = messages.len();
            let completed_cursor = messages
                .iter()
                .filter_map(|message| message.get("seq").and_then(|seq| seq.as_i64()))
                .max()
                .unwrap_or(cursor);
            send_wire_messages(
                writer,
                identity,
                messages,
                active_threads,
                transfer_context.as_ref(),
            )
            .await?;
            info!(
                request_id = %request_id.as_deref().unwrap_or(""),
                device_id = %device_id,
                thread_id = %thread_id,
                cursor = completed_cursor,
                entry_count,
                "gateway_resume_thread_done"
            );
            send_wire_message(
                writer,
                json!({
                    "kind": "thread_sync_completed",
                    "request_id": request_id,
                    "device_id": device_id,
                    "thread_id": thread_id,
                    "cursor": completed_cursor,
                    "checkpoint": checkpoint,
                    "entry_count": entry_count,
                }),
            )
            .await?;
        }
        Err(err) => {
            send_wire_message(
                writer,
                thread_sync_failed(
                    request_id.as_deref(),
                    &device_id,
                    &thread_id,
                    cursor,
                    checkpoint.as_deref(),
                    &err.to_string(),
                ),
            )
            .await?;
            warn!(
                request_id = %request_id.as_deref().unwrap_or(""),
                device_id = %device_id,
                thread_id = %thread_id,
                cursor,
                "resume_thread_failed: {err:#}"
            );
        }
    }
    Ok(())
}

async fn handle_branch_changes_request<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    app_server: Option<&CodexAppServerClient>,
    transfer_context: Option<&TransferContext>,
    inbound: BranchChangesRequest,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let result = branch_changes_payload(app_server, transfer_context, &inbound).await;
    let wire = match result {
        Ok(payload) => branch_changes_wire(identity, &inbound, "branch_changes_result", payload)?,
        Err(err) => {
            warn!(
                "branch_changes_request failed thread_id={} request_id={}: {err:#}",
                inbound.thread_id, inbound.request_id
            );
            branch_changes_wire(
                identity,
                &inbound,
                "branch_changes_failed",
                json!({ "error": err.to_string() }),
            )?
        }
    };
    send_wire_message(writer, wire).await
}

async fn branch_changes_payload(
    app_server: Option<&CodexAppServerClient>,
    transfer_context: Option<&TransferContext>,
    inbound: &BranchChangesRequest,
) -> Result<Value> {
    let cwd = branch_changes_cwd(app_server, &inbound.thread_id)
        .await
        .context("thread has no Git workspace")?;
    let changes = diff_summary::branch_changes(&cwd, inbound.base_ref.as_deref()).await?;
    let context = transfer_context.context("transfer context is unavailable")?;
    let bundle = serde_json::to_string_pretty(&changes.bundle)?;
    let (transfer_id, size_bytes) = context
        .upload_json_attachment(&inbound.device_id, bundle)
        .await?;
    Ok(json!({
        "summary": changes.summary,
        "files_summary": changes.files_summary,
        "transfer_id": transfer_id,
        "size_bytes": size_bytes,
        "base_ref": inbound.base_ref,
    }))
}

async fn branch_changes_cwd(
    app_server: Option<&CodexAppServerClient>,
    thread_id: &str,
) -> Option<String> {
    if let Some(app_server) = app_server
        && let Ok(payload) = app_server.read_thread_payload(thread_id, false).await
        && let Some(cwd) = string_field(&payload, "cwd")
    {
        return Some(cwd);
    }
    None
}

fn branch_changes_wire(
    identity: &AgentIdentity,
    inbound: &BranchChangesRequest,
    kind: &str,
    plaintext: Value,
) -> Result<Value> {
    let aad = crypto::payload_aad(&[
        ("kind", kind.to_string()),
        ("device_id", inbound.device_id.clone()),
        ("agent_id", identity.agent_id.clone()),
        ("request_id", inbound.request_id.clone()),
        ("thread_id", inbound.thread_id.clone()),
    ]);
    let ciphertext = encrypt_for_mobile(
        identity,
        &inbound.device_id,
        &serde_json::to_vec(&plaintext)?,
        &aad,
    )?;
    Ok(json!({
        "kind": kind,
        "device_id": inbound.device_id.clone(),
        "request_id": inbound.request_id.clone(),
        "thread_id": inbound.thread_id.clone(),
        "ciphertext": ciphertext,
    }))
}

async fn handle_thread_archive_request<S, E>(
    writer: &mut S,
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    inbound: ThreadArchiveRequest,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let result = archive_thread_payload(app_server, active_threads, &inbound).await;
    match result {
        Ok(messages) => {
            for message in messages {
                send_wire_message(writer, message).await?;
            }
        }
        Err(err) => {
            warn!(
                "thread_archive_request failed thread_id={} request_id={}: {err:#}",
                inbound.thread_id, inbound.request_id
            );
            send_wire_message(
                writer,
                thread_archive_failed_wire(&inbound, &err.to_string()),
            )
            .await?;
        }
    }
    Ok(())
}

/// Archives the source Codex thread and returns the mobile acknowledgement plus
/// the canonical archived metadata projection.
async fn archive_thread_payload(
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    inbound: &ThreadArchiveRequest,
) -> Result<Vec<Value>> {
    let app_server = app_server.context("codex app-server is unavailable")?;
    app_server.archive_thread(&inbound.thread_id).await?;
    let mut messages = vec![thread_archive_result_wire(inbound)];
    messages.extend(
        metadata_syncs_for_thread_id(
            Some(app_server),
            active_threads,
            &inbound.thread_id,
            Some("archived".to_string()),
        )
        .await?,
    );
    Ok(messages)
}

/// Builds the plain acknowledgement for a successful mobile archive request.
fn thread_archive_result_wire(inbound: &ThreadArchiveRequest) -> Value {
    json!({
        "kind": "thread_archive_result",
        "device_id": inbound.device_id.clone(),
        "request_id": inbound.request_id.clone(),
        "thread_id": inbound.thread_id.clone(),
    })
}

/// Builds the plain failure envelope when the archive request cannot reach Codex.
fn thread_archive_failed_wire(inbound: &ThreadArchiveRequest, error: &str) -> Value {
    json!({
        "kind": "thread_archive_failed",
        "device_id": inbound.device_id.clone(),
        "request_id": inbound.request_id.clone(),
        "thread_id": inbound.thread_id.clone(),
        "error": error,
    })
}

async fn handle_thread_rename_request<S, E>(
    writer: &mut S,
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    inbound: ThreadRenameRequest,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let result = rename_thread_payload(app_server, active_threads, &inbound).await;
    match result {
        Ok(messages) => {
            for message in messages {
                send_wire_message(writer, message).await?;
            }
        }
        Err(err) => {
            warn!(
                "thread_rename_request failed thread_id={} request_id={}: {err:#}",
                inbound.thread_id, inbound.request_id
            );
            send_wire_message(
                writer,
                thread_rename_failed_wire(&inbound, &err.to_string()),
            )
            .await?;
        }
    }
    Ok(())
}

/// Updates the source Codex thread title and returns the acknowledgement plus
/// a fresh metadata projection from Codex.
async fn rename_thread_payload(
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    inbound: &ThreadRenameRequest,
) -> Result<Vec<Value>> {
    let app_server = app_server.context("codex app-server is unavailable")?;
    let title = inbound.title.trim();
    if title.is_empty() {
        anyhow::bail!("thread title cannot be empty");
    }
    app_server
        .set_thread_name(&inbound.thread_id, title)
        .await?;
    let mut messages = vec![thread_rename_result_wire(inbound)];
    messages.extend(
        metadata_syncs_for_thread_id(Some(app_server), active_threads, &inbound.thread_id, None)
            .await?,
    );
    Ok(messages)
}

/// Builds the plain acknowledgement for a successful mobile rename request.
fn thread_rename_result_wire(inbound: &ThreadRenameRequest) -> Value {
    json!({
        "kind": "thread_rename_result",
        "device_id": inbound.device_id.clone(),
        "request_id": inbound.request_id.clone(),
        "thread_id": inbound.thread_id.clone(),
    })
}

/// Builds the plain failure envelope when the title cannot be updated in Codex.
fn thread_rename_failed_wire(inbound: &ThreadRenameRequest, error: &str) -> Value {
    json!({
        "kind": "thread_rename_failed",
        "device_id": inbound.device_id.clone(),
        "request_id": inbound.request_id.clone(),
        "thread_id": inbound.thread_id.clone(),
        "error": error,
    })
}

async fn handle_app_server_event<S, E>(
    writer: &mut S,
    runtime: &AgentChannelRuntime,
    event: Value,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    if is_app_server_request(&event) {
        handle_app_server_request(
            writer,
            runtime.codex_app_server.clone(),
            &runtime.identity,
            runtime.active_threads.clone(),
            runtime.pending_approvals.clone(),
            runtime.pending_user_inputs.clone(),
            runtime.completed_user_inputs.clone(),
            event,
        )
        .await?;
        return Ok(());
    }
    for sync in resolved_request_syncs(
        &event,
        runtime.pending_approvals.clone(),
        runtime.pending_user_inputs.clone(),
        runtime.completed_user_inputs.clone(),
    )
    .await
    {
        send_wire_message(writer, sync).await?;
    }
    let completed_thread_id =
        apply_turn_lifecycle_notification(runtime.active_threads.clone(), &event).await;
    match notification_metadata_syncs(
        runtime.codex_app_server.as_ref(),
        runtime.active_threads.clone(),
        &event,
    )
    .await
    {
        Ok(syncs) => {
            for sync in syncs {
                send_wire_message(writer, sync).await?;
            }
        }
        Err(err) => warn!("app-server metadata notification projection failed: {err:#}"),
    }
    let Some(thread_id) = notification_thread_id(&event) else {
        return Ok(());
    };
    let Some(active) = runtime.active_threads.read().await.get(&thread_id).cloned() else {
        return Ok(());
    };
    let device_id = active.device_id.clone();
    let cursor = active.cursor;
    let checkpoint = active.checkpoint.clone();
    if let Some(app_server) = runtime.codex_app_server.clone() {
        let inbound = ResumeThreadInbound {
            request_id: None,
            device_id: device_id.clone(),
            thread_id: thread_id.clone(),
            cursor,
            checkpoint: checkpoint.clone(),
        };
        match TaskRuntime::new(
            runtime.identity.clone(),
            app_server,
            runtime.transfer_context.clone(),
        )
        .resume_thread_messages(inbound)
        .await
        {
            Ok(messages) if !messages.is_empty() => {
                let completion = thread_sync_completed(
                    None,
                    &device_id,
                    &thread_id,
                    cursor,
                    checkpoint.as_deref(),
                    &messages,
                );
                send_wire_messages(
                    writer,
                    &runtime.identity,
                    messages,
                    runtime.active_threads.clone(),
                    runtime.transfer_context.as_ref(),
                )
                .await?;
                send_wire_message(writer, completion).await?;
            }
            Ok(_) => {}
            Err(err) => warn!("app-server notification projection failed: {err:#}"),
        }
    }
    // Keep the sync ACK above independent: only Codex's terminal turn event may
    // request an APNs wakeup.
    maybe_send_task_progress_push(
        writer,
        &runtime.identity,
        runtime.active_threads.clone(),
        &event,
        &device_id,
        &thread_id,
    )
    .await?;
    if let Some(thread_id) = completed_thread_id {
        start_next_queued_task(writer, runtime, &thread_id).await?;
    }
    Ok(())
}

async fn apply_turn_lifecycle_notification(
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    notification: &Value,
) -> Option<String> {
    let thread_id = notification_thread_id(notification)?;
    match notification_method(notification) {
        Some("turn/started") => {
            mark_thread_running(
                active_threads,
                &thread_id,
                notification_turn_id(notification),
            )
            .await;
            None
        }
        Some("turn/completed") => {
            mark_thread_idle(active_threads, &thread_id).await;
            Some(thread_id)
        }
        _ => None,
    }
}

async fn start_next_queued_task<S, E>(
    writer: &mut S,
    runtime: &AgentChannelRuntime,
    thread_id: &str,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let Some(app_server) = runtime.codex_app_server.clone() else {
        return Ok(());
    };
    let next = {
        let mut pending = runtime.pending_task_starts.write().await;
        let Some(queue) = pending.get_mut(thread_id) else {
            return Ok(());
        };
        let next = queue.pop_front();
        let queued_count = queue.len();
        let remove_empty = queue.is_empty();
        if remove_empty {
            pending.remove(thread_id);
        }
        next.map(|inbound| (inbound, queued_count))
    };
    let Some((inbound, queued_count)) = next else {
        return Ok(());
    };
    let device_id = inbound.device_id.clone();
    let project_id = inbound.project_id.clone();
    let fallback_thread_id = inbound
        .thread_id
        .clone()
        .unwrap_or_else(|| thread_id.to_string());
    let request_id = inbound.request_id.clone();
    send_wire_message(
        writer,
        task_queue_sync(&device_id, thread_id, queued_count, "started"),
    )
    .await?;
    start_task_now(
        writer,
        &runtime.identity,
        app_server,
        runtime.active_threads.clone(),
        runtime.transfer_context.clone(),
        inbound,
        &fallback_thread_id,
        request_id.as_deref(),
        &project_id,
    )
    .await
}

async fn maybe_send_task_progress_push<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    notification: &Value,
    device_id: &str,
    thread_id: &str,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let Some(marker) = task_progress_push_marker(notification) else {
        if is_task_progress_push_trigger(notification) {
            warn!(
                thread_id = %thread_id,
                device_id = %device_id,
                "turn_completed_push_skipped_missing_turn_id"
            );
        }
        return Ok(());
    };
    if !claim_task_progress_push(active_threads, thread_id, &marker).await {
        info!(
            thread_id = %thread_id,
            device_id = %device_id,
            marker = %marker,
            "task_progress_push_skip_duplicate"
        );
        return Ok(());
    }
    info!(
        thread_id = %thread_id,
        device_id = %device_id,
        marker = %marker,
        "task_progress_push_emit"
    );
    send_wire_message(
        writer,
        task_progress_push_wire(identity, device_id, thread_id)?,
    )
    .await
}

async fn claim_task_progress_push(
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    thread_id: &str,
    marker: &str,
) -> bool {
    let mut active = active_threads.write().await;
    let Some(current) = active.get_mut(thread_id) else {
        return false;
    };
    if current.last_pushed_completion.as_deref() == Some(marker) {
        return false;
    }
    current.last_pushed_completion = Some(marker.to_string());
    true
}

async fn notification_metadata_syncs(
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    notification: &Value,
) -> Result<Vec<Value>> {
    let Some(method) = notification_method(notification) else {
        return Ok(Vec::new());
    };
    match method {
        "thread/started" => {
            let Some(thread_payload) = notification_thread_payload(notification) else {
                return Ok(Vec::new());
            };
            let record =
                metadata_record_from_payload(thread_payload, None, active_threads, None).await;
            Ok(metadata::thread_metadata_wire_messages(&record))
        }
        "thread/status/changed" => {
            let Some(thread_id) = notification_thread_id(notification) else {
                return Ok(Vec::new());
            };
            let status = normalize_thread_status(notification_status(notification), false);
            metadata_syncs_for_thread_id(app_server, active_threads, &thread_id, Some(status)).await
        }
        "thread/archived" => {
            let Some(thread_id) = notification_thread_id(notification) else {
                return Ok(Vec::new());
            };
            metadata_syncs_for_thread_id(
                app_server,
                active_threads,
                &thread_id,
                Some("archived".to_string()),
            )
            .await
        }
        "thread/unarchived" => {
            let Some(thread_id) = notification_thread_id(notification) else {
                return Ok(Vec::new());
            };
            metadata_syncs_for_thread_id(app_server, active_threads, &thread_id, None).await
        }
        "thread/name/updated" => {
            let Some(thread_id) = notification_thread_id(notification) else {
                return Ok(Vec::new());
            };
            metadata_syncs_for_thread_id(app_server, active_threads, &thread_id, None).await
        }
        "thread/closed" => {
            let Some(thread_id) = notification_thread_id(notification) else {
                return Ok(Vec::new());
            };
            metadata_syncs_for_thread_id(
                app_server,
                active_threads,
                &thread_id,
                Some("closed".to_string()),
            )
            .await
        }
        _ => Ok(Vec::new()),
    }
}

async fn metadata_syncs_for_thread_id(
    app_server: Option<&CodexAppServerClient>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    thread_id: &str,
    status_override: Option<String>,
) -> Result<Vec<Value>> {
    let thread_payload = match app_server {
        Some(app_server) => app_server
            .read_thread_payload(thread_id, false)
            .await
            .unwrap_or_else(|_| json!({ "id": thread_id })),
        None => json!({ "id": thread_id }),
    };
    let record = metadata_record_from_payload(
        &thread_payload,
        Some(thread_id),
        active_threads,
        status_override,
    )
    .await;
    Ok(metadata::thread_metadata_wire_messages(&record))
}

async fn metadata_record_from_payload(
    payload: &Value,
    fallback_thread_id: Option<&str>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    status_override: Option<String>,
) -> CodexThreadRecord {
    let thread_id = string_field(payload, "id")
        .or_else(|| fallback_thread_id.map(str::to_string))
        .unwrap_or_else(|| "unknown-thread".to_string());
    let active = active_threads.read().await.get(&thread_id).cloned();
    let cwd = string_field(payload, "cwd");
    let workspace_store = CodexWorkspaceStore::new();
    let project_id = active
        .as_ref()
        .and_then(|active| active.project_id.clone())
        .or_else(|| {
            cwd.as_deref()
                .and_then(|cwd| workspace_store.project_for_cwd(cwd))
                .map(|project| project.project_id)
        })
        .unwrap_or_else(|| CONVERSATION_PROJECT_ID.to_string());
    let title = string_field(payload, "name")
        .or_else(|| string_field(payload, "preview"))
        .unwrap_or_else(|| thread_id.clone());
    CodexThreadRecord {
        thread_id,
        project_id,
        current_branch: cwd.as_deref().and_then(metadata::current_git_branch),
        cwd,
        title,
        status: status_override
            .unwrap_or_else(|| normalize_thread_status(payload.get("status"), false)),
        last_checkpoint_seen: None,
        updated_at: number_field(payload, "updatedAt").or_else(|| Some(unix_timestamp() as f64)),
    }
}

async fn handle_transfer_ready(
    transfer_context: Option<&TransferContext>,
    inbound: TransferReady,
) -> Result<()> {
    let Some(transfer_context) = transfer_context else {
        return Ok(());
    };
    if let Err(err) = transfer_context.handle_transfer_ready(&inbound).await {
        warn!(
            "transfer_ready cache failed transfer_id={} direction={}: {err:#}",
            inbound.transfer_id, inbound.direction
        );
    }
    Ok(())
}

async fn handle_app_server_request<S, E>(
    writer: &mut S,
    codex_app_server: Option<CodexAppServerClient>,
    identity: &AgentIdentity,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    pending_approvals: Arc<RwLock<HashMap<String, PendingApproval>>>,
    pending_user_inputs: Arc<RwLock<HashMap<String, PendingUserInput>>>,
    completed_user_inputs: Arc<RwLock<HashMap<String, CompletedUserInput>>>,
    request: Value,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let Some(app_server) = codex_app_server else {
        return Ok(());
    };
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let request_id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if method == "item/tool/requestUserInput" {
        if let Some((message, pending)) = build_user_input_request(&request_id, &params).await {
            forward_user_input_request_once(
                writer,
                app_server.clone(),
                identity,
                active_threads.clone(),
                pending_user_inputs.clone(),
                completed_user_inputs.clone(),
                message,
                pending,
            )
            .await?;
        } else {
            app_server
                .respond_error(request_id, -32600, "invalid request_user_input payload")
                .await?;
        }
        return Ok(());
    }
    if method == "mcpServer/elicitation/request" {
        if let Some((message, pending)) = build_mcp_elicitation_request(&request_id, &params).await
        {
            forward_user_input_request_once(
                writer,
                app_server.clone(),
                identity,
                active_threads.clone(),
                pending_user_inputs.clone(),
                completed_user_inputs.clone(),
                message,
                pending,
            )
            .await?;
        } else {
            app_server
                .respond_error(request_id, -32600, "invalid mcp elicitation payload")
                .await?;
        }
        return Ok(());
    }
    if let Some(approval_type) = approval_type_for_method(&method) {
        if let Some((message, pending)) =
            build_approval_request(&request_id, &method, approval_type, &params).await
        {
            pending_approvals
                .write()
                .await
                .insert(pending.approval_id.clone(), pending);
            if let Some(device_id) =
                active_device_for_thread(&active_threads, &message.thread_id).await
            {
                send_wire_message(
                    writer,
                    approval_request_wire(identity, &device_id, &message)?,
                )
                .await?;
            }
        } else {
            app_server
                .respond_error(request_id, -32600, "invalid approval payload")
                .await?;
        }
        return Ok(());
    }
    app_server
        .respond(
            request_id,
            json!({
                "unsupported": true,
                "method": method,
            }),
        )
        .await?;
    Ok(())
}

async fn forward_user_input_request_once<S, E>(
    writer: &mut S,
    app_server: CodexAppServerClient,
    identity: &AgentIdentity,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    pending_user_inputs: Arc<RwLock<HashMap<String, PendingUserInput>>>,
    completed_user_inputs: Arc<RwLock<HashMap<String, CompletedUserInput>>>,
    message: UserInputWireData,
    pending: PendingUserInput,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    match claim_user_input_request(pending_user_inputs, completed_user_inputs, pending.clone())
        .await
    {
        UserInputClaim::New => {}
        UserInputClaim::AlreadyPending => return Ok(()),
        UserInputClaim::AlreadyCompleted(Some(response_payload)) => {
            app_server
                .respond(pending.app_server_request_id.clone(), response_payload)
                .await?;
            return Ok(());
        }
        UserInputClaim::AlreadyCompleted(None) => return Ok(()),
    }
    if let Some(device_id) = active_device_for_thread(&active_threads, &message.thread_id).await {
        send_wire_message(
            writer,
            user_input_request_wire(identity, &device_id, &message)?,
        )
        .await?;
    }
    Ok(())
}

/// Claims a user-input request before it is sent to mobile. Codex app-server can
/// replay the same elicitation while a turn is blocked; the mobile UI must see
/// one lifecycle event, not a stream of duplicate pending cards.
async fn claim_user_input_request(
    pending_user_inputs: Arc<RwLock<HashMap<String, PendingUserInput>>>,
    completed_user_inputs: Arc<RwLock<HashMap<String, CompletedUserInput>>>,
    pending: PendingUserInput,
) -> UserInputClaim {
    {
        let mut completed = completed_user_inputs.write().await;
        prune_completed_user_inputs(&mut completed);
        if let Some(completed) = completed.get(&pending.request_id) {
            if completed.matches(&pending) {
                return UserInputClaim::AlreadyCompleted(completed.response_payload.clone());
            }
        }
    }

    let mut user_inputs = pending_user_inputs.write().await;
    if user_inputs.contains_key(&pending.request_id) {
        return UserInputClaim::AlreadyPending;
    }
    user_inputs.insert(pending.request_id.clone(), pending);
    UserInputClaim::New
}

fn mark_user_input_completed(
    completed_user_inputs: &mut HashMap<String, CompletedUserInput>,
    pending: &PendingUserInput,
    response_payload: Option<Value>,
) {
    prune_completed_user_inputs(completed_user_inputs);
    completed_user_inputs.insert(
        pending.request_id.clone(),
        CompletedUserInput {
            thread_id: pending.thread_id.clone(),
            completed_at: Instant::now(),
            response_payload,
        },
    );
    while completed_user_inputs.len() > COMPLETED_USER_INPUT_LIMIT {
        if let Some(oldest_key) = completed_user_inputs
            .iter()
            .min_by_key(|(_, completed)| completed.completed_at)
            .map(|(key, _)| key.clone())
        {
            completed_user_inputs.remove(&oldest_key);
        } else {
            break;
        }
    }
}

fn prune_completed_user_inputs(completed_user_inputs: &mut HashMap<String, CompletedUserInput>) {
    let now = Instant::now();
    completed_user_inputs.retain(|_, completed| {
        now.duration_since(completed.completed_at) <= COMPLETED_USER_INPUT_RETENTION
    });
}

impl CompletedUserInput {
    fn matches(&self, pending: &PendingUserInput) -> bool {
        self.thread_id == pending.thread_id
    }
}

async fn handle_approval_response(
    codex_app_server: Option<CodexAppServerClient>,
    pending_approvals: Arc<RwLock<HashMap<String, PendingApproval>>>,
    inbound: ApprovalResponseInbound,
) -> Result<Value> {
    let Some(app_server) = codex_app_server else {
        return Ok(approval_response_failed(
            &inbound.device_id,
            &inbound.approval_id,
            "desktop app-server is not connected",
        ));
    };
    let Some(pending) = pending_approvals.write().await.remove(&inbound.approval_id) else {
        return Ok(approval_response_failed(
            &inbound.device_id,
            &inbound.approval_id,
            "approval request is no longer pending",
        ));
    };
    if let Err(error) = submit_approval_response(&app_server, &pending, &inbound).await {
        pending_approvals
            .write()
            .await
            .insert(pending.approval_id.clone(), pending.clone());
        return Ok(approval_response_failed(
            &inbound.device_id,
            &inbound.approval_id,
            &format!("approval response failed: {error:#}"),
        ));
    }
    Ok(approval_sync(
        &pending.approval_id,
        &pending.thread_id,
        approval_type_for_method(&pending.method).unwrap_or("unknown"),
        "resolved",
    ))
}

async fn submit_approval_response(
    app_server: &CodexAppServerClient,
    pending: &PendingApproval,
    inbound: &ApprovalResponseInbound,
) -> Result<()> {
    match pending.method.as_str() {
        "item/commandExecution/requestApproval" | "item/fileChange/requestApproval" => {
            app_server
                .respond(
                    pending.request_id.clone(),
                    json!({ "decision": approval_decision(inbound) }),
                )
                .await?;
        }
        "item/permissions/requestApproval" => {
            if inbound.decision != "allow" {
                app_server
                    .respond_error(
                        pending.request_id.clone(),
                        -32001,
                        "permission request rejected by mobile client",
                    )
                    .await?;
            } else {
                app_server
                    .respond(
                        pending.request_id.clone(),
                        json!({
                            "permissions": pending.params.get("permissions").cloned().unwrap_or_else(|| json!({})),
                            "scope": permission_grant_scope(inbound),
                        }),
                    )
                    .await?;
            }
        }
        _ => {
            app_server
                .respond_error(
                    pending.request_id.clone(),
                    -32601,
                    "unsupported approval method",
                )
                .await?;
        }
    }
    Ok(())
}

async fn handle_user_input_response(
    codex_app_server: Option<CodexAppServerClient>,
    pending_user_inputs: Arc<RwLock<HashMap<String, PendingUserInput>>>,
    completed_user_inputs: Arc<RwLock<HashMap<String, CompletedUserInput>>>,
    inbound: UserInputResponseInbound,
) -> Result<Option<Value>> {
    let Some(app_server) = codex_app_server else {
        return Ok(Some(user_input_response_failed(
            &inbound.device_id,
            &inbound.request_id,
            "desktop app-server is not connected",
        )));
    };
    let Some(pending) = pending_user_inputs
        .write()
        .await
        .remove(&inbound.request_id)
    else {
        return Ok(Some(user_input_response_failed(
            &inbound.device_id,
            &inbound.request_id,
            "user input request is no longer pending",
        )));
    };
    let response_payload = match user_input_app_server_response(&pending, &inbound.answers) {
        Ok(payload) => payload,
        Err(error) => {
            pending_user_inputs
                .write()
                .await
                .insert(pending.request_id.clone(), pending.clone());
            return Ok(Some(user_input_response_failed(
                &inbound.device_id,
                &inbound.request_id,
                &format!("user input response failed: {error:#}"),
            )));
        }
    };
    if let Err(error) = app_server
        .respond(
            pending.app_server_request_id.clone(),
            response_payload.clone(),
        )
        .await
    {
        pending_user_inputs
            .write()
            .await
            .insert(pending.request_id.clone(), pending.clone());
        return Ok(Some(user_input_response_failed(
            &inbound.device_id,
            &inbound.request_id,
            &format!("user input response failed: {error:#}"),
        )));
    }
    {
        let mut completed = completed_user_inputs.write().await;
        mark_user_input_completed(&mut completed, &pending, Some(response_payload));
    }
    Ok(Some(user_input_sync(
        &pending.request_id,
        &pending.thread_id,
        "resolved",
    )))
}

async fn handle_metadata_refresh<S, E>(
    writer: &mut S,
    codex_app_server: Option<CodexAppServerClient>,
    inbound: MetadataRefreshMessage,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    info!(
        request_id = %inbound.request_id,
        device_id = %inbound.device_id,
        "gateway_metadata_refresh_start"
    );
    let Some(app_server) = codex_app_server else {
        warn!(
            request_id = %inbound.request_id,
            device_id = %inbound.device_id,
            "metadata_refresh_failed_app_server_unavailable"
        );
        send_wire_message(
            writer,
            json!({
                "kind": "metadata_refresh_failed",
                "device_id": inbound.device_id,
                "request_id": inbound.request_id,
                "error": "Codex app-server is not connected",
            }),
        )
        .await?;
        return Ok(());
    };

    match CodexMetadataProjector::new(app_server).snapshot().await {
        Ok(snapshot) => {
            for message in metadata::snapshot_wire_messages(&snapshot) {
                send_wire_message(writer, message).await?;
            }
            send_wire_message(
                writer,
                json!({
                    "kind": "metadata_refresh_completed",
                    "device_id": inbound.device_id,
                    "request_id": inbound.request_id,
                    "project_count": snapshot.projects.len(),
                    "thread_count": snapshot.threads.len(),
                    "approval_count": 0,
                }),
            )
            .await?;
            info!(
                request_id = %inbound.request_id,
                device_id = %inbound.device_id,
                project_count = snapshot.projects.len(),
                thread_count = snapshot.threads.len(),
                "gateway_metadata_refresh_done"
            );
        }
        Err(err) => {
            send_wire_message(
                writer,
                json!({
                    "kind": "metadata_refresh_failed",
                    "device_id": inbound.device_id,
                    "request_id": inbound.request_id,
                    "error": err.to_string(),
                }),
            )
            .await?;
            warn!(
                request_id = %inbound.request_id,
                device_id = %inbound.device_id,
                "metadata_refresh_failed: {err:#}"
            );
        }
    }
    Ok(())
}

fn task_error_update(device_id: &str, thread_id: &str, error: &str) -> serde_json::Value {
    json!({
        "kind": "task_update",
        "device_id": device_id,
        "thread_id": thread_id,
        "seq": 1,
        "ciphertext": error,
        "checkpoint": null,
        "role": "system",
        "type": "systemMessage",
        "phase": "failed",
        "project_id": null,
        "entry_id": format!("{thread_id}:error"),
        "created_at": null,
    })
}

#[derive(Debug, Clone)]
struct ApprovalWireData {
    approval_id: String,
    thread_id: String,
    approval_type: String,
    ciphertext: String,
}

#[derive(Debug, Clone)]
struct UserInputWireData {
    request_id: String,
    thread_id: String,
    questions: Vec<Value>,
}

async fn active_device_for_thread(
    active_threads: &Arc<RwLock<HashMap<String, ActiveThread>>>,
    thread_id: &str,
) -> Option<String> {
    active_threads
        .read()
        .await
        .get(thread_id)
        .map(|active| active.device_id.clone())
}

async fn build_approval_request(
    app_server_request_id: &Value,
    method: &str,
    approval_type: &str,
    params: &Value,
) -> Option<(ApprovalWireData, PendingApproval)> {
    let thread_id = params.get("threadId")?.as_str()?.to_string();
    let approval_id = params
        .get("approvalId")
        .or_else(|| params.get("itemId"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| request_id_string(app_server_request_id));
    let pending = PendingApproval {
        approval_id: approval_id.clone(),
        request_id: app_server_request_id.clone(),
        method: method.to_string(),
        thread_id: thread_id.clone(),
        params: params.clone(),
    };
    let message = ApprovalWireData {
        approval_id,
        thread_id,
        approval_type: approval_type.to_string(),
        ciphertext: serde_json::to_string(&json!({
            "method": method,
            "params": params,
        }))
        .ok()?,
    };
    Some((message, pending))
}

async fn build_user_input_request(
    app_server_request_id: &Value,
    params: &Value,
) -> Option<(UserInputWireData, PendingUserInput)> {
    let thread_id = params.get("threadId")?.as_str()?.to_string();
    let request_id = params
        .get("itemId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| request_id_string(app_server_request_id));
    let questions = params
        .get("questions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(project_user_input_question)
        .collect();
    let pending = PendingUserInput {
        request_id: request_id.clone(),
        app_server_request_id: app_server_request_id.clone(),
        thread_id: thread_id.clone(),
        response_format: UserInputResponseFormat::CodexRequestUserInput,
    };
    Some((
        UserInputWireData {
            request_id,
            thread_id,
            questions,
        },
        pending,
    ))
}

/// Projects Codex MCP elicitation callbacks into the existing mobile
/// user-input protocol while retaining enough schema metadata to answer the
/// original JSON-RPC request later.
async fn build_mcp_elicitation_request(
    app_server_request_id: &Value,
    params: &Value,
) -> Option<(UserInputWireData, PendingUserInput)> {
    let thread_id = params.get("threadId")?.as_str()?.to_string();
    match params.get("mode")?.as_str()? {
        "form" => build_mcp_form_elicitation_request(app_server_request_id, params, thread_id),
        "url" => build_mcp_url_elicitation_request(app_server_request_id, params, thread_id),
        _ => None,
    }
}

fn build_mcp_form_elicitation_request(
    app_server_request_id: &Value,
    params: &Value,
    thread_id: String,
) -> Option<(UserInputWireData, PendingUserInput)> {
    let request_id = request_id_string(app_server_request_id);
    let message = params.get("message").and_then(Value::as_str).unwrap_or("");
    let schema = params.get("requestedSchema")?;
    let properties = schema.get("properties")?.as_object()?;
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|required| {
            required
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let mut fields = Vec::new();
    let mut questions = Vec::new();
    for (index, (field_id, field_schema)) in properties.iter().enumerate() {
        let value_kind = mcp_field_value_kind(field_schema)?;
        let required = required.iter().any(|required| required == field_id);
        fields.push(McpElicitationField {
            id: field_id.clone(),
            value_kind,
            required,
        });
        questions.push(project_mcp_elicitation_question(
            field_id,
            field_schema,
            message,
            index == 0,
        ));
    }
    if questions.is_empty() {
        questions.push(mcp_elicitation_confirmation_question(
            "confirm",
            params.get("serverName").and_then(Value::as_str),
            message,
        ));
    }

    let pending = PendingUserInput {
        request_id: request_id.clone(),
        app_server_request_id: app_server_request_id.clone(),
        thread_id: thread_id.clone(),
        response_format: UserInputResponseFormat::McpElicitation(
            McpElicitationResponseFormat::Form { fields },
        ),
    };
    Some((
        UserInputWireData {
            request_id,
            thread_id,
            questions,
        },
        pending,
    ))
}

fn build_mcp_url_elicitation_request(
    app_server_request_id: &Value,
    params: &Value,
    thread_id: String,
) -> Option<(UserInputWireData, PendingUserInput)> {
    let request_id = params
        .get("elicitationId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| request_id_string(app_server_request_id));
    let message = params.get("message").and_then(Value::as_str).unwrap_or("");
    let url = params.get("url")?.as_str()?;
    let prompt = join_prompt(message, Some(url));
    let questions = vec![mcp_elicitation_confirmation_question(
        "action",
        params.get("serverName").and_then(Value::as_str),
        &prompt,
    )];
    let pending = PendingUserInput {
        request_id: request_id.clone(),
        app_server_request_id: app_server_request_id.clone(),
        thread_id: thread_id.clone(),
        response_format: UserInputResponseFormat::McpElicitation(McpElicitationResponseFormat::Url),
    };
    Some((
        UserInputWireData {
            request_id,
            thread_id,
            questions,
        },
        pending,
    ))
}

fn approval_request_wire(
    identity: &AgentIdentity,
    device_id: &str,
    message: &ApprovalWireData,
) -> Result<Value> {
    let aad = crypto::payload_aad(&[
        ("kind", "approval_request".to_string()),
        ("device_id", device_id.to_string()),
        ("agent_id", identity.agent_id.clone()),
        ("approval_id", message.approval_id.clone()),
        ("thread_id", message.thread_id.clone()),
        ("approval_type", message.approval_type.clone()),
    ]);
    let ciphertext = encrypt_for_mobile(identity, device_id, message.ciphertext.as_bytes(), &aad)?;
    Ok(json!({
        "kind": "approval_request",
        "device_id": device_id,
        "approval_id": message.approval_id,
        "thread_id": message.thread_id,
        "approval_type": message.approval_type,
        "ciphertext": ciphertext,
    }))
}

fn user_input_request_wire(
    identity: &AgentIdentity,
    device_id: &str,
    message: &UserInputWireData,
) -> Result<Value> {
    let aad = crypto::payload_aad(&[
        ("kind", "user_input_request".to_string()),
        ("device_id", device_id.to_string()),
        ("agent_id", identity.agent_id.clone()),
        ("request_id", message.request_id.clone()),
        ("thread_id", message.thread_id.clone()),
        ("status", "pending".to_string()),
    ]);
    let plaintext = serde_json::to_vec(&json!({ "questions": message.questions }))?;
    let ciphertext = encrypt_for_mobile(identity, device_id, &plaintext, &aad)?;
    Ok(json!({
        "kind": "user_input_request",
        "device_id": device_id,
        "request_id": message.request_id,
        "thread_id": message.thread_id,
        "status": "pending",
        "ciphertext": ciphertext,
    }))
}

fn task_progress_push_wire(
    identity: &AgentIdentity,
    device_id: &str,
    thread_id: &str,
) -> Result<Value> {
    let plaintext = serde_json::to_vec(&json!({
        "thread_id": thread_id,
    }))?;
    let aad = crypto::payload_aad(&[
        ("kind", "task_progress_push".to_string()),
        ("device_id", device_id.to_string()),
        ("agent_id", identity.agent_id.clone()),
    ]);
    let ciphertext = encrypt_for_mobile(identity, device_id, &plaintext, &aad)?;
    Ok(json!({
        "kind": "task_progress_push",
        "device_id": device_id,
        "ciphertext": ciphertext,
    }))
}

fn approval_sync(approval_id: &str, thread_id: &str, approval_type: &str, status: &str) -> Value {
    json!({
        "kind": "approval_sync",
        "approval_id": approval_id,
        "thread_id": thread_id,
        "approval_type": approval_type,
        "status": status,
    })
}

fn approval_response_failed(device_id: &str, approval_id: &str, error: &str) -> Value {
    json!({
        "kind": "approval_response_failed",
        "device_id": device_id,
        "approval_id": approval_id,
        "error": error,
    })
}

fn user_input_response_failed(device_id: &str, request_id: &str, error: &str) -> Value {
    json!({
        "kind": "user_input_response_failed",
        "device_id": device_id,
        "request_id": request_id,
        "error": error,
    })
}

fn user_input_sync(request_id: &str, thread_id: &str, status: &str) -> Value {
    json!({
        "kind": "user_input_sync",
        "request_id": request_id,
        "thread_id": thread_id,
        "status": status,
    })
}

fn project_user_input_question(question: Value) -> Value {
    let options = question
        .get("options")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|option| {
            json!({
                "label": option.get("label").and_then(Value::as_str).unwrap_or_default(),
                "description": option.get("description").and_then(Value::as_str).unwrap_or_default(),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "question_id": question.get("id").and_then(Value::as_str).unwrap_or_default(),
        "header": question.get("header").and_then(Value::as_str).unwrap_or_default(),
        "prompt": question.get("question").and_then(Value::as_str).unwrap_or_default(),
        "options": options,
        "is_other": question.get("isOther").and_then(Value::as_bool).unwrap_or(false),
        "is_secret": question.get("isSecret").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn project_mcp_elicitation_question(
    field_id: &str,
    field_schema: &Value,
    request_message: &str,
    include_request_message: bool,
) -> Value {
    let description = field_schema.get("description").and_then(Value::as_str);
    let prompt = if include_request_message {
        join_prompt(request_message, description)
    } else {
        description.unwrap_or(request_message).to_string()
    };
    let options = mcp_elicitation_options(field_schema);
    let is_other = options.is_empty();
    json!({
        "question_id": field_id,
        "header": field_schema
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(field_id),
        "prompt": prompt,
        "options": options,
        "is_other": is_other,
        "is_secret": false,
    })
}

fn mcp_elicitation_confirmation_question(
    question_id: &str,
    server_name: Option<&str>,
    prompt: &str,
) -> Value {
    json!({
        "question_id": question_id,
        "header": server_name.unwrap_or("MCP request"),
        "prompt": prompt,
        "options": [
            {"label": "accept", "description": "Continue"},
            {"label": "decline", "description": "Decline"},
        ],
        "is_other": false,
        "is_secret": false,
    })
}

fn join_prompt(message: &str, detail: Option<&str>) -> String {
    let message = message.trim();
    let detail = detail.unwrap_or_default().trim();
    match (message.is_empty(), detail.is_empty()) {
        (true, true) => "Additional input is required.".to_string(),
        (false, true) => message.to_string(),
        (true, false) => detail.to_string(),
        (false, false) => format!("{message}\n\n{detail}"),
    }
}

fn mcp_field_value_kind(field_schema: &Value) -> Option<McpElicitationValueKind> {
    match field_schema.get("type").and_then(Value::as_str)? {
        "string" => Some(McpElicitationValueKind::String),
        "number" => Some(McpElicitationValueKind::Number),
        "integer" => Some(McpElicitationValueKind::Integer),
        "boolean" => Some(McpElicitationValueKind::Boolean),
        "array" => Some(McpElicitationValueKind::StringArray),
        _ => None,
    }
}

fn mcp_elicitation_options(field_schema: &Value) -> Vec<Value> {
    if let Some(options) = field_schema.get("oneOf").and_then(Value::as_array) {
        return mcp_const_options(options);
    }
    if let Some(options) = field_schema.get("enum").and_then(Value::as_array) {
        let enum_names = field_schema.get("enumNames").and_then(Value::as_array);
        return options
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                let label = value.as_str()?;
                let description = enum_names
                    .and_then(|names| names.get(index))
                    .and_then(Value::as_str)
                    .filter(|name| *name != label)
                    .unwrap_or_default();
                Some(json!({
                    "label": label,
                    "description": description,
                }))
            })
            .collect();
    }
    if field_schema.get("type").and_then(Value::as_str) == Some("boolean") {
        return vec![
            json!({"label": "true", "description": "Yes"}),
            json!({"label": "false", "description": "No"}),
        ];
    }
    if let Some(items) = field_schema.get("items") {
        if let Some(options) = items.get("anyOf").and_then(Value::as_array) {
            return mcp_const_options(options);
        }
        if let Some(options) = items.get("enum").and_then(Value::as_array) {
            return options
                .iter()
                .filter_map(Value::as_str)
                .map(|label| {
                    json!({
                        "label": label,
                        "description": "",
                    })
                })
                .collect();
        }
    }
    Vec::new()
}

fn mcp_const_options(options: &[Value]) -> Vec<Value> {
    options
        .iter()
        .filter_map(|option| {
            let label = option.get("const").and_then(Value::as_str)?;
            let description = option
                .get("title")
                .and_then(Value::as_str)
                .filter(|title| *title != label)
                .unwrap_or_default();
            Some(json!({
                "label": label,
                "description": description,
            }))
        })
        .collect()
}

/// Converts encrypted mobile user-input answers back to the response payload
/// expected by the originating app-server callback.
fn user_input_app_server_response(pending: &PendingUserInput, answers: &Value) -> Result<Value> {
    match &pending.response_format {
        UserInputResponseFormat::CodexRequestUserInput => Ok(json!({ "answers": answers })),
        UserInputResponseFormat::McpElicitation(format) => {
            mcp_elicitation_response_payload(format, answers)
        }
    }
}

fn mcp_elicitation_response_payload(
    format: &McpElicitationResponseFormat,
    answers: &Value,
) -> Result<Value> {
    match format {
        McpElicitationResponseFormat::Form { fields } => {
            let action = form_elicitation_action(fields, answers)?;
            let content = if action == "accept" {
                mcp_elicitation_form_content(fields, answers)?
            } else {
                Value::Null
            };
            Ok(json!({
                "action": action,
                "content": content,
                "_meta": null,
            }))
        }
        McpElicitationResponseFormat::Url => Ok(json!({
            "action": mcp_url_elicitation_action(answers)?,
            "content": null,
            "_meta": null,
        })),
    }
}

fn form_elicitation_action(
    fields: &[McpElicitationField],
    answers: &Value,
) -> Result<&'static str> {
    if fields.is_empty() {
        return mcp_confirmation_action(answers, "confirm");
    }
    Ok("accept")
}

fn mcp_url_elicitation_action(answers: &Value) -> Result<&'static str> {
    mcp_confirmation_action(answers, "action")
}

fn mcp_confirmation_action(answers: &Value, question_id: &str) -> Result<&'static str> {
    let values = user_input_answer_values(answers, question_id);
    if values.len() > 1 {
        anyhow::bail!("expected one answer for {question_id}");
    }
    match values.first().map(String::as_str) {
        Some("decline") => Ok("decline"),
        Some("accept") => Ok("accept"),
        Some(other) => anyhow::bail!("unsupported confirmation answer {other}"),
        None => anyhow::bail!("missing confirmation answer for {question_id}"),
    }
}

fn mcp_elicitation_form_content(fields: &[McpElicitationField], answers: &Value) -> Result<Value> {
    let mut content = Map::new();
    for field in fields {
        let values = user_input_answer_values(answers, &field.id);
        if values.is_empty() {
            if field.required {
                anyhow::bail!("missing required answer for {}", field.id);
            }
            continue;
        }
        content.insert(field.id.clone(), mcp_answer_value(field, &values)?);
    }
    Ok(Value::Object(content))
}

fn mcp_answer_value(field: &McpElicitationField, values: &[String]) -> Result<Value> {
    if field.value_kind != McpElicitationValueKind::StringArray && values.len() > 1 {
        anyhow::bail!("expected one answer for {}", field.id);
    }
    let first = values
        .first()
        .context("missing answer value")?
        .trim()
        .to_string();
    match field.value_kind {
        McpElicitationValueKind::String => Ok(Value::String(first)),
        McpElicitationValueKind::Number => {
            let parsed = first
                .parse::<f64>()
                .with_context(|| format!("invalid number answer for {}", field.id))?;
            let number = Number::from_f64(parsed)
                .with_context(|| format!("invalid finite number answer for {}", field.id))?;
            Ok(Value::Number(number))
        }
        McpElicitationValueKind::Integer => {
            let parsed = first
                .parse::<i64>()
                .with_context(|| format!("invalid integer answer for {}", field.id))?;
            Ok(Value::Number(Number::from(parsed)))
        }
        McpElicitationValueKind::Boolean => {
            let value = match first.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" => true,
                "false" | "no" | "0" => false,
                _ => anyhow::bail!("invalid boolean answer for {}", field.id),
            };
            Ok(Value::Bool(value))
        }
        McpElicitationValueKind::StringArray => Ok(Value::Array(
            values
                .iter()
                .map(|value| Value::String(value.clone()))
                .collect(),
        )),
    }
}

fn user_input_answer_values(answers: &Value, question_id: &str) -> Vec<String> {
    let Some(answer) = answers.get(question_id) else {
        return Vec::new();
    };
    if let Some(values) = answer.get("answers").and_then(Value::as_array) {
        return values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    if let Some(values) = answer.as_array() {
        return values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
    }
    answer
        .as_str()
        .map(|value| vec![value.to_string()])
        .unwrap_or_default()
}

fn approval_type_for_method(method: &str) -> Option<&'static str> {
    match method {
        "item/commandExecution/requestApproval" => Some("shell_command"),
        "item/fileChange/requestApproval" => Some("file_change"),
        "item/permissions/requestApproval" => Some("permissions"),
        _ => None,
    }
}

fn approval_decision(inbound: &ApprovalResponseInbound) -> &'static str {
    if inbound.decision != "allow" {
        "decline"
    } else if grant_scope_is_session(inbound) {
        "acceptForSession"
    } else {
        "accept"
    }
}

fn permission_grant_scope(inbound: &ApprovalResponseInbound) -> &'static str {
    if grant_scope_is_session(inbound) {
        "session"
    } else {
        "turn"
    }
}

fn grant_scope_is_session(inbound: &ApprovalResponseInbound) -> bool {
    inbound
        .grant_scope
        .as_ref()
        .and_then(|scope| scope.get("scope"))
        .and_then(Value::as_str)
        == Some("session")
}

fn request_id_string(request_id: &Value) -> String {
    request_id
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| request_id.to_string())
}

fn wire_kind(payload: &Value) -> &str {
    payload
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
}

fn wire_request_id(payload: &Value) -> String {
    payload
        .get("request_id")
        .or_else(|| payload.get("id"))
        .map(request_id_string)
        .unwrap_or_default()
}

fn wire_device_id(payload: &Value) -> String {
    payload
        .get("device_id")
        .or_else(|| payload.get("source_device_id"))
        .or_else(|| payload.get("target_device_id"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn wire_thread_id(payload: &Value) -> String {
    payload
        .get("thread_id")
        .or_else(|| payload.get("threadId"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn wire_cursor(payload: &Value) -> Option<i64> {
    payload.get("cursor").and_then(Value::as_i64)
}

fn wire_entry_count(payload: &Value) -> Option<u64> {
    payload.get("entry_count").and_then(Value::as_u64)
}

fn is_app_server_request(payload: &Value) -> bool {
    payload.get("method").is_some() && payload.get("id").is_some()
}

async fn resolved_request_syncs(
    notification: &Value,
    pending_approvals: Arc<RwLock<HashMap<String, PendingApproval>>>,
    pending_user_inputs: Arc<RwLock<HashMap<String, PendingUserInput>>>,
    completed_user_inputs: Arc<RwLock<HashMap<String, CompletedUserInput>>>,
) -> Vec<Value> {
    if notification.get("method").and_then(Value::as_str) != Some("serverRequest/resolved") {
        return Vec::new();
    }
    let Some(request_id) = notification
        .get("params")
        .and_then(|params| params.get("requestId"))
        .map(request_id_string)
    else {
        return Vec::new();
    };
    let mut syncs = Vec::new();
    let approval = {
        let approvals = pending_approvals.read().await;
        approvals
            .values()
            .find(|pending| request_id_string(&pending.request_id) == request_id)
            .cloned()
    };
    if let Some(approval) = approval {
        pending_approvals
            .write()
            .await
            .remove(&approval.approval_id);
        syncs.push(approval_sync(
            &approval.approval_id,
            &approval.thread_id,
            approval_type_for_method(&approval.method).unwrap_or("unknown"),
            "resolved",
        ));
    }
    let user_input = {
        let user_inputs = pending_user_inputs.read().await;
        user_inputs
            .values()
            .find(|pending| request_id_string(&pending.app_server_request_id) == request_id)
            .cloned()
    };
    if let Some(user_input) = user_input {
        pending_user_inputs
            .write()
            .await
            .remove(&user_input.request_id);
        {
            let mut completed = completed_user_inputs.write().await;
            mark_user_input_completed(&mut completed, &user_input, None);
        }
        syncs.push(user_input_sync(
            &user_input.request_id,
            &user_input.thread_id,
            "resolved",
        ));
    }
    syncs
}

fn thread_sync_failed(
    request_id: Option<&str>,
    device_id: &str,
    thread_id: &str,
    cursor: i64,
    checkpoint: Option<&str>,
    error: &str,
) -> serde_json::Value {
    let mut payload = json!({
        "kind": "thread_sync_failed",
        "device_id": device_id,
        "thread_id": thread_id,
        "cursor": cursor,
        "checkpoint": checkpoint,
        "error": error,
    });
    if let Some(request_id) = request_id {
        payload["request_id"] = json!(request_id);
    }
    payload
}

async fn send_wire_message<S, E>(writer: &mut S, payload: serde_json::Value) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    let text = serde_json::to_string(&payload)?;
    info!(
        kind = %wire_kind(&payload),
        request_id = %wire_request_id(&payload),
        device_id = %wire_device_id(&payload),
        thread_id = %wire_thread_id(&payload),
        cursor = ?wire_cursor(&payload),
        entry_count = ?wire_entry_count(&payload),
        payload_bytes = text.len(),
        "gateway_ws_out_start"
    );
    if let Err(error) = writer.send(Message::Text(text.into())).await {
        warn!(
            kind = %wire_kind(&payload),
            request_id = %wire_request_id(&payload),
            device_id = %wire_device_id(&payload),
            thread_id = %wire_thread_id(&payload),
            "gateway_ws_out_failed: {error:#}"
        );
        return Err(error.into());
    }
    info!(
        kind = %wire_kind(&payload),
        request_id = %wire_request_id(&payload),
        device_id = %wire_device_id(&payload),
        thread_id = %wire_thread_id(&payload),
        "gateway_ws_out_done"
    );
    Ok(())
}

async fn send_wire_messages<S, E>(
    writer: &mut S,
    identity: &AgentIdentity,
    messages: Vec<serde_json::Value>,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
    transfer_context: Option<&TransferContext>,
) -> Result<()>
where
    S: Sink<Message, Error = E> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
{
    for message in messages {
        let message = transfers::normalize_task_update_transfers(message, transfer_context).await;
        let message = encrypt_task_update(identity, message)?;
        track_wire_message(&message, active_threads.clone()).await;
        send_wire_message(writer, message).await?;
    }
    Ok(())
}

fn thread_sync_completed(
    request_id: Option<&str>,
    device_id: &str,
    thread_id: &str,
    fallback_cursor: i64,
    fallback_checkpoint: Option<&str>,
    messages: &[serde_json::Value],
) -> serde_json::Value {
    let mut payload = json!({
        "kind": "thread_sync_completed",
        "device_id": device_id,
        "thread_id": thread_id,
        "cursor": completed_cursor(messages, fallback_cursor),
        "checkpoint": completed_checkpoint(messages, fallback_checkpoint),
        "entry_count": messages.len(),
    });
    if let Some(request_id) = request_id {
        payload["request_id"] = json!(request_id);
    }
    payload
}

fn completed_cursor(messages: &[serde_json::Value], fallback_cursor: i64) -> i64 {
    messages
        .iter()
        .filter_map(|message| message.get("seq").and_then(Value::as_i64))
        .max()
        .unwrap_or(fallback_cursor)
}

fn completed_checkpoint(
    messages: &[serde_json::Value],
    fallback_checkpoint: Option<&str>,
) -> Option<String> {
    messages
        .iter()
        .rev()
        .filter_map(|message| message.get("checkpoint").and_then(Value::as_str))
        .next()
        .map(str::to_string)
        .or_else(|| fallback_checkpoint.map(str::to_string))
}

fn encrypt_task_update(identity: &AgentIdentity, mut payload: Value) -> Result<Value> {
    if payload.get("kind").and_then(Value::as_str) != Some("task_update") {
        return Ok(payload);
    }
    let device_id = string_field(&payload, "device_id").context("task_update missing device_id")?;
    let ciphertext =
        string_field(&payload, "ciphertext").context("task_update missing ciphertext")?;
    let aad = task_update_aad(identity, &payload, &device_id);
    payload["ciphertext"] = json!(encrypt_for_mobile(
        identity,
        &device_id,
        ciphertext.as_bytes(),
        &aad,
    )?);
    Ok(payload)
}

fn encrypt_for_mobile(
    identity: &AgentIdentity,
    device_id: &str,
    plaintext: &[u8],
    aad: &[u8],
) -> Result<String> {
    let binding = bindings::binding_for_device(device_id)?
        .with_context(|| format!("missing pair binding for device {device_id}"))?;
    crypto::encrypt_payload(
        &identity.encryption_private_key,
        &binding.ios_encryption_public_key,
        &binding.binding_id,
        crypto::PayloadDirection::AgentToIos,
        plaintext,
        aad,
    )
}

fn decrypt_from_mobile(
    identity: &AgentIdentity,
    device_id: &str,
    ciphertext: &str,
    aad: &[u8],
) -> Result<Vec<u8>> {
    let binding = bindings::binding_for_device(device_id)?
        .with_context(|| format!("missing pair binding for device {device_id}"))?;
    crypto::decrypt_payload(
        &identity.encryption_private_key,
        &binding.ios_encryption_public_key,
        &binding.binding_id,
        crypto::PayloadDirection::IosToAgent,
        ciphertext,
        aad,
    )
}

fn task_update_aad(identity: &AgentIdentity, payload: &Value, device_id: &str) -> Vec<u8> {
    crypto::payload_aad(&[
        ("kind", "task_update".to_string()),
        ("device_id", device_id.to_string()),
        ("agent_id", identity.agent_id.clone()),
        (
            "thread_id",
            string_field(payload, "thread_id").unwrap_or_default(),
        ),
        (
            "seq",
            payload
                .get("seq")
                .and_then(Value::as_i64)
                .map(|seq| seq.to_string())
                .unwrap_or_default(),
        ),
        ("role", string_field(payload, "role").unwrap_or_default()),
        ("type", string_field(payload, "type").unwrap_or_default()),
        (
            "project_id",
            string_field(payload, "project_id").unwrap_or_default(),
        ),
        (
            "entry_id",
            string_field(payload, "entry_id").unwrap_or_default(),
        ),
    ])
}

fn decrypt_approval_response(
    identity: &AgentIdentity,
    value: Value,
) -> Result<ApprovalResponseInbound> {
    let encrypted: EncryptedMobilePayload = serde_json::from_value(value)?;
    let aad = crypto::payload_aad(&[
        ("kind", "approval_response".to_string()),
        ("device_id", encrypted.device_id.clone()),
        ("agent_id", identity.agent_id.clone()),
    ]);
    let plaintext =
        decrypt_from_mobile(identity, &encrypted.device_id, &encrypted.ciphertext, &aad)?;
    let mut inbound: ApprovalResponseInbound = serde_json::from_slice(&plaintext)?;
    inbound.device_id = encrypted.device_id;
    Ok(inbound)
}

fn decrypt_user_input_response(
    identity: &AgentIdentity,
    value: Value,
) -> Result<UserInputResponseInbound> {
    let encrypted: EncryptedMobilePayload = serde_json::from_value(value)?;
    let aad = crypto::payload_aad(&[
        ("kind", "user_input_response".to_string()),
        ("device_id", encrypted.device_id.clone()),
        ("agent_id", identity.agent_id.clone()),
    ]);
    let plaintext =
        decrypt_from_mobile(identity, &encrypted.device_id, &encrypted.ciphertext, &aad)?;
    let mut inbound: UserInputResponseInbound = serde_json::from_slice(&plaintext)?;
    inbound.device_id = encrypted.device_id;
    Ok(inbound)
}

async fn track_wire_message(
    message: &serde_json::Value,
    active_threads: Arc<RwLock<HashMap<String, ActiveThread>>>,
) {
    if message.get("kind").and_then(|kind| kind.as_str()) != Some("task_update") {
        return;
    }
    let Some(thread_id) = message
        .get("thread_id")
        .and_then(|thread_id| thread_id.as_str())
        .map(str::to_string)
    else {
        return;
    };
    let Some(device_id) = message
        .get("device_id")
        .and_then(|device_id| device_id.as_str())
        .map(str::to_string)
    else {
        return;
    };
    let cursor = message.get("seq").and_then(|seq| seq.as_i64()).unwrap_or(0);
    let checkpoint = message
        .get("checkpoint")
        .and_then(|checkpoint| checkpoint.as_str())
        .map(str::to_string);
    let project_id = message
        .get("project_id")
        .and_then(|project_id| project_id.as_str())
        .map(str::to_string);
    let mut active = active_threads.write().await;
    active
        .entry(thread_id)
        .and_modify(|current| {
            current.device_id = device_id.clone();
            current.cursor = current.cursor.max(cursor);
            current.checkpoint = checkpoint.clone().or_else(|| current.checkpoint.clone());
            current.project_id = project_id.clone().or_else(|| current.project_id.clone());
        })
        .or_insert(ActiveThread {
            device_id,
            cursor,
            checkpoint,
            project_id,
            active_turn_id: None,
            last_pushed_completion: None,
        });
}

async fn next_notification(
    receiver: &mut Option<broadcast::Receiver<serde_json::Value>>,
) -> Option<serde_json::Value> {
    let receiver = receiver.as_mut()?;
    loop {
        match receiver.recv().await {
            Ok(notification) => return Some(notification),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                warn!("app-server notification receiver lagged by {skipped} messages");
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => return None,
        }
    }
}

async fn next_app_server_request(
    app_server: Option<&CodexAppServerClient>,
) -> Option<serde_json::Value> {
    app_server?.recv_request().await
}

fn notification_thread_id(notification: &serde_json::Value) -> Option<String> {
    notification
        .get("params")
        .and_then(|params| {
            params
                .get("threadId")
                .or_else(|| params.get("thread_id"))
                .and_then(|thread_id| thread_id.as_str())
                .or_else(|| {
                    params
                        .get("thread")
                        .and_then(|thread| thread.get("id"))
                        .and_then(|thread_id| thread_id.as_str())
                })
        })
        .or_else(|| {
            notification
                .get("threadId")
                .or_else(|| notification.get("thread_id"))
                .and_then(|thread_id| thread_id.as_str())
        })
        .map(str::to_string)
}

fn notification_method(notification: &serde_json::Value) -> Option<&str> {
    notification.get("method").and_then(Value::as_str)
}

fn is_task_progress_push_trigger(notification: &serde_json::Value) -> bool {
    matches!(notification_method(notification), Some("turn/completed"))
}

fn task_progress_push_marker(notification: &serde_json::Value) -> Option<String> {
    if !is_task_progress_push_trigger(notification) {
        return None;
    }
    notification_turn_id(notification).map(|turn_id| format!("turn:{turn_id}"))
}

fn notification_turn_id(notification: &serde_json::Value) -> Option<String> {
    notification
        .get("params")
        .and_then(|params| {
            params
                .get("turnId")
                .or_else(|| params.get("turn_id"))
                .and_then(Value::as_str)
                .or_else(|| {
                    params
                        .get("turn")
                        .and_then(|turn| turn.get("id"))
                        .and_then(Value::as_str)
                })
        })
        .or_else(|| {
            notification
                .get("turnId")
                .or_else(|| notification.get("turn_id"))
                .and_then(Value::as_str)
        })
        .map(str::to_string)
}

fn notification_thread_payload(notification: &serde_json::Value) -> Option<&Value> {
    notification
        .get("params")
        .and_then(|params| params.get("thread"))
}

fn notification_status(notification: &serde_json::Value) -> Option<&Value> {
    notification
        .get("params")
        .and_then(|params| params.get("status"))
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn number_field(payload: &Value, key: &str) -> Option<f64> {
    payload
        .get(key)
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
}

async fn handle_pair_handshake(
    identity: &AgentIdentity,
    pairing: Arc<RwLock<PairingRuntimeState>>,
    inbound: PairHandshakeMessage,
) -> PairHandshakeAck {
    match accept_pair_handshake(identity, pairing, &inbound).await {
        Ok(handshake_hash) => signed_ack(identity, inbound, handshake_hash, "accepted", None),
        Err(err) => PairHandshakeAck {
            kind: "pair_handshake_ack",
            request_id: inbound.request_id,
            device_id: inbound.device_id,
            agent_id: inbound.agent_id,
            pair_token: inbound.pair_token,
            binding_id: inbound.binding_id,
            handshake_hash: crypto::sha256_hex(inbound.encrypted_handshake),
            ack_status: "rejected".to_string(),
            signature: None,
            error: Some(err.to_string()),
        },
    }
}

async fn accept_pair_handshake(
    identity: &AgentIdentity,
    pairing: Arc<RwLock<PairingRuntimeState>>,
    inbound: &PairHandshakeMessage,
) -> Result<String> {
    if inbound.agent_id != identity.agent_id {
        anyhow::bail!("pair handshake targets a different agent")
    }
    let secret = pairing
        .read()
        .await
        .secrets
        .get(&inbound.pair_token)
        .cloned()
        .context("pair token is not active in this gateway")?;
    if inbound.agent_pairing_public_key != secret.pairing_public_key {
        anyhow::bail!("pairing public key mismatch")
    }
    let envelope: EncryptedHandshakeEnvelope =
        serde_json::from_str(&inbound.encrypted_handshake)
            .context("encrypted_handshake must be a JSON envelope")?;
    let plaintext = crypto::decrypt_pairing_handshake(
        &secret.pairing_private_key,
        &envelope.ios_encryption_public_key,
        &envelope.nonce,
        &envelope.ciphertext,
        &inbound.pair_token,
    )?;
    let handshake: PairHandshakePlaintext =
        serde_json::from_slice(&plaintext).context("invalid pair handshake plaintext")?;
    if handshake.device_id != inbound.device_id {
        anyhow::bail!("pair handshake device mismatch")
    }
    crypto::decode_prefixed(&handshake.ios_encryption_public_key, crypto::X25519_PREFIX)
        .context("invalid iOS encryption public key")?;
    bindings::save_binding(PairedDeviceBinding {
        binding_id: inbound.binding_id.clone(),
        device_id: inbound.device_id.clone(),
        agent_id: inbound.agent_id.clone(),
        ios_encryption_public_key: handshake.ios_encryption_public_key,
        paired_at: unix_timestamp(),
    })?;
    Ok(crypto::sha256_hex(&inbound.encrypted_handshake))
}

fn signed_ack(
    identity: &AgentIdentity,
    inbound: PairHandshakeMessage,
    handshake_hash: String,
    ack_status: &str,
    error: Option<String>,
) -> PairHandshakeAck {
    let digest = pair_ack_digest(
        &inbound.binding_id,
        &inbound.device_id,
        &inbound.agent_id,
        &inbound.pair_token,
        &handshake_hash,
        ack_status,
    );
    let signature = crypto::sign_ed25519(&identity.signing_private_key, &digest).ok();
    PairHandshakeAck {
        kind: "pair_handshake_ack",
        request_id: inbound.request_id,
        device_id: inbound.device_id,
        agent_id: inbound.agent_id,
        pair_token: inbound.pair_token,
        binding_id: inbound.binding_id,
        handshake_hash,
        ack_status: ack_status.to_string(),
        signature,
        error,
    }
}

fn pair_ack_digest(
    binding_id: &str,
    device_id: &str,
    agent_id: &str,
    pair_token: &str,
    handshake_hash: &str,
    ack_status: &str,
) -> String {
    crypto::sha256_hex(format!(
        "{binding_id}:{device_id}:{agent_id}:{pair_token}:{handshake_hash}:{ack_status}"
    ))
}

fn agent_ws_url(server_url: &str, agent_id: &str, session_token: &str) -> Result<Url> {
    let mut url = Url::parse(server_url).context("invalid niuma-server URL")?;
    match url.scheme() {
        "http" => url.set_scheme("ws").ok(),
        "https" => url.set_scheme("wss").ok(),
        _ => None,
    }
    .context("server URL must use http or https")?;
    let base_path = url.path().trim_end_matches('/');
    let websocket_path = if base_path.is_empty() {
        "/ws/agent".to_string()
    } else {
        format!("{base_path}/ws/agent")
    };
    url.set_path(&websocket_path);
    url.query_pairs_mut()
        .clear()
        .append_pair("agent_id", agent_id)
        .append_pair("session_token", session_token);
    Ok(url)
}

fn unix_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::Arc;

    use serde_json::json;
    use tokio::sync::RwLock;

    use super::{
        ActiveThread, ApprovalResponseInbound, McpElicitationField, McpElicitationResponseFormat,
        McpElicitationValueKind, PendingApproval, PendingUserInput, ThreadArchiveRequest,
        ThreadRenameRequest, UserInputClaim, UserInputResponseFormat, UserInputResponseInbound,
        agent_ws_url, apply_turn_lifecycle_notification, build_mcp_elicitation_request,
        claim_task_progress_push, claim_user_input_request, handle_approval_response,
        handle_user_input_response, mark_user_input_completed, task_progress_push_marker,
        thread_archive_failed_wire, thread_archive_result_wire, thread_rename_failed_wire,
        thread_rename_result_wire, thread_sync_completed, user_input_app_server_response,
    };

    #[test]
    fn agent_ws_url_preserves_reverse_proxy_prefix() {
        let url = agent_ws_url(
            "https://example.invalid/niuma-server/",
            "agent_1",
            "session_1",
        )
        .expect("websocket URL should be valid");

        assert_eq!(
            url.as_str(),
            "wss://example.invalid/niuma-server/ws/agent?agent_id=agent_1&session_token=session_1"
        );
    }

    #[test]
    fn agent_ws_url_accepts_reverse_proxy_prefix_without_slash() {
        let url = agent_ws_url(
            "https://example.invalid/niuma-server",
            "agent_1",
            "session_1",
        )
        .expect("websocket URL should be valid");

        assert_eq!(
            url.as_str(),
            "wss://example.invalid/niuma-server/ws/agent?agent_id=agent_1&session_token=session_1"
        );
    }

    #[test]
    fn agent_ws_url_still_supports_root_server() {
        let url = agent_ws_url("http://127.0.0.1:8000", "agent_1", "session_1").expect("valid URL");

        assert_eq!(
            url.as_str(),
            "ws://127.0.0.1:8000/ws/agent?agent_id=agent_1&session_token=session_1"
        );
    }

    #[test]
    fn notification_completion_follows_projected_updates() {
        let completion = thread_sync_completed(
            None,
            "ios-device",
            "thread-1",
            6,
            Some("turn:old"),
            &[
                json!({
                    "kind": "task_update",
                    "seq": 7,
                    "checkpoint": "turn:new-user"
                }),
                json!({
                    "kind": "task_update",
                    "seq": 8,
                    "checkpoint": "turn:new-assistant"
                }),
            ],
        );

        assert_eq!(
            completion.get("kind").and_then(|value| value.as_str()),
            Some("thread_sync_completed")
        );
        assert_eq!(
            completion.get("cursor").and_then(|value| value.as_i64()),
            Some(8)
        );
        assert_eq!(
            completion
                .get("checkpoint")
                .and_then(|value| value.as_str()),
            Some("turn:new-assistant")
        );
        assert_eq!(
            completion
                .get("entry_count")
                .and_then(|value| value.as_u64()),
            Some(2)
        );
    }

    #[test]
    fn task_progress_push_marker_only_uses_terminal_turns() {
        let sync_completion = task_progress_push_marker(&json!({
            "kind": "thread_sync_completed",
            "thread_id": "thread-1",
            "checkpoint": "turn:turn-1"
        }));
        assert_eq!(sync_completion, None);

        let progress = task_progress_push_marker(&json!({
            "method": "item/completed",
            "params": {
                "threadId": "thread-1",
                "turnId": "turn-1"
            }
        }));
        assert_eq!(progress, None);

        let completed = task_progress_push_marker(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1",
                "turn": {
                    "id": "turn-1",
                    "items": [],
                    "status": "completed"
                }
            }
        }));
        assert_eq!(completed.as_deref(), Some("turn:turn-1"));
    }

    #[test]
    fn task_progress_push_marker_requires_app_server_turn_id() {
        let marker = task_progress_push_marker(&json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thread-1"
            }
        }));

        assert_eq!(marker, None);
    }

    #[test]
    fn thread_archive_acknowledgements_preserve_mobile_request_ids() {
        let request = ThreadArchiveRequest {
            request_id: "archive-1".to_string(),
            device_id: "ios-device".to_string(),
            thread_id: "thread-1".to_string(),
        };

        let success = thread_archive_result_wire(&request);
        assert_eq!(success["kind"], "thread_archive_result");
        assert_eq!(success["request_id"], "archive-1");
        assert_eq!(success["device_id"], "ios-device");
        assert_eq!(success["thread_id"], "thread-1");

        let failed = thread_archive_failed_wire(&request, "boom");
        assert_eq!(failed["kind"], "thread_archive_failed");
        assert_eq!(failed["request_id"], "archive-1");
        assert_eq!(failed["device_id"], "ios-device");
        assert_eq!(failed["thread_id"], "thread-1");
        assert_eq!(failed["error"], "boom");
    }

    #[test]
    fn thread_rename_acknowledgements_preserve_mobile_request_ids() {
        let request = ThreadRenameRequest {
            request_id: "rename-1".to_string(),
            device_id: "ios-device".to_string(),
            thread_id: "thread-1".to_string(),
            title: "New title".to_string(),
        };

        let success = thread_rename_result_wire(&request);
        assert_eq!(success["kind"], "thread_rename_result");
        assert_eq!(success["request_id"], "rename-1");
        assert_eq!(success["device_id"], "ios-device");
        assert_eq!(success["thread_id"], "thread-1");

        let failed = thread_rename_failed_wire(&request, "boom");
        assert_eq!(failed["kind"], "thread_rename_failed");
        assert_eq!(failed["request_id"], "rename-1");
        assert_eq!(failed["device_id"], "ios-device");
        assert_eq!(failed["thread_id"], "thread-1");
        assert_eq!(failed["error"], "boom");
    }

    #[tokio::test]
    async fn task_progress_push_claim_dedupes_same_completion() {
        let active_threads = Arc::new(RwLock::new(HashMap::from([(
            "thread-1".to_string(),
            ActiveThread {
                device_id: "ios-device".to_string(),
                cursor: 0,
                checkpoint: None,
                project_id: None,
                active_turn_id: None,
                last_pushed_completion: None,
            },
        )])));

        assert!(claim_task_progress_push(active_threads.clone(), "thread-1", "turn:turn-1").await);
        assert!(!claim_task_progress_push(active_threads.clone(), "thread-1", "turn:turn-1").await);
        assert!(claim_task_progress_push(active_threads, "thread-1", "turn:turn-2").await);
    }

    #[tokio::test]
    async fn turn_lifecycle_notifications_update_active_turn_marker() {
        let active_threads = Arc::new(RwLock::new(HashMap::from([(
            "thread-1".to_string(),
            ActiveThread {
                device_id: "ios-device".to_string(),
                cursor: 0,
                checkpoint: None,
                project_id: None,
                active_turn_id: None,
                last_pushed_completion: None,
            },
        )])));

        let completed = apply_turn_lifecycle_notification(
            active_threads.clone(),
            &json!({
                "method": "turn/started",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
        )
        .await;
        assert_eq!(completed, None);
        assert_eq!(
            active_threads
                .read()
                .await
                .get("thread-1")
                .and_then(|thread| thread.active_turn_id.as_deref()),
            Some("turn-1")
        );

        let completed = apply_turn_lifecycle_notification(
            active_threads.clone(),
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thread-1",
                    "turnId": "turn-1"
                }
            }),
        )
        .await;
        assert_eq!(completed.as_deref(), Some("thread-1"));
        assert_eq!(
            active_threads
                .read()
                .await
                .get("thread-1")
                .and_then(|thread| thread.active_turn_id.as_deref()),
            None
        );
    }

    #[tokio::test]
    async fn mcp_form_elicitation_projects_to_mobile_user_input() {
        let (message, pending) = build_mcp_elicitation_request(
            &json!("rpc-1"),
            &json!({
                "mode": "form",
                "threadId": "thread-1",
                "serverName": "github",
                "message": "Choose repository settings.",
                "requestedSchema": {
                    "type": "object",
                    "required": ["visibility", "notify"],
                    "properties": {
                        "visibility": {
                            "type": "string",
                            "title": "Visibility",
                            "description": "Who can see the repository?",
                            "oneOf": [
                                {"const": "private", "title": "Private"},
                                {"const": "public", "title": "Public"}
                            ]
                        },
                        "notify": {
                            "type": "boolean",
                            "title": "Notify"
                        }
                    }
                }
            }),
        )
        .await
        .expect("mcp elicitation should project");

        assert_eq!(message.request_id, "rpc-1");
        assert_eq!(message.thread_id, "thread-1");
        assert_eq!(message.questions.len(), 2);
        assert_eq!(message.questions[0]["question_id"], "notify");
        assert_eq!(message.questions[0]["options"][0]["label"], "true");
        assert_eq!(message.questions[1]["question_id"], "visibility");
        assert_eq!(message.questions[1]["options"][0]["label"], "private");
        assert!(
            message.questions[1]["prompt"]
                .as_str()
                .expect("prompt should be a string")
                .contains("Who can see the repository?")
        );
        match pending.response_format {
            UserInputResponseFormat::McpElicitation(McpElicitationResponseFormat::Form {
                fields,
            }) => {
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].id, "notify");
                assert!(fields[0].required);
                assert_eq!(fields[0].value_kind, McpElicitationValueKind::Boolean);
                assert_eq!(fields[1].id, "visibility");
                assert!(fields[1].required);
                assert_eq!(fields[1].value_kind, McpElicitationValueKind::String);
            }
            _ => panic!("unexpected response format"),
        }
    }

    #[tokio::test]
    async fn user_input_claim_dedupes_pending_request() {
        let pending_user_inputs = Arc::new(RwLock::new(HashMap::new()));
        let completed_user_inputs = Arc::new(RwLock::new(HashMap::new()));
        let pending = PendingUserInput {
            request_id: "input-1".to_string(),
            app_server_request_id: json!("rpc-1"),
            thread_id: "thread-1".to_string(),
            response_format: UserInputResponseFormat::CodexRequestUserInput,
        };

        assert!(matches!(
            claim_user_input_request(
                pending_user_inputs.clone(),
                completed_user_inputs.clone(),
                pending.clone(),
            )
            .await,
            UserInputClaim::New
        ));
        assert!(matches!(
            claim_user_input_request(pending_user_inputs, completed_user_inputs, pending).await,
            UserInputClaim::AlreadyPending
        ));
    }

    #[tokio::test]
    async fn user_input_claim_reuses_completed_response() {
        let pending_user_inputs = Arc::new(RwLock::new(HashMap::new()));
        let completed_user_inputs = Arc::new(RwLock::new(HashMap::new()));
        let pending = PendingUserInput {
            request_id: "input-1".to_string(),
            app_server_request_id: json!("rpc-1"),
            thread_id: "thread-1".to_string(),
            response_format: UserInputResponseFormat::CodexRequestUserInput,
        };

        {
            let mut completed = completed_user_inputs.write().await;
            mark_user_input_completed(&mut completed, &pending, Some(json!({"ok": true})));
        }
        let duplicate = PendingUserInput {
            app_server_request_id: json!("rpc-2"),
            ..pending
        };

        match claim_user_input_request(pending_user_inputs, completed_user_inputs, duplicate).await
        {
            UserInputClaim::AlreadyCompleted(Some(response)) => {
                assert_eq!(response["ok"], true);
            }
            claim => panic!("unexpected claim: {claim:?}"),
        }
    }

    #[test]
    fn mcp_form_elicitation_response_builds_structured_content() {
        let pending = PendingUserInput {
            request_id: "input-1".to_string(),
            app_server_request_id: json!("rpc-1"),
            thread_id: "thread-1".to_string(),
            response_format: UserInputResponseFormat::McpElicitation(
                McpElicitationResponseFormat::Form {
                    fields: vec![
                        McpElicitationField {
                            id: "name".to_string(),
                            value_kind: McpElicitationValueKind::String,
                            required: true,
                        },
                        McpElicitationField {
                            id: "count".to_string(),
                            value_kind: McpElicitationValueKind::Integer,
                            required: true,
                        },
                        McpElicitationField {
                            id: "enabled".to_string(),
                            value_kind: McpElicitationValueKind::Boolean,
                            required: true,
                        },
                        McpElicitationField {
                            id: "labels".to_string(),
                            value_kind: McpElicitationValueKind::StringArray,
                            required: false,
                        },
                    ],
                },
            ),
        };

        let response = user_input_app_server_response(
            &pending,
            &json!({
                "name": {"answers": ["release"]},
                "count": {"answers": ["3"]},
                "enabled": {"answers": ["true"]},
                "labels": {"answers": ["ios", "cli"]}
            }),
        )
        .expect("response should be convertible");

        assert_eq!(response["action"], "accept");
        assert_eq!(response["content"]["name"], "release");
        assert_eq!(response["content"]["count"], 3);
        assert_eq!(response["content"]["enabled"], true);
        assert_eq!(response["content"]["labels"], json!(["ios", "cli"]));
        assert!(response["_meta"].is_null());
    }

    #[test]
    fn mcp_form_elicitation_response_rejects_multiple_scalar_answers() {
        let pending = PendingUserInput {
            request_id: "input-1".to_string(),
            app_server_request_id: json!("rpc-1"),
            thread_id: "thread-1".to_string(),
            response_format: UserInputResponseFormat::McpElicitation(
                McpElicitationResponseFormat::Form {
                    fields: vec![McpElicitationField {
                        id: "visibility".to_string(),
                        value_kind: McpElicitationValueKind::String,
                        required: true,
                    }],
                },
            ),
        };

        let error = user_input_app_server_response(
            &pending,
            &json!({
                "visibility": {"answers": ["private", "public"]}
            }),
        )
        .expect_err("scalar elicitation fields must not accept multiple values");

        assert!(error.to_string().contains("expected one answer"));
    }

    #[test]
    fn mcp_url_elicitation_response_maps_decline_action() {
        let pending = PendingUserInput {
            request_id: "input-1".to_string(),
            app_server_request_id: json!("rpc-1"),
            thread_id: "thread-1".to_string(),
            response_format: UserInputResponseFormat::McpElicitation(
                McpElicitationResponseFormat::Url,
            ),
        };

        let response = user_input_app_server_response(
            &pending,
            &json!({
                "action": {"answers": ["decline"]}
            }),
        )
        .expect("url elicitation should be convertible");

        assert_eq!(response["action"], "decline");
        assert!(response["content"].is_null());
        assert!(response["_meta"].is_null());
    }

    #[tokio::test]
    async fn approval_response_without_app_server_reports_failure() {
        let response = handle_approval_response(
            None,
            Arc::new(RwLock::new(HashMap::new())),
            ApprovalResponseInbound {
                device_id: "ios-device".to_string(),
                approval_id: "approval-1".to_string(),
                decision: "allow".to_string(),
                grant_scope: None,
            },
        )
        .await
        .expect("failure event is returned");

        assert_eq!(response["kind"], "approval_response_failed");
        assert_eq!(response["device_id"], "ios-device");
        assert_eq!(response["approval_id"], "approval-1");
        assert_eq!(response["error"], "desktop app-server is not connected");
    }

    #[tokio::test]
    async fn approval_response_without_app_server_keeps_pending_request() {
        let pending_approvals = Arc::new(RwLock::new(HashMap::from([(
            "approval-1".to_string(),
            PendingApproval {
                approval_id: "approval-1".to_string(),
                request_id: json!("request-1"),
                method: "item/permissions/requestApproval".to_string(),
                thread_id: "thread-1".to_string(),
                params: json!({}),
            },
        )])));
        let response = handle_approval_response(
            None,
            pending_approvals.clone(),
            ApprovalResponseInbound {
                device_id: "ios-device".to_string(),
                approval_id: "approval-1".to_string(),
                decision: "allow".to_string(),
                grant_scope: None,
            },
        )
        .await
        .expect("failure event is returned");

        assert_eq!(response["kind"], "approval_response_failed");
        assert_eq!(response["approval_id"], "approval-1");
        assert!(pending_approvals.read().await.contains_key("approval-1"));
    }

    #[tokio::test]
    async fn user_input_response_without_app_server_reports_failure() {
        let response = handle_user_input_response(
            None,
            Arc::new(RwLock::new(HashMap::new())),
            Arc::new(RwLock::new(HashMap::new())),
            UserInputResponseInbound {
                device_id: "ios-device".to_string(),
                request_id: "input-1".to_string(),
                answers: json!({}),
            },
        )
        .await
        .expect("failure event is returned")
        .expect("failure event is present");

        assert_eq!(response["kind"], "user_input_response_failed");
        assert_eq!(response["device_id"], "ios-device");
        assert_eq!(response["request_id"], "input-1");
        assert_eq!(response["error"], "desktop app-server is not connected");
    }
}
