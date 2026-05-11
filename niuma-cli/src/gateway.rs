//! Foreground gateway runtime and loopback HTTP control surface.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

use crate::bindings::{self, PairedDeviceBinding};
use crate::cli::GatewayArgs;
use crate::codex::{self, CodexRuntime};
use crate::codex_app_server::CodexAppServerClient;
use crate::config::{self, ConfigValueSource, GatewayConfig};
use crate::identity::AgentIdentity;
use crate::pairing::{self, PairingMaterial, PairingPayload, PairingRuntimeState};
use crate::paths;
use crate::realtime::{self, AgentChannelStatus};
use crate::server::{NiumaServerClient, is_unauthorized_response};

#[derive(Clone)]
struct GatewayState {
    config: GatewayConfig,
    identity: AgentIdentity,
    codex_runtime: CodexRuntime,
    codex_app_server: Option<CodexAppServerClient>,
    server: Option<NiumaServerClient>,
    session_token: Option<Arc<RwLock<String>>>,
    pairing: Arc<RwLock<PairingRuntimeState>>,
    agent_channel: Arc<RwLock<AgentChannelStatus>>,
    started_at: i64,
}

#[derive(Debug, Serialize)]
struct ApiError {
    detail: String,
}

#[derive(Debug, Serialize)]
struct GatewayStatus {
    status: &'static str,
    mode: &'static str,
    agent_id: String,
    device_name: String,
    state_root: String,
    config_path: String,
    server_url: String,
    server_url_source: ConfigValueSource,
    saved_server_url: Option<String>,
    server_connected: bool,
    authenticated: bool,
    agent_ws_connected: bool,
    agent_ws_last_connected_at: Option<i64>,
    agent_ws_last_error: Option<String>,
    pairing_payload_ready: bool,
    pair_token_expires_at: Option<i64>,
    codex_runtime: CodexRuntime,
    codex_app_server_running: bool,
    dashboard_url: String,
    open_browser: bool,
    started_at: i64,
    last_pairing_error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ServerUrlRequest {
    server_url: String,
}

#[derive(Debug, Serialize)]
struct ServerUrlUpdateResponse {
    server_url: String,
    active_server_url: String,
    server_url_source: ConfigValueSource,
    config_path: String,
    restart_required: bool,
    restart_required_reason: &'static str,
    saved_config_overridden_on_restart: bool,
}

#[derive(Debug, Serialize)]
struct ServerUrlTestResponse {
    server_url: String,
    reachable: bool,
}

#[derive(Debug, Serialize)]
struct PairingDeleteResponse {
    binding_id: String,
    device_id: String,
    server_revoked: bool,
    local_removed: bool,
}

/// Run the local gateway HTTP service and pairing token maintenance loop.
pub async fn run(args: GatewayArgs) -> Result<()> {
    let config = GatewayConfig::from_args(&args)?;
    paths::ensure_state_dirs()?;
    let identity = AgentIdentity::load_or_create(&config.device_name)?;
    let codex_runtime = codex::resolve(config.disable_codex_plugins);
    let codex_app_server = if config.pairing_page_only {
        None
    } else {
        Some(CodexAppServerClient::start(&codex_runtime.command).await?)
    };

    let (server, session_token, initial_pairing) = if config.pairing_page_only {
        (
            None,
            None,
            PairingRuntimeState {
                payload: None,
                last_error: Some("pairing_page_only mode has no live server payload".to_string()),
                secrets: Default::default(),
            },
        )
    } else {
        let server = NiumaServerClient::new(&config.server_url)?;
        server
            .health()
            .await
            .with_context(|| format!("niuma-server is unavailable at {}", config.server_url))?;
        server.register_agent(&identity).await?;
        let session_token = Arc::new(RwLock::new(server.authenticate_agent(&identity).await?));
        let current_session_token = session_token.read().await.clone();
        let material = pairing::refresh_payload(&server, &identity, &current_session_token).await?;
        let mut secrets = std::collections::HashMap::new();
        secrets.insert(material.secret.pair_token.clone(), material.secret);
        (
            Some(server),
            Some(session_token),
            PairingRuntimeState {
                payload: Some(material.payload),
                last_error: None,
                secrets,
            },
        )
    };

    let state = GatewayState {
        config: config.clone(),
        identity,
        codex_runtime,
        codex_app_server,
        server,
        session_token,
        pairing: Arc::new(RwLock::new(initial_pairing)),
        agent_channel: Arc::new(RwLock::new(AgentChannelStatus::default())),
        started_at: unix_timestamp(),
    };

    spawn_pairing_refresh(state.clone());
    if !config.pairing_page_only {
        realtime::spawn_agent_channel(
            state.identity.clone(),
            config.clone(),
            state.session_token.clone(),
            state.pairing.clone(),
            state.codex_app_server.clone(),
            state.server.clone(),
            state.agent_channel.clone(),
        );
    }

    let dashboard_url = dashboard_url(&config);
    let app = dashboard_router(state);

    let listener = TcpListener::bind((config.dashboard_host.as_str(), config.dashboard_port))
        .await
        .with_context(|| format!("failed to bind {}", dashboard_url))?;

    if config.open_browser {
        open_browser(&dashboard_url);
    }
    info!("niuma gateway listening on {}", dashboard_url);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn dashboard_router(state: GatewayState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/static/dashboard.css", get(dashboard_css))
        .route("/static/dashboard.js", get(dashboard_js))
        .route("/api/status", get(api_status))
        .route("/api/config/server-url", put(update_server_url))
        .route("/api/config/server-url/test", post(test_server_url))
        .route("/api/pairings", get(paired_devices))
        .route("/api/pairings/{binding_id}", delete(delete_pairing))
        .route("/api/pairing/payload", get(pairing_payload))
        .route("/api/pairing/qr.svg", get(pairing_qr_svg))
        .route("/api/pairing/refresh", post(refresh_pairing_payload))
        .with_state(state)
}

fn spawn_pairing_refresh(state: GatewayState) {
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(state.config.heartbeat_seconds)).await;
            let Some(payload) = state.pairing.read().await.payload.clone() else {
                continue;
            };
            if payload.expires_at - unix_timestamp() > 60 {
                continue;
            }
            if let Err(err) = refresh_pairing(&state).await {
                warn!("pair token refresh failed: {err:#}");
                state.pairing.write().await.last_error = Some(err.to_string());
            }
        }
    });
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../static/dashboard.html"))
}

