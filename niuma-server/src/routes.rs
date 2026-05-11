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
    tracing::info!(
        device_id = %payload.device_id,
        device_type = %payload.device_type,
        "http_devices_register_in"
    );
    db::register_device(&state.pool, &payload).await?;
    tracing::info!(
        device_id = %payload.device_id,
        "http_devices_register_out"
    );
    Ok(Json(DeviceRegisterResponse {
        registered: true,
        server_time: chrono::Utc::now().timestamp(),
    }))
}

async fn auth_challenge(
    State(state): State<AppState>,
    Json(payload): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, ApiError> {
    tracing::info!(
        device_id = %payload.device_id,
        "http_auth_challenge_in"
    );
    let (challenge_id, challenge, expires_at) =
        db::issue_challenge(&state.pool, &state.settings, &payload.device_id).await?;
    tracing::info!(
        device_id = %payload.device_id,
        challenge_id = %challenge_id,
        "http_auth_challenge_out"
    );
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
    tracing::info!(
        device_id = %payload.device_id,
        challenge_id = %payload.challenge_id,
        "http_auth_verify_in"
    );
    let token = match db::verify_challenge(&state.pool, &state.settings, &payload).await {
        Ok(token) => token,
        Err(error) => {
            tracing::warn!(
                device_id = %payload.device_id,
                challenge_id = %payload.challenge_id,
                "http_auth_verify_failed: {error:#}"
            );
            return Err(error.into());
        }
    };
    tracing::info!(
        device_id = %payload.device_id,
        "http_auth_verify_out"
    );
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
    tracing::info!(
        agent_id = %payload.agent_id,
        "http_pair_request_in"
    );
    if let Err(error) = db::require_session(
        &state.pool,
        session_token(&headers),
        payload.agent_id.as_str(),
    )
    .await
    {
        tracing::warn!(
            agent_id = %payload.agent_id,
            status = %error.status(),
            "http_pair_request_auth_failed: {error}"
        );
        return Err(error);
    }
    let (pair_token, expires_at) = db::issue_pair_token(
        &state.pool,
        &state.settings,
        &payload.agent_id,
        &payload.agent_pairing_public_key,
    )
    .await?;
    tracing::info!(
        agent_id = %payload.agent_id,
        expires_at,
        "http_pair_request_out"
    );
    Ok(Json(PairRequestResponse {
        pair_token,
        expires_at,
    }))
}

async fn pair_confirm(
    State(state): State<AppState>,
    Json(payload): Json<PairConfirmRequest>,
) -> Result<Json<PairConfirmResponse>, ApiError> {
    tracing::info!(
        device_id = %payload.device_id,
        agent_id = %payload.agent_id,
        "http_pair_confirm_in"
    );
    let binding_id = db::validate_pair_confirm(&state.pool, &state.settings, &payload).await?;
    let ack = request_agent_pair_ack(&state, &payload, &binding_id).await?;
    validate_agent_pair_ack(&state, &payload, &binding_id, &ack).await?;
    db::commit_pair_confirm(&state.pool, &payload, &binding_id).await?;
    tracing::info!(
        device_id = %payload.device_id,
        agent_id = %payload.agent_id,
        binding_id = %binding_id,
        "http_pair_confirm_out"
    );
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
    tracing::info!(
        device_id = %payload.device_id,
        agent_id = %payload.agent_id,
        "http_pair_revoke_in"
    );
    db::require_session(
        &state.pool,
        session_token(&headers),
        payload.device_id.as_str(),
    )
    .await?;
    let revoked = db::revoke_pairing(&state.pool, &payload.device_id, &payload.agent_id).await?;
    tracing::info!(
        device_id = %payload.device_id,
        agent_id = %payload.agent_id,
        revoked,
        "http_pair_revoke_out"
    );
    Ok(Json(PairRevokeResponse { revoked }))
}

async fn update_push_token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<PushTokenUpdateRequest>,
) -> Result<Json<PushTokenUpdateResponse>, ApiError> {
    tracing::info!(
        device_id = %payload.device_id,
        "http_push_token_update_in"
    );
    db::require_session(&state.pool, session_token(&headers), &payload.device_id).await?;
    let updated =
        db::update_ios_push_token(&state.pool, &payload.device_id, &payload.push_token).await?;
    if !updated {
        tracing::warn!(
            device_id = %payload.device_id,
            "http_push_token_update_unknown_device"
        );
        return Err(ApiError::NotFound("unknown iOS device".to_string()));
    }
    tracing::info!(
        device_id = %payload.device_id,
        "http_push_token_update_out"
    );
    Ok(Json(PushTokenUpdateResponse { updated }))
}

