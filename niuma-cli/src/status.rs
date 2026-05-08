//! Status commands for foreground or service-managed gateways.

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::Value;

use crate::cli::StatusArgs;
use crate::config::StatusConfig;

/// Query the local gateway status endpoint and print pretty JSON.
pub async fn print_gateway_status(args: StatusArgs) -> Result<()> {
    let config = StatusConfig::from_args(&args)?;
    let status = fetch_gateway_status(&config.dashboard_host, config.dashboard_port).await?;
    println!("{}", serde_json::to_string_pretty(&status)?);
    Ok(())
}

pub async fn fetch_gateway_status(host: &str, port: u16) -> Result<Value> {
    let url = format!("http://{host}:{port}/api/status");
    let value: Value = Client::new()
        .get(&url)
        .send()
        .await
        .with_context(|| format!("gateway HTTP is not reachable at {url}"))?
        .error_for_status()
        .with_context(|| format!("gateway status failed at {url}"))?
        .json()
        .await
        .with_context(|| format!("gateway returned invalid JSON at {url}"))?;
    if value.get("codex_runtime").is_none() || value.get("state_root").is_none() {
        anyhow::bail!("{url} is reachable, but it is not a niuma-cli gateway status endpoint");
    }
    Ok(value)
}