async fn dashboard_css() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        include_str!("../static/dashboard.css"),
    )
}

async fn dashboard_js() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "application/javascript; charset=utf-8",
        )],
        include_str!("../static/dashboard.js"),
    )
}

async fn api_status(State(state): State<GatewayState>) -> Json<GatewayStatus> {
    Json(build_status(&state).await)
}

async fn update_server_url(
    State(state): State<GatewayState>,
    Json(request): Json<ServerUrlRequest>,
) -> Result<Json<ServerUrlUpdateResponse>, (StatusCode, Json<ApiError>)> {
    let normalized = normalize_server_url(&request.server_url)
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    config::save_server_url(&normalized)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let config_path = paths::config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_default();
    let active_server_url = active_server_url(&state);
    let source = state.config.server_url_source;
    Ok(Json(ServerUrlUpdateResponse {
        server_url: normalized,
        active_server_url,
        server_url_source: source,
        config_path,
        restart_required: true,
        restart_required_reason: "server_url is read during gateway startup",
        saved_config_overridden_on_restart: matches!(
            source,
            ConfigValueSource::Cli | ConfigValueSource::Env
        ),
    }))
}

async fn test_server_url(
    Json(request): Json<ServerUrlRequest>,
) -> Result<Json<ServerUrlTestResponse>, (StatusCode, Json<ApiError>)> {
    let server = NiumaServerClient::new(request.server_url.trim())
        .map_err(|err| api_error(StatusCode::BAD_REQUEST, err.to_string()))?;
    server
        .health()
        .await
        .map_err(|err| api_error(StatusCode::BAD_GATEWAY, err.to_string()))?;
    Ok(Json(ServerUrlTestResponse {
        server_url: server.base_url().to_string(),
        reachable: true,
    }))
}