async fn ensure_transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(transfer_id): Path<String>,
    Json(payload): Json<TransferEnsureRequest>,
) -> Result<Json<TransferEnsureResponse>, ApiError> {
    tracing::info!(
        transfer_id = %transfer_id,
        source_device_id = %payload.source_device_id,
        target_device_id = %payload.target_device_id,
        direction = %payload.direction,
        encrypted_size_bytes = payload.encrypted_size_bytes,
        "http_transfer_ensure_in"
    );
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
    tracing::info!(
        transfer_id = %result.manifest.transfer_id,
        needs_upload = result.needs_upload,
        expires_at = result.manifest.expires_at,
        "http_transfer_ensure_out"
    );
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
    tracing::info!(
        transfer_id = %transfer_id,
        body_bytes = body.len(),
        "http_transfer_upload_in"
    );
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
    tracing::info!(
        transfer_id = %transfer_id,
        source_device_id = %completed.source_device_id,
        target_device_id = %completed.target_device_id,
        expires_at = completed.expires_at,
        "http_transfer_upload_out"
    );
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
    tracing::info!(
        transfer_id = %transfer_id,
        device_id = %query.device_id,
        "http_transfer_download_in"
    );
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
    tracing::info!(
        transfer_id = %transfer_id,
        device_id = %query.device_id,
        body_bytes = body.len(),
        "http_transfer_download_out"
    );
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
    tracing::info!(
        transfer_id = %transfer_id,
        receiver_device_id = %payload.receiver_device_id,
        "http_transfer_ack_in"
    );
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
    tracing::info!(
        transfer_id = %transfer_id,
        receiver_device_id = %payload.receiver_device_id,
        "http_transfer_ack_out"
    );
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
        let delivered = state
            .hub
            .send_to_agent(&manifest.target_device_id, payload)
            .await;
        tracing::info!(
            transfer_id = %manifest.transfer_id,
            direction = %manifest.direction,
            target_device_id = %manifest.target_device_id,
            delivered,
            "transfer_ready_to_agent"
        );
    } else {
        let delivered = state
            .hub
            .send_to_mobile(&manifest.target_device_id, payload, None)
            .await;
        tracing::info!(
            transfer_id = %manifest.transfer_id,
            direction = %manifest.direction,
            target_device_id = %manifest.target_device_id,
            delivered,
            "transfer_ready_to_mobile"
        );
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
    tracing::info!(
        device_id = %query.device_id,
        agent_id = %query.agent_id,
        connection_id = %connection_id,
        "mobile_ws_connected"
    );
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
    tracing::info!(
        device_id = %query.device_id,
        agent_id = %query.agent_id,
        connection_id = %connection_id,
        "mobile_ws_disconnected"
    );
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
    tracing::info!(
        agent_id = %query.agent_id,
        connection_id = %connection_id,
        "agent_ws_connected"
    );
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
    tracing::info!(
        agent_id = %query.agent_id,
        connection_id = %connection_id,
        "agent_ws_disconnected"
    );
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
    if payload.get("request_id").is_none() {
        payload["request_id"] = json!(crypto::random_token(12));
    }
    tracing::info!(
        kind = %kind,
        request_id = %request_id(&payload),
        device_id = %query.device_id,
        agent_id = %query.agent_id,
        thread_id = %thread_id(&payload),
        cursor = cursor(&payload),
        checkpoint_present = checkpoint_present(&payload),
        payload_bytes = text.len(),
        "ws_mobile_in"
    );
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
        "branch_changes_request" => {
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_to_agent_or_error(state, tx, &query.agent_id, payload, None).await
        }
        "thread_archive_request" => {
            let request_id = payload
                .get("request_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let thread_id = payload
                .get("thread_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_thread_action_to_agent_or_error(
                state,
                tx,
                &query.agent_id,
                payload,
                request_id,
                thread_id,
                &query.device_id,
                "thread_archive_request",
                "thread_archive_failed",
            )
            .await
        }
        "thread_rename_request" => {
            let request_id = payload
                .get("request_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let thread_id = payload
                .get("thread_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_thread_action_to_agent_or_error(
                state,
                tx,
                &query.agent_id,
                payload,
                request_id,
                thread_id,
                &query.device_id,
                "thread_rename_request",
                "thread_rename_failed",
            )
            .await
        }
        "approval_response" => {
            let approval_id = payload
                .get("approval_id")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_approval_response_to_agent_or_error(
                state,
                tx,
                &query.agent_id,
                payload,
                approval_id,
            )
            .await
        }
        "user_input_response" => {
            payload["device_id"] = json!(query.device_id);
            payload["source"] = json!("mobile");
            route_to_agent_or_error(state, tx, &query.agent_id, payload, None).await
        }
        _ => Err(format!("unsupported mobile message kind={kind}")),
    }
}

async fn route_thread_action_to_agent_or_error(
    state: &AppState,
    tx: &mpsc::UnboundedSender<Message>,
    agent_id: &str,
    payload: Value,
    request_id: Option<String>,
    thread_id: Option<String>,
    device_id: &str,
    request_kind: &'static str,
    failure_kind: &'static str,
) -> Result<(), String> {
    tracing::info!(
        kind = request_kind,
        request_id = %request_id.as_deref().unwrap_or(""),
        agent_id = %agent_id,
        device_id = %device_id,
        thread_id = %thread_id.as_deref().unwrap_or(""),
        "ws_route_to_agent_start"
    );
    let delivered = state.hub.send_to_agent(agent_id, payload).await;
    tracing::info!(
        kind = request_kind,
        request_id = %request_id.as_deref().unwrap_or(""),
        agent_id = %agent_id,
        device_id = %device_id,
        thread_id = %thread_id.as_deref().unwrap_or(""),
        delivered,
        "ws_route_to_agent_done"
    );
    if !delivered {
        send_json(
            tx,
            json!({
                "kind": failure_kind,
                "device_id": device_id,
                "request_id": request_id.unwrap_or_default(),
                "thread_id": thread_id.unwrap_or_default(),
                "error": "desktop agent is offline",
            }),
        );
    }
    Ok(())
}

async fn route_approval_response_to_agent_or_error(
    state: &AppState,
    tx: &mpsc::UnboundedSender<Message>,
    agent_id: &str,
    payload: Value,
    approval_id: Option<String>,
) -> Result<(), String> {
    let device_id = device_id(&payload);
    let request_id = request_id(&payload);
    tracing::info!(
        kind = "approval_response",
        request_id = %request_id,
        agent_id = %agent_id,
        device_id = %device_id,
        approval_id = %approval_id.as_deref().unwrap_or(""),
        "ws_route_to_agent_start"
    );
    let delivered = state.hub.send_to_agent(agent_id, payload).await;
    tracing::info!(
        kind = "approval_response",
        request_id = %request_id,
        agent_id = %agent_id,
        device_id = %device_id,
        approval_id = %approval_id.as_deref().unwrap_or(""),
        delivered,
        "ws_route_to_agent_done"
    );
    if !delivered {
        send_json(
            tx,
            json!({
                "kind": "approval_response_failed",
                "approval_id": approval_id.unwrap_or_default(),
                "error": "desktop agent is offline",
            }),
        );
    }
    Ok(())
}

async fn route_to_agent_or_error(
    state: &AppState,
    tx: &mpsc::UnboundedSender<Message>,
    agent_id: &str,
    payload: Value,
    metadata_request_id: Option<String>,
) -> Result<(), String> {
    let kind = payload_kind(&payload).unwrap_or("unknown").to_string();
    let request_id = request_id(&payload);
    let device_id = device_id(&payload);
    let thread_id = thread_id(&payload);
    let cursor = cursor(&payload);
    let checkpoint_present = checkpoint_present(&payload);
    tracing::info!(
        kind = %kind,
        request_id = %request_id,
        agent_id = %agent_id,
        device_id = %device_id,
        thread_id = %thread_id,
        cursor,
        checkpoint_present,
        "ws_route_to_agent_start"
    );
    let delivered = state.hub.send_to_agent(agent_id, payload).await;
    tracing::info!(
        kind = %kind,
        request_id = %request_id,
        agent_id = %agent_id,
        device_id = %device_id,
        thread_id = %thread_id,
        cursor,
        checkpoint_present,
        delivered,
        "ws_route_to_agent_done"
    );
    if !delivered && let Some(request_id) = metadata_request_id {
        send_json(
            tx,
            json!({
                "kind": "metadata_refresh_failed",
                "request_id": request_id,
                "device_id": device_id,
                "error": "desktop agent is offline",
            }),
        );
    }
    if !delivered && kind == "resume_thread" {
        send_json(
            tx,
            json!({
                "kind": "thread_sync_failed",
                "request_id": empty_to_null(&request_id),
                "device_id": device_id,
                "thread_id": thread_id,
                "cursor": cursor,
                "checkpoint": None::<String>,
                "error": "desktop agent is offline",
            }),
        );
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
    tracing::info!(
        kind = %kind,
        request_id = %request_id(&payload),
        agent_id = %agent_id,
        device_id = %device_id(&payload),
        thread_id = %thread_id(&payload),
        cursor = cursor(&payload),
        entry_count = entry_count(&payload),
        payload_bytes = text.len(),
        "ws_agent_in"
    );
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
                let delivered = state
                    .hub
                    .send_to_mobile(&device_id, payload.clone(), Some(agent_id))
                    .await;
                tracing::info!(
                    kind = %kind,
                    request_id = %request_id(&payload),
                    agent_id = %agent_id,
                    device_id = %device_id,
                    delivered,
                    "ws_route_to_mobile_done"
                );
            }
            Ok(())
        }
        "task_update"
        | "thread_sync_completed"
        | "thread_sync_failed"
        | "metadata_refresh_completed"
        | "metadata_refresh_failed"
        | "branch_changes_result"
        | "branch_changes_failed"
        | "thread_archive_result"
        | "thread_archive_failed"
        | "thread_rename_result"
        | "thread_rename_failed"
        | "approval_request"
        | "approval_response_failed"
        | "user_input_request" => {
            deliver_agent_payload_to_mobile(state, agent_id, &kind, payload).await
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

async fn deliver_agent_payload_to_mobile(
    state: &AppState,
    agent_id: &str,
    kind: &str,
    mut payload: Value,
) -> Result<(), String> {
    let device_id = value_str_ws(&payload, "device_id")?.to_string();
    let request_id = request_id(&payload);
    let thread_id = thread_id(&payload);
    let cursor = cursor(&payload);
    let entry_count = entry_count(&payload);
    payload["source"] = json!("agent");
    payload["agent_id"] = json!(agent_id);
    tracing::info!(
        agent_id = %agent_id,
        device_id = %device_id,
        kind,
        request_id = %request_id,
        thread_id = %thread_id,
        cursor,
        entry_count,
        "ws_route_to_mobile_start"
    );
    let delivered = state
        .hub
        .send_to_mobile(&device_id, payload, Some(agent_id))
        .await;
    tracing::info!(
        agent_id = %agent_id,
        device_id = %device_id,
        kind,
        request_id = %request_id,
        thread_id = %thread_id,
        cursor,
        entry_count,
        delivered,
        "ws_route_to_mobile_done"
    );
    // Mobile WebSocket delivery is the server->mobile channel. A missing mobile
    // connection is expected when iOS is locked or offline and must not break
    // the independent gateway->server channel that may carry the APNs trigger.
    Ok(())
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
    tracing::warn!(error = %error, "websocket_error_close");
    send_json(tx, json!({"kind": "error", "detail": error}));
    let _ = tx.send(Message::Close(Some(CloseFrame {
        code: 4400,
        reason: "invalid message".into(),
    })));
}

fn send_json(tx: &mpsc::UnboundedSender<Message>, payload: Value) {
    if let Ok(text) = serde_json::to_string(&payload) {
        tracing::info!(
            kind = %payload_kind(&payload).unwrap_or("unknown"),
            request_id = %request_id(&payload),
            device_id = %device_id(&payload),
            thread_id = %thread_id(&payload),
            payload_bytes = text.len(),
            "ws_server_out"
        );
        let _ = tx.send(Message::Text(text.into()));
    }
}

fn parse_json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|error| error.to_string())
}

