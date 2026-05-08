//! HTTP and WebSocket routes that preserve the existing Niuma wire protocol.

use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Query, State, WebSocketUpgrade,
        ws::{CloseFrame, Message, WebSocket},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    AppState,
    apns::PushSendOutcome,
    crypto, db,
    db::TransferBinding,
    error::ApiError,
    models::{
        ChallengeRequest, ChallengeResponse, DeviceRegisterRequest, DeviceRegisterResponse,
        HealthResponse, PairConfirmRequest, PairConfirmResponse, PairRequest, PairRequestResponse,
        PairRevokeRequest, PairRevokeResponse, PushTokenUpdateRequest, PushTokenUpdateResponse,
        TransferAckRequest, TransferAckResponse, TransferEnsureRequest, TransferEnsureResponse,
        TransferUploadResponse, VerifyRequest, VerifyResponse,
    },
    transfer::TransferManifest,
};

/// Build the application router without binding state, so `main` can inject it once.
pub fn router(max_transfer_bytes: usize) -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/devices/register", post(register_device))
        .route("/auth/challenge", post(auth_challenge))
        .route("/auth/verify", post(auth_verify))
        .route("/pair/request", post(pair_request))
        .route("/pair/confirm", post(pair_confirm))
        .route("/pair/revoke", post(pair_revoke))
        .route("/devices/push-token", post(update_push_token))
        .route("/transfers/{transfer_id}/ensure", post(ensure_transfer))
        .route(
            "/transfers/{transfer_id}",
            put(upload_transfer)
                .get(download_transfer)
                .layer(DefaultBodyLimit::max(max_transfer_bytes)),
        )
        .route("/transfers/{transfer_id}/ack", post(ack_transfer))
        .route("/ws/mobile", get(mobile_ws))
        .route("/ws/agent", get(agent_ws))
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn register_device(
    State(state): State<AppState>,
    Json(payload): Json<DeviceRegisterRequest>,
) -> Result<Json<DeviceRegisterResponse>, ApiError> {
    db::register_device(&state.pool, &payload).await?;
    Ok(Json(DeviceRegisterResponse {
        registered: true,
        server_time: chrono::Utc::now().timestamp(),
    }))
}

async fn auth_challenge(
    State(state): State<AppState>,
    Json(payload): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    let (challenge_id, challenge, expires_at) =
        db::issue_challenge(&state.pool, &state.settings, &payload.device_id).await?;
    Ok(Json(ChallengeResponse {
        challenge_id,
        challenge,
        expires_at,
    }))
}

