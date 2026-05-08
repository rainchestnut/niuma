//! Foreground gateway runtime and loopback HTTP control surface.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use qrcode::QrCode;
use qrcode::render::svg;
use serde::Serialize;
use tokio::net::TcpListener;
use tokio::sync::RwLock;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

use crate::cli::GatewayArgs;
use crate::codex::{self, CodexRuntime};
use crate::codex_app_server::CodexAppServerClient;
use crate::config::GatewayConfig;
use crate::identity::AgentIdentity;
use crate::pairing::{self, PairingMaterial, PairingPayload, PairingRuntimeState};
use crate::paths;
use crate::realtime;
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
    server_url: String,
    server_connected: bool,
    authenticated: bool,
    pairing_payload_ready: bool,
    pair_token_expires_at: Option<i64>,
    codex_runtime: CodexRuntime,
    dashboard_url: String,
    open_browser: bool,
    started_at: i64,
    last_pairing_error: Option<String>,
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
        );
    }

    let dashboard_url = dashboard_url(&config);
    let app = Router::new()
        .route("/", get(index))
        .route("/api/status", get(api_status))
        .route("/api/pairing/payload", get(pairing_payload))
        .route("/api/pairing/qr.svg", get(pairing_qr_svg))
        .route("/api/pairing/refresh", post(refresh_pairing_payload))
        .with_state(state);

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

async fn index(State(state): State<GatewayState>) -> Html<String> {
    Html(render_index(&dashboard_url(&state.config)))
}

async fn api_status(State(state): State<GatewayState>) -> Json<GatewayStatus> {
    Json(build_status(&state).await)
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
        server_url: state
            .server
            .as_ref()
            .map(|server| server.base_url().to_string())
            .unwrap_or_else(|| state.config.server_url.clone()),
        server_connected: state.server.is_some(),
        authenticated: state.session_token.is_some(),
        pairing_payload_ready: pairing.payload.is_some(),
        pair_token_expires_at: pairing.payload.as_ref().map(|payload| payload.expires_at),
        codex_runtime: state.codex_runtime.clone(),
        dashboard_url: dashboard_url(&state.config),
        open_browser: state.config.open_browser,
        started_at: state.started_at,
        last_pairing_error: pairing.last_error.clone(),
    }
}

fn render_index(dashboard_url: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Niuma Gateway</title>
  <style>
    :root {{ color-scheme: light dark; font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif; }}
    body {{ margin: 0; min-height: 100vh; display: grid; place-items: center; background: Canvas; color: CanvasText; }}
    main {{ width: min(880px, calc(100vw - 32px)); display: grid; gap: 20px; grid-template-columns: minmax(260px, 360px) 1fr; align-items: start; }}
    section {{ border: 1px solid color-mix(in srgb, CanvasText 14%, transparent); border-radius: 8px; padding: 20px; }}
    h1 {{ margin: 0 0 8px; font-size: 24px; }}
    h2 {{ margin: 0 0 12px; font-size: 16px; }}
    img {{ width: 100%; aspect-ratio: 1; background: white; border-radius: 8px; }}
    pre {{ overflow: auto; white-space: pre-wrap; word-break: break-word; font-size: 12px; }}
    button {{ height: 36px; padding: 0 14px; border-radius: 6px; border: 0; background: CanvasText; color: Canvas; cursor: pointer; }}
    .muted {{ opacity: .68; }}
    @media (max-width: 720px) {{ main {{ grid-template-columns: 1fr; }} }}
  </style>
</head>
<body>
  <main>
    <section>
      <h1>Niuma Gateway</h1>
      <p class="muted">{dashboard_url}</p>
      <img id="qr" alt="Niuma pairing QR" src="/api/pairing/qr.svg" />
      <p><button id="refresh">刷新二维码</button></p>
    </section>
    <section>
      <h2>状态</h2>
      <pre id="status">loading...</pre>
      <h2>配对 Payload</h2>
      <pre id="payload">loading...</pre>
    </section>
  </main>
  <script>
    async function readJSON(path) {{
      const response = await fetch(path, {{ cache: "no-store" }});
      const text = await response.text();
      try {{ return JSON.stringify(JSON.parse(text), null, 2); }}
      catch {{ return text; }}
    }}
    async function refresh(force = false) {{
      if (force) await fetch("/api/pairing/refresh", {{ method: "POST" }});
      document.getElementById("status").textContent = await readJSON("/api/status");
      document.getElementById("payload").textContent = await readJSON("/api/pairing/payload");
      document.getElementById("qr").src = "/api/pairing/qr.svg?ts=" + Date.now();
    }}
    document.getElementById("refresh").addEventListener("click", () => refresh(true));
    refresh(false);
    setInterval(() => refresh(false), 5000);
  </script>
</body>
</html>"#
    )
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