fn payload_kind(payload: &Value) -> Result<&str, String> {
    value_str_ws(payload, "kind")
}

fn request_id(payload: &Value) -> String {
    payload
        .get("request_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn device_id(payload: &Value) -> String {
    payload
        .get("device_id")
        .or_else(|| payload.get("source_device_id"))
        .or_else(|| payload.get("target_device_id"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn thread_id(payload: &Value) -> String {
    payload
        .get("thread_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn cursor(payload: &Value) -> i64 {
    payload.get("cursor").and_then(Value::as_i64).unwrap_or(0)
}

fn entry_count(payload: &Value) -> i64 {
    payload
        .get("entry_count")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

fn checkpoint_present(payload: &Value) -> bool {
    payload
        .get("checkpoint")
        .is_some_and(|value| !value.is_null())
}

fn empty_to_null(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        json!(value)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        apns::ApnsPushService, config::Settings, hub::ConnectionHub, transfer::TransferStore,
    };
    use sqlx::postgres::PgPoolOptions;
    use std::{sync::Arc, time::Duration};
    use tokio::sync::mpsc;

    fn test_settings() -> Settings {
        Settings {
            host: "127.0.0.1".to_string(),
            port: 8000,
            log_level: "info".to_string(),
            log_dir: std::env::temp_dir().join("niuma-server-logs-test"),
            log_retention_days: crate::logging::default_retention_days(),
            database_url: "postgres://localhost/niuma_test".to_string(),
            database_pool_size: 1,
            database_connect_timeout: Duration::from_secs(1),
            challenge_ttl_seconds: 120,
            pair_token_ttl_seconds: 300,
            session_token_ttl_seconds: 3600,
            nonce_ttl_seconds: 600,
            auth_timestamp_tolerance_seconds: 120,
            pair_token_max_attempts: 5,
            transfer_storage_dir: std::env::temp_dir()
                .join(format!("niuma-routes-test-{}", crypto::random_token(8))),
            transfer_ttl_seconds: 1800,
            transfer_max_encrypted_bytes: 1024,
            apns_key_id: None,
            apns_team_id: None,
            apns_topic: None,
            apns_auth_key_path: None,
            apns_auth_key_pem: None,
            apns_environment: "sandbox".to_string(),
        }
    }

    fn test_state() -> AppState {
        let settings = test_settings();
        let pool = PgPoolOptions::new()
            .connect_lazy(&settings.database_url)
            .expect("create lazy postgres pool");
        AppState {
            transfers: Arc::new(TransferStore::new(&settings).expect("create transfer store")),
            push: Arc::new(ApnsPushService::new(&settings).expect("create APNs service")),
            settings,
            pool,
            hub: Arc::new(ConnectionHub::default()),
        }
    }

    async fn recv_json(rx: &mut mpsc::UnboundedReceiver<Message>) -> Value {
        let message = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .expect("receive routed websocket message")
            .expect("message is present");
        match message {
            Message::Text(text) => serde_json::from_str(&text.to_string()).expect("json text"),
            other => panic!("expected text websocket message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn mobile_branch_changes_request_routes_to_gateway() {
        let state = test_state();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        state.hub.connect_agent("agent-1", agent_tx).await;
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        let query = MobileWsQuery {
            device_id: "ios-device".to_string(),
            agent_id: "agent-1".to_string(),
            session_token: "session-token".to_string(),
        };
        let request = json!({
            "kind": "branch_changes_request",
            "request_id": "request-1",
            "device_id": "spoofed-device",
            "thread_id": "thread-1",
            "base_ref": "main"
        });

        handle_mobile_text(&state, &query, &mobile_tx, &request.to_string())
            .await
            .expect("route branch changes request");

        let routed = recv_json(&mut agent_rx).await;
        assert_eq!(routed["kind"], "branch_changes_request");
        assert_eq!(routed["request_id"], "request-1");
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["base_ref"], "main");
        assert_eq!(routed["source"], "mobile");
        assert!(mobile_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn mobile_thread_archive_request_routes_to_gateway() {
        let state = test_state();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        state.hub.connect_agent("agent-1", agent_tx).await;
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        let query = MobileWsQuery {
            device_id: "ios-device".to_string(),
            agent_id: "agent-1".to_string(),
            session_token: "session-token".to_string(),
        };
        let request = json!({
            "kind": "thread_archive_request",
            "request_id": "archive-1",
            "device_id": "spoofed-device",
            "thread_id": "thread-1"
        });

        handle_mobile_text(&state, &query, &mobile_tx, &request.to_string())
            .await
            .expect("route archive request");

        let routed = recv_json(&mut agent_rx).await;
        assert_eq!(routed["kind"], "thread_archive_request");
        assert_eq!(routed["request_id"], "archive-1");
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["source"], "mobile");
        assert!(mobile_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn mobile_thread_archive_request_reports_offline_gateway() {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        let query = MobileWsQuery {
            device_id: "ios-device".to_string(),
            agent_id: "agent-offline".to_string(),
            session_token: "session-token".to_string(),
        };
        let request = json!({
            "kind": "thread_archive_request",
            "request_id": "archive-1",
            "thread_id": "thread-1"
        });

        handle_mobile_text(&state, &query, &mobile_tx, &request.to_string())
            .await
            .expect("offline archive request reports failure to mobile");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], "thread_archive_failed");
        assert_eq!(routed["request_id"], "archive-1");
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["error"], "desktop agent is offline");
    }

    #[tokio::test]
    async fn mobile_thread_rename_request_routes_to_gateway() {
        let state = test_state();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        state.hub.connect_agent("agent-1", agent_tx).await;
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        let query = MobileWsQuery {
            device_id: "ios-device".to_string(),
            agent_id: "agent-1".to_string(),
            session_token: "session-token".to_string(),
        };
        let request = json!({
            "kind": "thread_rename_request",
            "request_id": "rename-1",
            "device_id": "spoofed-device",
            "thread_id": "thread-1",
            "title": "New title"
        });

        handle_mobile_text(&state, &query, &mobile_tx, &request.to_string())
            .await
            .expect("route rename request");

        let routed = recv_json(&mut agent_rx).await;
        assert_eq!(routed["kind"], "thread_rename_request");
        assert_eq!(routed["request_id"], "rename-1");
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["title"], "New title");
        assert_eq!(routed["source"], "mobile");
        assert!(mobile_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn mobile_thread_rename_request_reports_offline_gateway() {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        let query = MobileWsQuery {
            device_id: "ios-device".to_string(),
            agent_id: "agent-offline".to_string(),
            session_token: "session-token".to_string(),
        };
        let request = json!({
            "kind": "thread_rename_request",
            "request_id": "rename-1",
            "thread_id": "thread-1",
            "title": "New title"
        });

        handle_mobile_text(&state, &query, &mobile_tx, &request.to_string())
            .await
            .expect("offline rename request reports failure to mobile");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], "thread_rename_failed");
        assert_eq!(routed["request_id"], "rename-1");
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["error"], "desktop agent is offline");
    }

    #[tokio::test]
    async fn mobile_approval_response_reports_offline_gateway() {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        let query = MobileWsQuery {
            device_id: "ios-device".to_string(),
            agent_id: "agent-offline".to_string(),
            session_token: "session-token".to_string(),
        };
        let request = json!({
            "kind": "approval_response",
            "approval_id": "approval-1",
            "ciphertext": "encrypted-payload"
        });

        handle_mobile_text(&state, &query, &mobile_tx, &request.to_string())
            .await
            .expect("offline approval response reports failure to mobile");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], "approval_response_failed");
        assert_eq!(routed["approval_id"], "approval-1");
        assert_eq!(routed["error"], "desktop agent is offline");
    }

    #[tokio::test]
    async fn agent_branch_changes_result_routes_to_mobile() {
        assert_agent_branch_changes_routes_to_mobile("branch_changes_result").await;
    }

    #[tokio::test]
    async fn agent_branch_changes_failed_routes_to_mobile() {
        assert_agent_branch_changes_routes_to_mobile("branch_changes_failed").await;
    }

    #[tokio::test]
    async fn agent_thread_archive_result_routes_to_mobile() {
        assert_agent_thread_archive_routes_to_mobile("thread_archive_result").await;
    }

    #[tokio::test]
    async fn agent_thread_archive_failed_routes_to_mobile() {
        assert_agent_thread_archive_routes_to_mobile("thread_archive_failed").await;
    }

    #[tokio::test]
    async fn agent_thread_rename_result_routes_to_mobile() {
        assert_agent_thread_rename_routes_to_mobile("thread_rename_result").await;
    }

    #[tokio::test]
    async fn agent_thread_rename_failed_routes_to_mobile() {
        assert_agent_thread_rename_routes_to_mobile("thread_rename_failed").await;
    }

    #[tokio::test]
    async fn agent_approval_response_failed_routes_to_mobile() {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        state
            .hub
            .connect_mobile("ios-device", "agent-1", mobile_tx)
            .await;
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        let response = json!({
            "kind": "approval_response_failed",
            "device_id": "ios-device",
            "approval_id": "approval-1",
            "error": "approval request is no longer pending"
        });

        handle_agent_text(&state, "agent-1", &agent_tx, &response.to_string())
            .await
            .expect("route approval response failure");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], "approval_response_failed");
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["approval_id"], "approval-1");
        assert_eq!(routed["error"], "approval request is no longer pending");
        assert_eq!(routed["source"], "agent");
        assert_eq!(routed["agent_id"], "agent-1");
        assert!(agent_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn offline_mobile_delivery_does_not_fail_agent_channel() {
        let state = test_state();
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();

        for kind in ["task_update", "thread_sync_completed"] {
            let message = json!({
                "kind": kind,
                "device_id": "ios-offline",
                "thread_id": "thread-1",
                "seq": 1,
                "cursor": 1,
                "entry_count": 1,
                "ciphertext": "encrypted-payload"
            });

            handle_agent_text(&state, "agent-1", &agent_tx, &message.to_string())
                .await
                .expect("offline mobile websocket is not an agent channel failure");
        }

        assert!(agent_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn failed_mobile_delivery_prunes_b_channel_connection() {
        let state = test_state();
        let (mobile_tx, mobile_rx) = mpsc::unbounded_channel();
        state
            .hub
            .connect_mobile("ios-stale", "agent-1", mobile_tx)
            .await;
        drop(mobile_rx);
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        let message = json!({
            "kind": "task_update",
            "device_id": "ios-stale",
            "thread_id": "thread-1",
            "seq": 1,
            "ciphertext": "encrypted-payload"
        });

        handle_agent_text(&state, "agent-1", &agent_tx, &message.to_string())
            .await
            .expect("stale mobile websocket is handled in the mobile channel");

        assert!(agent_rx.try_recv().is_err());
        assert!(
            state
                .hub
                .connected_mobile_ids_for_agent("agent-1")
                .await
                .is_empty()
        );
    }

    async fn assert_agent_branch_changes_routes_to_mobile(kind: &str) {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        state
            .hub
            .connect_mobile("ios-device", "agent-1", mobile_tx)
            .await;
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        let response = json!({
            "kind": kind,
            "device_id": "ios-device",
            "request_id": "request-1",
            "thread_id": "thread-1",
            "ciphertext": "encrypted-payload"
        });

        handle_agent_text(&state, "agent-1", &agent_tx, &response.to_string())
            .await
            .expect("route branch changes response");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], kind);
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["request_id"], "request-1");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["ciphertext"], "encrypted-payload");
        assert_eq!(routed["source"], "agent");
        assert_eq!(routed["agent_id"], "agent-1");
        assert!(agent_rx.try_recv().is_err());
    }

    async fn assert_agent_thread_archive_routes_to_mobile(kind: &str) {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        state
            .hub
            .connect_mobile("ios-device", "agent-1", mobile_tx)
            .await;
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        let response = json!({
            "kind": kind,
            "device_id": "ios-device",
            "request_id": "archive-1",
            "thread_id": "thread-1",
            "error": "boom"
        });

        handle_agent_text(&state, "agent-1", &agent_tx, &response.to_string())
            .await
            .expect("route archive response");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], kind);
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["request_id"], "archive-1");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["source"], "agent");
        assert_eq!(routed["agent_id"], "agent-1");
        assert!(agent_rx.try_recv().is_err());
    }

    async fn assert_agent_thread_rename_routes_to_mobile(kind: &str) {
        let state = test_state();
        let (mobile_tx, mut mobile_rx) = mpsc::unbounded_channel();
        state
            .hub
            .connect_mobile("ios-device", "agent-1", mobile_tx)
            .await;
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel();
        let response = json!({
            "kind": kind,
            "device_id": "ios-device",
            "request_id": "rename-1",
            "thread_id": "thread-1",
            "error": "boom"
        });

        handle_agent_text(&state, "agent-1", &agent_tx, &response.to_string())
            .await
            .expect("route rename response");

        let routed = recv_json(&mut mobile_rx).await;
        assert_eq!(routed["kind"], kind);
        assert_eq!(routed["device_id"], "ios-device");
        assert_eq!(routed["request_id"], "rename-1");
        assert_eq!(routed["thread_id"], "thread-1");
        assert_eq!(routed["source"], "agent");
        assert_eq!(routed["agent_id"], "agent-1");
        assert!(agent_rx.try_recv().is_err());
    }
}
