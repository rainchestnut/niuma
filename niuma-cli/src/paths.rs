//! Filesystem locations owned by the installed Niuma CLI.

use std::path::PathBuf;

use anyhow::{Context, Result};

pub const LAUNCH_AGENT_LABEL: &str = "com.niuma.gateway";

/// Return the fixed user state root for the Rust gateway.
pub fn state_root() -> Result<PathBuf> {
    home_dir().map(|home| home.join(".niuma"))
}

pub fn config_path() -> Result<PathBuf> {
    Ok(state_root()?.join("config.toml"))
}

pub fn identity_dir() -> Result<PathBuf> {
    Ok(state_root()?.join("identity"))
}

pub fn transfers_dir() -> Result<PathBuf> {
    Ok(state_root()?.join("transfers"))
}

pub fn logs_dir() -> Result<PathBuf> {
    Ok(state_root()?.join("logs"))
}

pub fn runtime_dir() -> Result<PathBuf> {
    Ok(state_root()?.join("runtime"))
}

pub fn launch_agent_path() -> Result<PathBuf> {
    Ok(home_dir()?
        .join("Library")
        .join("LaunchAgents")
        .join(format!("{LAUNCH_AGENT_LABEL}.plist")))
}

/// Create all top-level runtime directories expected by the gateway.
pub fn ensure_state_dirs() -> Result<()> {
    for dir in [
        state_root()?,
        identity_dir()?,
        transfers_dir()?,
        logs_dir()?,
        runtime_dir()?,
    ] {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create {}", dir.display()))?;
    }
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set; cannot resolve ~/.niuma")
}