async fn auth_verify(
    State(state): State<AppState>,
    Json(payload): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, ApiError> {
    let token = db::verify_challenge(&state.pool, &state.settings, &payload).await?;
    Ok(Json(VerifyResponse {
        verified: true,
        session_token: Some(token),
    }))
}

async fn pair_request(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PairRequest>,
) -> Result<Json<PairRequestResponse>, ApiError> {
    db::require_session(
        &state.pool,
        session_token(&headers),
        payload.agent_id.as_str(),
    )
    .await?;
    let (pair_token, expires_at) = db::issue_pair_token(
        &state.pool,
        &state.settings,
        &payload.agent_id,
        &payload.agent_pairing_public_key,
    )
    .await?;
    Ok(Json(PairRequestResponse {
        pair_token,
        expires_at,
    }))
}

async fn pair_confirm(
    State(state): State<AppState>,
    Json(payload): Json<PairConfirmRequest>,
) -> Result<Json<PairConfirmResponse>, ApiError> {
    let binding_id = db::validate_pair_confirm(&state.pool, &state.settings, &payload).await?;
    let ack = request_agent_pair_ack(&state, &payload, &binding_id).await?;
    validate_agent_pair_ack(&state, &payload, &binding_id, &ack).await?;
    db::commit_pair_confirm(&state.pool, &payload, &binding_id).await?;
    Ok(Json(PairConfirmResponse {
        binding_id,
        status: "active".to_string(),
        agent_ack: ack,
    }))
}

async fn request_agent_pair_ack(
    state: &AppState,
    payload: &PairConfirmRequest,
    binding_id: &str,
) -> Result<Value, ApiError> {
    let handshake = json!({
        "device_id": payload.device_id,
        "agent_id": payload.agent_id,
        "pair_token": payload.pair_token,
        "binding_id": binding_id,
        "agent_pairing_public_key": payload.agent_pairing_public_key,
        "encrypted_handshake": payload.encrypted_handshake,
    });
    state
        .hub
        .request_agent_pair_handshake(&payload.agent_id, handshake)
        .await
        .map_err(|error| ApiError::Conflict(error.to_string()))
}

async fn validate_agent_pair_ack(
    state: &AppState,
    payload: &PairConfirmRequest,
    binding_id: &str,
    ack: &Value,
) -> Result<(), ApiError> {
    let ack_status = value_str(ack, "ack_status")?;
    if ack_status != "accepted" {
        db::record_pair_failure(&state.pool, &payload.pair_token, &state.settings).await?;
        return Err(ApiError::BadRequest(
            ack.get("error")
                .and_then(Value::as_str)
                .unwrap_or("desktop gateway rejected pair handshake")
                .to_string(),
        ));
    }
    if value_str(ack, "device_id")? != payload.device_id
        || value_str(ack, "agent_id")? != payload.agent_id
        || value_str(ack, "pair_token")? != payload.pair_token
        || value_str(ack, "binding_id")? != binding_id
    {
        db::record_pair_failure(&state.pool, &payload.pair_token, &state.settings).await?;
        return Err(ApiError::BadRequest(
            "pair handshake ack mismatch".to_string(),
        ));
    }
    let handshake_hash = value_str(ack, "handshake_hash")?;
    if handshake_hash != crypto::sha256_hex(&payload.encrypted_handshake) {
        db::record_pair_failure(&state.pool, &payload.pair_token, &state.settings).await?;
        return Err(ApiError::BadRequest(
            "pair handshake hash mismatch".to_string(),
        ));
    }
    let signature = value_str(ack, "signature")?;
    let digest = crypto::pair_ack_digest(
        binding_id,
        &payload.device_id,
        &payload.agent_id,
        &payload.pair_token,
        handshake_hash,
        ack_status,
    );
    let public_key = db::public_key(&state.pool, &payload.agent_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("unknown agent".to_string()))?;
    if !crypto::verify_ed25519(&public_key, &digest, signature) {
        db::record_pair_failure(&state.pool, &payload.pair_token, &state.settings).await?;
        return Err(ApiError::BadRequest(
            "pair handshake ack signature invalid".to_string(),
        ));
    }
    Ok(())
}

async fn pair_revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PairRevokeRequest>,
) -> Result<Json<PairRevokeResponse>, ApiError> {
    db::require_session(
        &state.pool,
        session_token(&headers),
        payload.device_id.as_str(),
    )
    .await?;
    let revoked = db::revoke_pairing(&state.pool, &payload.device_id, &payload.agent_id).await?;
    Ok(Json(PairRevokeResponse { revoked }))
}

async fn update_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PushTokenUpdateRequest>,
) -> Result<Json<PushTokenUpdateResponse>, ApiError> {
    db::require_session(&state.pool, session_token(&headers), &payload.device_id).await?;
    let updated =
        db::update_ios_push_token(&state.pool, &payload.device_id, &payload.push_token).await?;
    if !updated {
        return Err(ApiError::NotFound("unknown iOS device".to_string()));
    }
    Ok(Json(PushTokenUpdateResponse { updated }))
}

