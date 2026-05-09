//! Structured logging setup for the desktop gateway.
//!
//! Human-readable logs still go to stdout/stderr for foreground use and
//! launchd capture. JSONL logs are written under `~/.niuma/logs` so one
//! request can be traced across the gateway and server.

use std::{fs, path::Path, time::Duration};

use anyhow::Result;
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::paths;

const DEFAULT_RETENTION_DAYS: u64 = 14;

/// Install stdout and rolling JSONL file subscribers.
pub fn init() -> Result<WorkerGuard> {
    let logs_dir = paths::logs_dir()?;
    fs::create_dir_all(&logs_dir)?;
    let retention_days = retention_days();
    cleanup_old_logs(&logs_dir, retention_days);

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("gateway")
        .filename_suffix("jsonl")
        .build(&logs_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let filter = env_filter("niuma=info,tower_http=warn");

    let stdout_layer = fmt::layer().with_target(true);
    let file_layer = fmt::layer()
        .json()
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(filter)
        .with(stdout_layer)
        .with(file_layer)
        .init();

    tracing::info!(
        log_dir = %logs_dir.display(),
        retention_days,
        "gateway_logging_initialized"
    );
    Ok(guard)
}

fn retention_days() -> u64 {
    std::env::var("NIUMA_LOG_RETENTION_DAYS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

fn env_filter(default: &str) -> EnvFilter {
    std::env::var("NIUMA_LOG_LEVEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| EnvFilter::try_new(value).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new(default))
}

fn cleanup_old_logs(logs_dir: &Path, days: u64) {
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return;
    };
    let max_age = Duration::from_secs(days.max(1).saturating_mul(24 * 60 * 60));
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("gateway.") || !name.ends_with(".jsonl") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let Ok(modified) = metadata.modified() else {
            continue;
        };
        if modified.elapsed().is_ok_and(|age| age > max_age) {
            let _ = fs::remove_file(path);
        }
    }
}