async fn paired_devices() -> Result<Json<Vec<PairedDeviceBinding>>, (StatusCode, Json<ApiError>)> {
    bindings::list_bindings()
        .map(Json)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

async fn delete_pairing(
    State(state): State<GatewayState>,
    Path(binding_id): Path<String>,
) -> Result<Json<PairingDeleteResponse>, (StatusCode, Json<ApiError>)> {
    let binding = bindings::binding_for_id(&binding_id)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .ok_or_else(|| api_error(StatusCode::NOT_FOUND, "local binding not found".to_string()))?;
    if binding.agent_id != state.identity.agent_id {
        return Err(api_error(
            StatusCode::CONFLICT,
            "binding belongs to a different desktop agent".to_string(),
        ));
    }
    let server = state.server.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server connection is unavailable; local binding was not removed".to_string(),
        )
    })?;
    let session_token = state.session_token.as_ref().ok_or_else(|| {
        api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent session is unavailable; local binding was not removed".to_string(),
        )
    })?;
    let token = session_token.read().await.clone();
    let revoke = server
        .revoke_pair_binding(&binding.binding_id, &binding.agent_id, &token)
        .await
        .map_err(|err| api_error(StatusCode::BAD_GATEWAY, err.to_string()))?;
    if revoke.binding_id != binding.binding_id {
        return Err(api_error(
            StatusCode::BAD_GATEWAY,
            "server returned a mismatched binding id".to_string(),
        ));
    }
    let removed = bindings::delete_binding(&binding.binding_id)
        .map_err(|err| api_error(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?
        .is_some();
    Ok(Json(PairingDeleteResponse {
        binding_id: revoke.binding_id,
        device_id: binding.device_id,
        server_revoked: revoke.revoked,
        local_removed: removed,
    }))
}

async fn pairing_payload(
    State(state): State<GatewayState>,
) -> Result<Json<PairingPayload>, (StatusCode, Json<ApiError>)> {
    match state.pairing.read().await.payload.clone() {
        Some(payload) => Ok(Json(payload)),
        None => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                detail: state
                    .pairing
                    .read()
                    .await
                    .last_error
                    .clone()
                    .unwrap_or_else(|| "pairing payload is not ready".to_string()),
            }),
        )),
    }
}

fn normalize_server_url(server_url: &str) -> Result<String> {
    let trimmed = server_url.trim();
    if trimmed.is_empty() {
        anyhow::bail!("server_url must not be empty");
    }
    Ok(NiumaServerClient::new(trimmed)?.base_url().to_string())
}

fn api_error(status: StatusCode, detail: String) -> (StatusCode, Json<ApiError>) {
    (status, Json(ApiError { detail }))
}

async fn refresh_pairing_payload(
    State(state): State<GatewayState>,
) -> Result<Json<PairingPayload>, (StatusCode, Json<ApiError>)> {
    match refresh_pairing(&state).await {
        Ok(payload) => Ok(Json(payload)),
        Err(err) => Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ApiError {
                detail: err.to_string(),
            }),
        )),
    }
}

async fn pairing_qr_svg(State(state): State<GatewayState>) -> impl IntoResponse {
    let Some(payload) = state.pairing.read().await.payload.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            [(header::CONTENT_TYPE, "application/json")],
            serde_json::to_string(&ApiError {
                detail: "pairing payload is not ready".to_string(),
            })
            .unwrap_or_default(),
        );
    };
    let qr_json = match payload.to_qr_json() {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::to_string(&ApiError {
                    detail: err.to_string(),
                })
                .unwrap_or_default(),
            );
        }
    };
    let code = match QrCode::new(qr_json.as_bytes()) {
        Ok(value) => value,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "application/json")],
                serde_json::to_string(&ApiError {
                    detail: err.to_string(),
                })
                .unwrap_or_default(),
            );
        }
    };
    let image = code
        .render::<svg::Color<'_>>()
        .min_dimensions(320, 320)
        .build();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "image/svg+xml")],
        image,
    )
}