async fn ensure_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<String>,
    Json(payload): Json<TransferEnsureRequest>,
) -> Result<Json<TransferEnsureResponse>, ApiError> {
    db::require_session(
        &state.pool,
        session_token(&headers),
        &payload.source_device_id,
    )
    .await?;
    db::require_transfer_binding(
        &state.pool,
        &TransferBinding {
            source_device_id: payload.source_device_id.clone(),
            target_device_id: payload.target_device_id.clone(),
            direction: payload.direction.clone(),
        },
    )
    .await?;
    let result = state
        .transfers
        .ensure_transfer(
            &transfer_id,
            &payload.source_device_id,
            &payload.target_device_id,
            &payload.direction,
            payload.encrypted_size_bytes,
        )
        .await
        .map_err(transfer_error)?;
    if !result.needs_upload {
        notify_transfer_ready(&state, &result.manifest).await;
    }
    Ok(Json(TransferEnsureResponse {
        transfer_id: result.manifest.transfer_id,
        expires_at: result.manifest.expires_at,
        needs_upload: result.needs_upload,
    }))
}

async fn upload_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<String>,
    body: Bytes,
) -> Result<Json<TransferUploadResponse>, ApiError> {
    let manifest = state
        .transfers
        .read_manifest(&transfer_id)
        .await
        .map_err(transfer_error)?;
    let device_id = required_header(&headers, "X-Device-ID")?;
    require_transfer_participant(&state, &manifest, device_id, session_token(&headers)).await?;
    if device_id != manifest.source_device_id {
        return Err(ApiError::Forbidden(
            "only source may upload transfer".to_string(),
        ));
    }
    let completed = state
        .transfers
        .write_payload(&transfer_id, &body)
        .await
        .map_err(transfer_error)?;
    notify_transfer_ready(&state, &completed).await;
    Ok(Json(TransferUploadResponse {
        uploaded: true,
        expires_at: completed.expires_at,
    }))
}

async fn download_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<String>,
    Query(query): Query<TransferDownloadQuery>,
) -> Result<Response, ApiError> {
    let manifest = state
        .transfers
        .read_manifest(&transfer_id)
        .await
        .map_err(transfer_error)?;
    require_transfer_participant(&state, &manifest, &query.device_id, session_token(&headers))
        .await?;
    if query.device_id != manifest.target_device_id {
        return Err(ApiError::Forbidden(
            "only target may download transfer".to_string(),
        ));
    }
    let body = state
        .transfers
        .read_payload(&transfer_id)
        .await
        .map_err(transfer_error)?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        body,
    )
        .into_response())
}

async fn ack_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<String>,
    Json(payload): Json<TransferAckRequest>,
) -> Result<Json<TransferAckResponse>, ApiError> {
    let manifest = state
        .transfers
        .read_manifest(&transfer_id)
        .await
        .map_err(transfer_error)?;
    require_transfer_participant(
        &state,
        &manifest,
        &payload.receiver_device_id,
        session_token(&headers),
    )
    .await?;
    if payload.receiver_device_id != manifest.target_device_id {
        return Err(ApiError::Forbidden(
            "only target may ack transfer".to_string(),
        ));
    }
    state
        .transfers
        .ack_transfer(&transfer_id)
        .await
        .map_err(transfer_error)?;
    Ok(Json(TransferAckResponse { acknowledged: true }))
}

async fn require_transfer_participant(
    state: &AppState,
    manifest: &TransferManifest,
    device_id: &str,
    token: Option<&str>,
) -> Result<(), ApiError> {
    db::require_session(&state.pool, token, device_id).await?;
    if device_id != manifest.source_device_id && device_id != manifest.target_device_id {
        return Err(ApiError::Forbidden(
            "device is not a transfer participant".to_string(),
        ));
    }
    db::require_transfer_binding(
        &state.pool,
        &TransferBinding {
            source_device_id: manifest.source_device_id.clone(),
            target_device_id: manifest.target_device_id.clone(),
            direction: manifest.direction.clone(),
        },
    )
    .await
}

async fn notify_transfer_ready(state: &AppState, manifest: &TransferManifest) {
    let payload = json!({
        "kind": "transfer_ready",
        "transfer_id": manifest.transfer_id,
        "direction": manifest.direction,
        "source_device_id": manifest.source_device_id,
        "target_device_id": manifest.target_device_id,
        "encrypted_size_bytes": manifest.encrypted_size_bytes,
        "expires_at": manifest.expires_at,
        "source": "server",
    });
    if manifest.direction == "ios_to_agent" {
        state
            .hub
            .send_to_agent(&manifest.target_device_id, payload)
            .await;
    } else {
        state
            .hub
            .send_to_mobile(&manifest.target_device_id, payload, None)
            .await;
    }
}

