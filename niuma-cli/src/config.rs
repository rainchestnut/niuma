//! Configuration loading with CLI, environment, and ~/.niuma/config.toml layers.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::cli::{GatewayArgs, StatusArgs};
use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub server_url: Option<String>,
    pub device_name: Option<String>,
    pub dashboard_host: Option<String>,
    pub dashboard_port: Option<u16>,
    pub heartbeat_seconds: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct GatewayConfig {
    pub server_url: String,
    pub device_name: String,
    pub dashboard_host: String,
    pub dashboard_port: u16,
    pub heartbeat_seconds: u64,
    pub pairing_page_only: bool,
    pub open_browser: bool,
    pub disable_codex_plugins: bool,
}

#[derive(Debug, Clone)]
pub struct StatusConfig {
    pub dashboard_host: String,
    pub dashboard_port: u16,
}

impl GatewayConfig {
    /// Resolve gateway runtime configuration from all supported sources.
    pub fn from_args(args: &GatewayArgs) -> Result<Self> {
        let file = read_config_file()?;
        Ok(Self {
            server_url: first_string(
                args.server_url.clone(),
                "NIUMA_SERVER_URL",
                file.as_ref().and_then(|value| value.server_url.clone()),
                "http://127.0.0.1:8000",
            ),
            device_name: first_string(
                args.device_name.clone(),
                "NIUMA_DEVICE_NAME",
                file.as_ref().and_then(|value| value.device_name.clone()),
                default_device_name().as_str(),
            ),
            dashboard_host: first_string(
                args.dashboard_host.clone(),
                "NIUMA_DASHBOARD_HOST",
                file.as_ref().and_then(|value| value.dashboard_host.clone()),
                "127.0.0.1",
            ),
            dashboard_port: args
                .dashboard_port
                .or_else(|| env_u16("NIUMA_DASHBOARD_PORT"))
                .or_else(|| file.as_ref().and_then(|value| value.dashboard_port))
                .unwrap_or(8765),
            heartbeat_seconds: env_u64("NIUMA_HEARTBEAT_SECONDS")
                .or_else(|| file.as_ref().and_then(|value| value.heartbeat_seconds))
                .unwrap_or(30),
            pairing_page_only: args.pairing_page_only,
            open_browser: !args.no_open,
            disable_codex_plugins: args.disable_codex_plugins,
        })
    }
}

impl StatusConfig {
    pub fn from_args(args: &StatusArgs) -> Result<Self> {
        let file = read_config_file()?;
        Ok(Self {
            dashboard_host: first_string(
                args.dashboard_host.clone(),
                "NIUMA_DASHBOARD_HOST",
                file.as_ref().and_then(|value| value.dashboard_host.clone()),
                "127.0.0.1",
            ),
            dashboard_port: args
                .dashboard_port
                .or_else(|| env_u16("NIUMA_DASHBOARD_PORT"))
                .or_else(|| file.as_ref().and_then(|value| value.dashboard_port))
                .unwrap_or(8765),
        })
    }
}

pub fn read_config_file() -> Result<Option<ConfigFile>> {
    let path = paths::config_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let file =
        toml::from_str(&text).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(file))
}

fn first_string(cli: Option<String>, env_key: &str, file: Option<String>, default: &str) -> String {
    cli.or_else(|| std::env::var(env_key).ok())
        .or(file)
        .unwrap_or_else(|| default.to_string())
}

fn env_u16(key: &str) -> Option<u16> {
    std::env::var(key).ok().and_then(|value| value.parse().ok())
}

fn env_u64(key: &str) -> Option<u64> {
    std::env::var(key).ok().and_then(|value| value.parse().ok())
}

fn default_device_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| {
            std::process::Command::new("scutil")
                .args(["--get", "ComputerName"])
                .output()
                .ok()
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "Niuma Desktop".to_string())
}
