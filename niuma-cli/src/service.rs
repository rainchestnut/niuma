//! macOS launchd service management for the gateway.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::cli::{ResetArgs, ServiceInstallArgs};
use crate::config::StatusConfig;
use crate::paths;
use crate::status;

/// Install the LaunchAgent plist and optionally start it.
pub async fn install(args: ServiceInstallArgs) -> Result<()> {
    paths::ensure_state_dirs()?;
    let plist_path = paths::launch_agent_path()?;
    if let Some(parent) = plist_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let binary = std::env::current_exe().context("failed to resolve current niuma binary")?;
    let plist = render_plist(&binary, args.no_open)?;
    std::fs::write(&plist_path, plist)
        .with_context(|| format!("failed to write {}", plist_path.display()))?;
    println!("installed {}", plist_path.display());
    if args.start {
        start().await?;
    }
    Ok(())
}

/// Start the installed LaunchAgent.
pub async fn start() -> Result<()> {
    let plist_path = paths::launch_agent_path()?;
    if !plist_path.exists() {
        anyhow::bail!(
            "{} is not installed; run `niuma service install` first",
            plist_path.display()
        );
    }
    if launchd_loaded()? {
        println!("{} is already loaded", paths::LAUNCH_AGENT_LABEL);
        return Ok(());
    }
    ensure_gateway_port_free().await?;
    run_launchctl([
        "bootstrap".to_string(),
        launchd_domain()?,
        plist_path.display().to_string(),
    ])?;
    println!("started {}", paths::LAUNCH_AGENT_LABEL);
    Ok(())
}

/// Stop the installed LaunchAgent if it is loaded.
pub async fn stop() -> Result<()> {
    if launchd_loaded()? {
        run_launchctl(["bootout".to_string(), launchd_service_target()?])?;
        wait_for_launchd_unloaded().await?;
        wait_for_gateway_port_free().await?;
        println!("stopped {}", paths::LAUNCH_AGENT_LABEL);
    }
    Ok(())
}

/// Restart the LaunchAgent and wait for the old gateway process to leave the port.
pub async fn restart() -> Result<()> {
    stop().await?;
    start().await
}

/// Stop and remove the LaunchAgent plist.
pub async fn uninstall() -> Result<()> {
    stop().await?;
    let plist_path = paths::launch_agent_path()?;
    if plist_path.exists() {
        std::fs::remove_file(&plist_path)
            .with_context(|| format!("failed to remove {}", plist_path.display()))?;
        println!("removed {}", plist_path.display());
    }
    Ok(())
}

pub async fn print_status() -> Result<()> {
    let loaded = launchd_loaded()?;
    println!("launchd_label={}", paths::LAUNCH_AGENT_LABEL);
    println!("launchd_loaded={loaded}");
    if loaded {
        if let Ok(output) = launchctl_output(["print".to_string(), launchd_service_target()?]) {
            for line in output.lines().take(12) {
                println!("launchd: {line}");
            }
        }
    }
    let config = StatusConfig::from_args(&crate::cli::StatusArgs {
        dashboard_host: None,
        dashboard_port: None,
    })?;
    match status::fetch_gateway_status(&config.dashboard_host, config.dashboard_port).await {
        Ok(value) => println!("gateway_status={}", serde_json::to_string_pretty(&value)?),
        Err(err) => println!("gateway_status_error={err:#}"),
    }
    Ok(())
}

/// Destructively remove service installation and local state.
pub async fn reset(args: ResetArgs) -> Result<()> {
    if !args.yes {
        anyhow::bail!("refusing to reset without --yes");
    }
    uninstall().await?;
    let root = paths::state_root()?;
    if root.exists() {
        std::fs::remove_dir_all(&root)
            .with_context(|| format!("failed to remove {}", root.display()))?;
        println!("removed {}", root.display());
    }
    Ok(())
}

fn render_plist(binary: &Path, no_open: bool) -> Result<String> {
    let logs_dir = paths::logs_dir()?;
    std::fs::create_dir_all(&logs_dir)
        .with_context(|| format!("failed to create {}", logs_dir.display()))?;
    let mut arguments = vec![binary.display().to_string(), "gateway".to_string()];
    if no_open {
        arguments.push("--no-open".to_string());
    }
    let args_xml = arguments
        .iter()
        .map(|arg| format!("        <string>{}</string>", xml_escape(arg)))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
{args_xml}
    </array>
    <key>KeepAlive</key>
    <true/>
    <key>RunAtLoad</key>
    <false/>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>
"#,
        label = paths::LAUNCH_AGENT_LABEL,
        stdout = xml_escape(&logs_dir.join("gateway.out.log").display().to_string()),
        stderr = xml_escape(&logs_dir.join("gateway.err.log").display().to_string()),
    ))
}

async fn ensure_gateway_port_free() -> Result<()> {
    let config = StatusConfig::from_args(&crate::cli::StatusArgs {
        dashboard_host: None,
        dashboard_port: None,
    })?;
    match std::net::TcpListener::bind((config.dashboard_host.as_str(), config.dashboard_port)) {
        Ok(listener) => {
            drop(listener);
            Ok(())
        }
        Err(err) => {
            let owner =
                port_owner(config.dashboard_port).unwrap_or_else(|| "unknown pid".to_string());
            anyhow::bail!(
                "gateway port {} is already in use by {}; stop it before starting service: {}",
                config.dashboard_port,
                owner,
                err
            )
        }
    }
}

fn port_owner(port: u16) -> Option<String> {
    Command::new("lsof")
        .args(["-nP", &format!("-iTCP:{port}"), "-sTCP:LISTEN"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|text| text.lines().nth(1).map(|line| line.to_string()))
}

fn launchd_loaded() -> Result<bool> {
    Ok(launchctl_output(["print".to_string(), launchd_service_target()?]).is_ok())
}

fn run_launchctl<I>(args: I) -> Result<()>
where
    I: IntoIterator<Item = String>,
{
    let output = Command::new("launchctl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to run launchctl")?;
    if output.status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "launchctl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

fn launchctl_output<I>(args: I) -> Result<String>
where
    I: IntoIterator<Item = String>,
{
    let output = Command::new("launchctl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .context("failed to run launchctl")?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        anyhow::bail!(
            "launchctl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}

fn launchd_domain() -> Result<String> {
    let output = Command::new("id")
        .arg("-u")
        .output()
        .context("failed to run id -u")?;
    if !output.status.success() {
        anyhow::bail!("id -u failed");
    }
    let uid = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(format!("gui/{uid}"))
}

/// Return the launchctl service-target form required by commands such as bootout.
fn launchd_service_target() -> Result<String> {
    Ok(format!(
        "{}/{}",
        launchd_domain()?,
        paths::LAUNCH_AGENT_LABEL
    ))
}

/// Wait until launchd no longer reports the LaunchAgent as loaded.
async fn wait_for_launchd_unloaded() -> Result<()> {
    let started = Instant::now();
    while launchd_loaded()? {
        if started.elapsed() >= Duration::from_secs(10) {
            anyhow::bail!(
                "timed out waiting for {} to unload",
                paths::LAUNCH_AGENT_LABEL
            );
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

/// Wait until the dashboard port is free after launchd stops the old gateway.
async fn wait_for_gateway_port_free() -> Result<()> {
    let started = Instant::now();
    loop {
        match ensure_gateway_port_free().await {
            Ok(()) => return Ok(()),
            Err(_) if started.elapsed() < Duration::from_secs(10) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