async fn mobile_ws(
    State(state): State<AppState>,
    Query(query): Query<MobileWsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| mobile_socket(state, query, socket))
}

async fn agent_ws(
    State(state): State<AppState>,
    Query(query): Query<AgentWsQuery>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| agent_socket(state, query, socket))
}

async fn mobile_socket(state: AppState, query: MobileWsQuery, socket: WebSocket) {
    if !db::validate_session_token(&state.pool, &query.session_token, &query.device_id)
        .await
        .unwrap_or(false)
    {
        close_socket(socket, 4401, "invalid session token").await;
        return;
    }
    if !db::is_paired(&state.pool, &query.device_id, &query.agent_id)
        .await
        .unwrap_or(false)
    {
        close_socket(socket, 4403, "device not paired").await;
        return;
    }
    let _ = db::touch_device(&state.pool, &query.device_id).await;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let connection_id = state
        .hub
        .connect_mobile(&query.device_id, &query.agent_id, tx.clone())
        .await;
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_sender.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(next) = ws_receiver.next().await {
        let Ok(message) = next else {
            break;
        };
        match message {
            Message::Text(text) => {
                if let Err(error) = handle_mobile_text(&state, &query, &tx, &text).await {
                    send_error_and_close(&tx, error);
                    break;
                }
            }
            Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => {
                    if let Err(error) = handle_mobile_text(&state, &query, &tx, &text).await {
                        send_error_and_close(&tx, error);
                        break;
                    }
                }
                Err(_) => {
                    send_error_and_close(&tx, "invalid utf-8 websocket payload".to_string());
                    break;
                }
            },
            Message::Ping(bytes) => {
                let _ = tx.send(Message::Pong(bytes));
            }
            Message::Pong(_) => {}
            Message::Close(_) => break,
        }
    }
    state
        .hub
        .disconnect_mobile(&query.device_id, &connection_id)
        .await;
    writer.abort();
}