async fn refresh_pairing(state: &GatewayState) -> Result<PairingPayload> {
    let server = state
        .server
        .as_ref()
        .context("gateway is not connected to niuma-server")?;
    let session_token = state
        .session_token
        .as_ref()
        .context("gateway is not authenticated")?;
    let current_session_token = session_token.read().await.clone();
    let material =
        match pairing::refresh_payload(server, &state.identity, &current_session_token).await {
            Ok(material) => material,
            Err(err) if is_unauthorized_response(&err) => {
                info!("gateway session token expired; re-authenticating");
                // Pairing refresh is the periodic boundary that first observes expired
                // session tokens, so renew the shared token before retrying the request.
                let renewed_session_token = server
                    .authenticate_agent(&state.identity)
                    .await
                    .context("gateway session re-authentication failed")?;
                *session_token.write().await = renewed_session_token.clone();
                pairing::refresh_payload(server, &state.identity, &renewed_session_token).await?
            }
            Err(err) => return Err(err),
        };
    {
        let mut pairing_state = state.pairing.write().await;
        install_pairing_material(&mut pairing_state, material.clone());
    }
    Ok(material.payload)
}

fn install_pairing_material(pairing_state: &mut PairingRuntimeState, material: PairingMaterial) {
    pairing_state
        .secrets
        .retain(|_, secret| secret.expires_at > unix_timestamp());
    pairing_state
        .secrets
        .insert(material.secret.pair_token.clone(), material.secret);
    pairing_state.payload = Some(material.payload);
    pairing_state.last_error = None;
}

async fn build_status(state: &GatewayState) -> GatewayStatus {
    let pairing = state.pairing.read().await;
    let agent_channel = state.agent_channel.read().await.clone();
    let saved_config = config::read_config_file().ok().flatten();
    GatewayStatus {
        status: "ok",
        mode: if state.config.pairing_page_only {
            "pairing_page_only"
        } else {
            "gateway"
        },
        agent_id: state.identity.agent_id.clone(),
        device_name: state.identity.device_name.clone(),
        state_root: paths::state_root()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        config_path: paths::config_path()
            .map(|path| path.display().to_string())
            .unwrap_or_default(),
        server_url: active_server_url(state),
        server_url_source: state.config.server_url_source,
        saved_server_url: saved_config.and_then(|file| file.server_url),
        server_connected: state.server.is_some(),
        authenticated: state.session_token.is_some(),
        agent_ws_connected: agent_channel.connected,
        agent_ws_last_connected_at: agent_channel.last_connected_at,
        agent_ws_last_error: agent_channel.last_error,
        pairing_payload_ready: pairing.payload.is_some(),
        pair_token_expires_at: pairing.payload.as_ref().map(|payload| payload.expires_at),
        codex_runtime: state.codex_runtime.clone(),
        codex_app_server_running: state.codex_app_server.is_some(),
        dashboard_url: dashboard_url(&state.config),
        open_browser: state.config.open_browser,
        started_at: state.started_at,
        last_pairing_error: pairing.last_error.clone(),
    }
}

fn active_server_url(state: &GatewayState) -> String {
    state
        .server
        .as_ref()
        .map(|server| server.base_url().to_string())
        .unwrap_or_else(|| state.config.server_url.clone())
}

fn dashboard_url(config: &GatewayConfig) -> String {
    format!("http://{}:{}", config.dashboard_host, config.dashboard_port)
}

fn open_browser(url: &str) {
    if let Err(err) = std::process::Command::new("open").arg(url).spawn() {
        warn!("failed to open browser for {url}: {err}");
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!("failed to listen for shutdown signal: {err}");
    }
}

