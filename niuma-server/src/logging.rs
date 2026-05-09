//! Structured logging setup for the control-plane server.
//!
//! The server keeps stdout logs for systemd/journalctl and writes JSONL files
//! for cross-process request tracing. Payload content is intentionally logged
//! only through explicit safe fields at call sites.

use std::{fs, path::Path, time::Duration};

use anyhow::Result;
use tracing_appender::{non_blocking::WorkerGuard, rolling::Rotation};
use tracing_subscriber::{EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt};

use crate::config::Settings;

const DEFAULT_RETENTION_DAYS: u64 = 14;

/// Install stdout and rolling JSONL file subscribers.
pub fn init(settings: &Settings) -> Result<WorkerGuard> {
    fs::create_dir_all(&settings.log_dir)?;
    cleanup_old_logs(&settings.log_dir, settings.log_retention_days);

    let file_appender = tracing_appender::rolling::Builder::new()
        .rotation(Rotation::DAILY)
        .filename_prefix("server")
        .filename_suffix("jsonl")
        .build(&settings.log_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let filter = env_filter(&settings.log_level);

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
        log_dir = %settings.log_dir.display(),
        retention_days = settings.log_retention_days,
        "server_logging_initialized"
    );
    Ok(guard)
}

fn cleanup_old_logs(logs_dir: &Path, days: u64) {
    let max_age = Duration::from_secs(days.max(1).saturating_mul(24 * 60 * 60));
    let Ok(entries) = fs::read_dir(logs_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        if !name.starts_with("server.") || !name.ends_with(".jsonl") {
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

fn env_filter(default: &str) -> EnvFilter {
    std::env::var("NIUMA_LOG_LEVEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .and_then(|value| EnvFilter::try_new(value).ok())
        .or_else(|| EnvFilter::try_from_default_env().ok())
        .unwrap_or_else(|| EnvFilter::new(default))
}

pub fn default_retention_days() -> u64 {
    DEFAULT_RETENTION_DAYS
}