async fn agent_socket(state: AppState, query: AgentWsQuery, socket: WebSocket) {
    if !db::validate_session_token(&state.pool, &query.session_token, &query.agent_id)
        .await
        .unwrap_or(false)
    {
        close_socket(socket, 4401, "invalid session token").await;
        return;
    }
    let _ = db::touch_device(&state.pool, &query.agent_id).await;

    let (mut ws_sender, mut ws_receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
    let connection_id = state.hub.connect_agent(&query.agent_id, tx.clone()).await;
    let writer = tokio::spawn(async move {
        while let Some(message) = rx.recv().await {
            if ws_sender.send(message).await.is_err() {
                break;
            }
        }
    });

    while let Some(next) = ws_receiver.next().await {
        let Ok(message) = next else {
            break;
        };
        match message {
            Message::Text(text) => {
                if let Err(error) = handle_agent_text(&state, &query.agent_id, &tx, &text).await {
                    send_error_and_close(&tx, error);
                    break;
                }
            }
            Message::Binary(bytes) => match String::from_utf8(bytes.to_vec()) {
                Ok(text) => {
                    if let Err(error) = handle_agent_text(&state, &query.agent_id, &tx, &text).await
                    {
                        send_error_and_close(&tx, error);
                        break;
                    }
                }
                Err(_) => {
                    send_error_and_close(&tx, "invalid utf-8 websocket payload".to_string());
                    break;
                }
            },
            Message::Ping(bytes) => {
                let _ = tx.send(Message::Pong(bytes));
            }
            Message::Pong(_) => {}
            Message::Close(_) => break,
        }
    }
    state
        .hub
        .disconnect_agent(&query.agent_id, &connection_id)
        .await;
    writer.abort();
}

async fn handle_mobile_text(
    state: &AppState,
    query: &MobileWsQuery,
    tx: &mpsc::UnboundedSender<Message>,
    text: &str,
) -> Result<(), String> {
    let mut payload = parse_json(text)?;
    let kind = payload_kind(&payload)?.to_string();
    match kind.as_str() {
        "task_start" => {
            validate_task_start(state, &payload, &query.device_id, &query.agent_id).await?;
            tracing::info!(
                device_id = %query.device_id,
                agent_id = %query.agent_id,
                "ws_mobile_task_start_validated"
            );
            payload["source"] = json!("mobile");
            route_to_agent_or_error(state, tx, &query.agent_id, payload, None).await
        }
        "resume_thread" => {
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_to_agent_or_error(state, tx, &query.agent_id, payload, None).await
        }
        "metadata_refresh" => {
            let request_id = payload
                .get("request_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_to_agent_or_error(state, tx, &query.agent_id, payload, request_id).await
        }
        "approval_response" | "user_input_response" => {
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_to_agent_or_error(state, tx, &query.agent_id, payload, None).await
        }
        _ => Err(format!("unsupported mobile message kind={kind}")),
    }
}

async fn route_to_agent_or_error(
    state: &AppState,
    tx: &mpsc::UnboundedSender<Message>,
    agent_id: &str,
    payload: Value,
    metadata_request_id: Option<String>,
) -> Result<(), String> {
    let delivered = state.hub.send_to_agent(agent_id, payload).await;
    tracing::info!(agent_id = %agent_id, delivered, "ws_route_to_agent");
    if !delivered {
        if let Some(request_id) = metadata_request_id {
            send_json(
                tx,
                json!({
                    "kind": "metadata_refresh_failed",
                    "request_id": request_id,
                    "error": "desktop agent is offline",
                }),
            );
        }
    }
    Ok(())
}

async fn validate_task_start(
    state: &AppState,
    payload: &Value,
    device_id: &str,
    agent_id: &str,
) -> Result<(), String> {
    if value_str_ws(payload, "device_id")? != device_id
        || value_str_ws(payload, "agent_id")? != agent_id
    {
        return Err("message routing mismatch".to_string());
    }
    let project_id = value_str_ws(payload, "project_id")?;
    let thread_id = payload.get("thread_id").and_then(Value::as_str);
    let ciphertext = value_str_ws(payload, "ciphertext")?;
    let signature = value_str_ws(payload, "signature")?;
    let digest = crypto::task_start_digest(device_id, agent_id, project_id, thread_id, ciphertext);
    let public_key = db::public_key(&state.pool, device_id)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "unknown device_id".to_string())?;
    if crypto::verify_ed25519(&public_key, &digest, signature) {
        Ok(())
    } else {
        Err("task_start signature invalid".to_string())
    }
}

async fn handle_agent_text(
    state: &AppState,
    agent_id: &str,
    tx: &mpsc::UnboundedSender<Message>,
    text: &str,
) -> Result<(), String> {
    let mut payload = parse_json(text)?;
    let kind = payload_kind(&payload)?.to_string();
    match kind.as_str() {
        "pair_handshake_ack" => {
            state
                .hub
                .complete_agent_pair_handshake(agent_id, &payload)
                .await;
            Ok(())
        }
        "project_sync" | "thread_sync" | "approval_sync" | "model_sync" | "user_input_sync" => {
            payload["source"] = json!("agent");
            payload["agent_id"] = json!(agent_id);
            for device_id in state.hub.connected_mobile_ids_for_agent(agent_id).await {
                state
                    .hub
                    .send_to_mobile(&device_id, payload.clone(), Some(agent_id))
                    .await;
            }
            Ok(())
        }
        "task_update"
        | "thread_sync_completed"
        | "thread_sync_failed"
        | "metadata_refresh_completed"
        | "metadata_refresh_failed"
        | "approval_request"
        | "user_input_request" => {
            let device_id = value_str_ws(&payload, "device_id")?.to_string();
            payload["source"] = json!("agent");
            payload["agent_id"] = json!(agent_id);
            let delivered = state
                .hub
                .send_to_mobile(&device_id, payload, Some(agent_id))
                .await;
            tracing::info!(
                agent_id = %agent_id,
                device_id = %device_id,
                kind = %kind,
                delivered,
                "ws_route_to_mobile"
            );
            if delivered {
                Ok(())
            } else {
                Err("target mobile websocket is not connected".to_string())
            }
        }
        "task_progress_push" => handle_task_progress_push(state, agent_id, &payload).await,
        "transfer_ready" => {
            // Transfer notifications are produced by this server. Ignore echoes
            // from older gateways instead of treating them as persisted state.
            let _ = tx;
            Ok(())
        }
        _ => Err(format!("unsupported agent message kind={kind}")),
    }
}

async fn handle_task_progress_push(
    state: &AppState,
    agent_id: &str,
    payload: &Value,
) -> Result<(), String> {
    let device_id = value_str_ws(payload, "device_id")?.to_string();
    let ciphertext = value_str_ws(payload, "ciphertext")?.to_string();
    let paired = db::is_paired(&state.pool, &device_id, agent_id)
        .await
        .map_err(|error| error.to_string())?;
    if !paired {
        return Err("devices are not paired".to_string());
    }
    let Some(push_token) = db::ios_push_token(&state.pool, &device_id)
        .await
        .map_err(|error| error.to_string())?
        .filter(|token| !token.is_empty())
    else {
        tracing::info!(
            agent_id = %agent_id,
            device_id = %device_id,
            "task_progress_push_skipped_missing_token"
        );
        return Ok(());
    };
    match state
        .push
        .send_task_progress(&push_token, agent_id, &ciphertext)
        .await
    {
        Ok(PushSendOutcome::Sent) => tracing::info!(
            agent_id = %agent_id,
            device_id = %device_id,
            "task_progress_push_sent"
        ),
        Ok(PushSendOutcome::Disabled) => tracing::warn!(
            agent_id = %agent_id,
            device_id = %device_id,
            "task_progress_push_disabled"
        ),
        Err(error) => tracing::warn!(
            agent_id = %agent_id,
            device_id = %device_id,
            "task_progress_push_failed: {error:#}"
        ),
    }
    Ok(())
}

async fn close_socket(mut socket: WebSocket, code: u16, reason: &'static str) {
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code,
            reason: reason.into(),
        })))
        .await;
}