fn unix_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use axum::{
        extract::Path,
        http::HeaderMap,
        routing::{delete, get},
    };
    use serde_json::json;
    use tokio::task::JoinHandle;
    use uuid::Uuid;

    static HOME_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    #[tokio::test]
    async fn update_server_url_endpoint_persists_config_and_requires_restart() {
        let home = temp_home("server-url");
        let _home = HomeOverride::new(&home);
        let (dashboard_url, dashboard_task) = spawn_dashboard().await;

        let response = reqwest::Client::new()
            .put(format!("{dashboard_url}/api/config/server-url"))
            .json(&json!({ "server_url": "https://example.invalid/niuma-server" }))
            .send()
            .await
            .expect("dashboard request");

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await.expect("json response");
        assert_eq!(body["restart_required"], true);
        assert_eq!(body["server_url"], "https://example.invalid/niuma-server/");

        let config_text =
            std::fs::read_to_string(home.join(".niuma/config.toml")).expect("saved config");
        assert!(config_text.contains("server_url = \"https://example.invalid/niuma-server/\""));

        dashboard_task.abort();
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn test_server_url_endpoint_calls_healthz() {
        let home = temp_home("healthz");
        let _home = HomeOverride::new(&home);
        let health_listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("health listener");
        let health_addr = health_listener.local_addr().expect("health addr");
        let health_task = tokio::spawn(async move {
            let app = Router::new().route("/healthz", get(|| async { "ok" }));
            axum::serve(health_listener, app)
                .await
                .expect("health server");
        });
        let (dashboard_url, dashboard_task) = spawn_dashboard().await;

        let response = reqwest::Client::new()
            .post(format!("{dashboard_url}/api/config/server-url/test"))
            .json(&json!({ "server_url": format!("http://{health_addr}") }))
            .send()
            .await
            .expect("dashboard request");

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await.expect("json response");
        assert_eq!(body["reachable"], true);
        assert_eq!(body["server_url"], format!("http://{health_addr}/"));

        dashboard_task.abort();
        health_task.abort();
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn dashboard_serves_html_css_and_js_static_assets() {
        let home = temp_home("static-assets");
        let _home = HomeOverride::new(&home);
        let (dashboard_url, dashboard_task) = spawn_dashboard().await;
        let client = reqwest::Client::new();

        let html = client
            .get(&dashboard_url)
            .send()
            .await
            .expect("html request")
            .text()
            .await
            .expect("html response");
        assert!(html.contains(r#"<link rel="stylesheet" href="/static/dashboard.css" />"#));
        assert!(html.contains(r#"<script src="/static/dashboard.js"></script>"#));
        assert!(!html.contains("const serverInput"));

        let css = client
            .get(format!("{dashboard_url}/static/dashboard.css"))
            .send()
            .await
            .expect("css request");
        assert_eq!(
            css.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("text/css; charset=utf-8")
        );
        assert!(css.text().await.expect("css response").contains(".qr"));

        let js = client
            .get(format!("{dashboard_url}/static/dashboard.js"))
            .send()
            .await
            .expect("js request");
        assert_eq!(
            js.headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok()),
            Some("application/javascript; charset=utf-8")
        );
        assert!(
            js.text()
                .await
                .expect("js response")
                .contains("const serverInput")
        );

        dashboard_task.abort();
        let _ = std::fs::remove_dir_all(home);
    }

    #[tokio::test]
    async fn delete_pairing_revokes_server_before_removing_local_binding() {
        let home = temp_home("delete-pairing");
        let _home = HomeOverride::new(&home);
        bindings::save_binding(PairedDeviceBinding {
            binding_id: "binding-delete".to_string(),
            device_id: "ios-delete".to_string(),
            agent_id: "agent_test".to_string(),
            ios_encryption_public_key: "x25519:test".to_string(),
            paired_at: 1_700_000_001,
        })
        .expect("seed local binding");

        let revoke_listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("revoke listener");
        let revoke_addr = revoke_listener.local_addr().expect("revoke addr");
        let revoke_task = tokio::spawn(async move {
            let app = Router::new().route(
                "/pair-bindings/{binding_id}",
                delete(
                    |Path(binding_id): Path<String>, headers: HeaderMap| async move {
                        let token = headers
                            .get("X-Session-Token")
                            .and_then(|value| value.to_str().ok());
                        let agent_id = headers
                            .get("X-Agent-ID")
                            .and_then(|value| value.to_str().ok());
                        if binding_id != "binding-delete"
                            || token != Some("session-token")
                            || agent_id != Some("agent_test")
                        {
                            return StatusCode::UNAUTHORIZED.into_response();
                        }
                        Json(json!({
                            "binding_id": binding_id,
                            "revoked": true
                        }))
                        .into_response()
                    },
                ),
            );
            axum::serve(revoke_listener, app)
                .await
                .expect("revoke server");
        });
        let state = test_state_with_server(&format!("http://{revoke_addr}"), "session-token");
        let (dashboard_url, dashboard_task) = spawn_dashboard_with_state(state).await;

        let response = reqwest::Client::new()
            .delete(format!("{dashboard_url}/api/pairings/binding-delete"))
            .send()
            .await
            .expect("delete request");

        assert!(response.status().is_success());
        let body: serde_json::Value = response.json().await.expect("json response");
        assert_eq!(body["binding_id"], "binding-delete");
        assert_eq!(body["device_id"], "ios-delete");
        assert_eq!(body["server_revoked"], true);
        assert_eq!(body["local_removed"], true);
        assert!(bindings::list_bindings().expect("list bindings").is_empty());

        dashboard_task.abort();
        revoke_task.abort();
        let _ = std::fs::remove_dir_all(home);
    }

    async fn spawn_dashboard() -> (String, JoinHandle<()>) {
        spawn_dashboard_with_state(test_state()).await
    }

    async fn spawn_dashboard_with_state(state: GatewayState) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("dashboard listener");
        let addr = listener.local_addr().expect("dashboard addr");
        let task = tokio::spawn(async move {
            axum::serve(listener, dashboard_router(state))
                .await
                .expect("dashboard server");
        });
        (format!("http://{addr}"), task)
    }

    fn test_state() -> GatewayState {
        GatewayState {
            config: GatewayConfig {
                server_url: "http://127.0.0.1:8000".to_string(),
                server_url_source: ConfigValueSource::Default,
                device_name: "Test Mac".to_string(),
                dashboard_host: "127.0.0.1".to_string(),
                dashboard_port: 8765,
                heartbeat_seconds: 30,
                pairing_page_only: true,
                open_browser: false,
                disable_codex_plugins: false,
            },
            identity: AgentIdentity {
                agent_id: "agent_test".to_string(),
                device_name: "Test Mac".to_string(),
                os_type: "darwin".to_string(),
                signing_private_key: "ed25519:private".to_string(),
                signing_public_key: "ed25519:public".to_string(),
                signing_key_fingerprint: "fingerprint-signing".to_string(),
                encryption_private_key: "x25519:private".to_string(),
                encryption_public_key: "x25519:public".to_string(),
                encryption_key_fingerprint: "fingerprint-encryption".to_string(),
            },
            codex_runtime: CodexRuntime {
                source: crate::codex::CodexRuntimeSource::PathCodex,
                command: vec!["codex".to_string(), "app-server".to_string()],
            },
            codex_app_server: None,
            server: None,
            session_token: None,
            pairing: Arc::new(RwLock::new(PairingRuntimeState {
                payload: None,
                last_error: Some("pairing payload is not ready".to_string()),
                secrets: Default::default(),
            })),
            agent_channel: Arc::new(RwLock::new(AgentChannelStatus::default())),
            started_at: 1_700_000_000,
        }
    }

    fn test_state_with_server(server_url: &str, session_token: &str) -> GatewayState {
        let mut state = test_state();
        state.config.pairing_page_only = false;
        state.server = Some(NiumaServerClient::new(server_url).expect("server URL"));
        state.session_token = Some(Arc::new(RwLock::new(session_token.to_string())));
        state
    }

    fn temp_home(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("niuma-dashboard-{label}-{}", Uuid::new_v4()))
    }

    fn home_guard() -> MutexGuard<'static, ()> {
        HOME_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    struct HomeOverride {
        _guard: MutexGuard<'static, ()>,
        old_home: Option<OsString>,
    }

    impl HomeOverride {
        fn new(path: &PathBuf) -> Self {
            let guard = home_guard();
            let old_home = std::env::var_os("HOME");
            // These dashboard interface tests need the production ~/.niuma path
            // resolver while keeping writes inside a per-test temporary directory.
            unsafe {
                std::env::set_var("HOME", path);
            }
            Self {
                _guard: guard,
                old_home,
            }
        }
    }

    impl Drop for HomeOverride {
        fn drop(&mut self) {
            unsafe {
                match &self.old_home {
                    Some(value) => std::env::set_var("HOME", value),
                    None => std::env::remove_var("HOME"),
                }
            }
        }
    }
}
