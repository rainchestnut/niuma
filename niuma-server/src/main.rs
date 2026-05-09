//! Niuma payload-blind control-plane server.
//!
//! The server owns device auth, pairing, WebSocket routing, and temporary
//! transfer relay state. Codex business payloads remain opaque to this process.

mod apns;
mod config;
mod crypto;
mod db;
mod error;
mod hub;
mod logging;
mod models;
mod routes;
mod transfer;

use std::{net::SocketAddr, sync::Arc};

use anyhow::Context;
use axum::Router;
use sqlx::postgres::PgPoolOptions;
use tokio::signal;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;

use crate::{
    apns::ApnsPushService, config::Settings, db::init_schema, hub::ConnectionHub,
    transfer::TransferStore,
};

/// Shared runtime state injected into HTTP and WebSocket handlers.
#[derive(Clone)]
pub struct AppState {
    pub settings: Settings,
    pub pool: sqlx::PgPool,
    pub hub: Arc<ConnectionHub>,
    pub transfers: Arc<TransferStore>,
    pub push: Arc<ApnsPushService>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let settings = Settings::load().context("failed to load server settings")?;
    let _log_guard = logging::init(&settings)?;

    let pool = PgPoolOptions::new()
        .max_connections(settings.database_pool_size)
        .acquire_timeout(settings.database_connect_timeout)
        .connect(&settings.database_url)
        .await
        .context("failed to connect PostgreSQL")?;
    init_schema(&pool)
        .await
        .context("failed to initialize schema")?;

    let state = AppState {
        transfers: Arc::new(TransferStore::new(&settings)?),
        push: Arc::new(ApnsPushService::new(&settings)?),
        settings: settings.clone(),
        pool,
        hub: Arc::new(ConnectionHub::default()),
    };
    state.transfers.cleanup_expired().await;
    spawn_cleanup_loop(state.clone());

    let app = app_router(state);
    let addr = SocketAddr::new(settings.host.parse()?, settings.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("niuma-server listening on http://{addr}");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn app_router(state: AppState) -> Router {
    routes::router(state.settings.transfer_max_encrypted_bytes)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

fn spawn_cleanup_loop(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            interval.tick().await;
            if let Err(error) = db::cleanup_expired(&state.pool).await {
                tracing::warn!("database cleanup failed: {error:#}");
            }
            state.transfers.cleanup_expired().await;
        }
    });
}

async fn shutdown_signal() {
    let _ = signal::ctrl_c().await;
}