fn send_error_and_close(tx: &mpsc::UnboundedSender<Message>, error: String) {
    send_json(tx, json!({"kind": "error", "detail": error}));
    let _ = tx.send(Message::Close(Some(CloseFrame {
        code: 4400,
        reason: "invalid message".into(),
    })));
}

fn send_json(tx: &mpsc::UnboundedSender<Message>, payload: Value) {
    if let Ok(text) = serde_json::to_string(&payload) {
        let _ = tx.send(Message::Text(text.into()));
    }
}

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|error| error.to_string())
}

fn payload_kind(payload: &Value) -> Result<&str, String> {
    value_str_ws(payload, "kind")
}

fn value_str<'a>(payload: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::BadRequest(format!("{key} is required")))
}

fn value_str_ws<'a>(payload: &'a Value, key: &str) -> Result<&'a str, String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{key} is required"))
}

fn session_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get("X-Session-Token")
        .and_then(|value| value.to_str().ok())
}

fn required_header<'a>(headers: &'a HeaderMap, key: &str) -> Result<&'a str, ApiError> {
    headers
        .get(key)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::Unauthorized(format!("missing {key}")))
}

fn transfer_error(error: anyhow::Error) -> ApiError {
    let message = error.to_string();
    if message.contains("maximum size") {
        ApiError::PayloadTooLarge(message)
    } else if message.contains("transfer not found") {
        ApiError::NotFound(message)
    } else if message.contains("payload not found") || message.contains("not completed") {
        ApiError::Conflict(message)
    } else {
        ApiError::BadRequest(message)
    }
}

#[derive(Debug, Deserialize)]
struct MobileWsQuery {
    device_id: String,
    agent_id: String,
    session_token: String,
}

#[derive(Debug, Deserialize)]
struct AgentWsQuery {
    agent_id: String,
    session_token: String,
}

#[derive(Debug, Deserialize)]
struct TransferDownloadQuery {
    device_id: String,
}
