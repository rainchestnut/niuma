//! Command-line shape for the installed `niuma` binary.

use clap::{Args, Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "niuma", version, about = "Niuma desktop gateway")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Run the foreground desktop gateway runtime.
    Gateway(GatewayArgs),
    /// Manage the macOS launchd service for the gateway.
    Service {
        #[command(subcommand)]
        command: ServiceCommands,
    },
    /// Read the current gateway HTTP status endpoint.
    Status(StatusArgs),
    /// Stop service state and delete ~/.niuma. Requires --yes.
    Reset(ResetArgs),
}

#[derive(Debug, Args, Clone)]
pub struct GatewayArgs {
    /// Niuma server base URL.
    #[arg(long)]
    pub server_url: Option<String>,
    /// Local loopback host for the gateway control page.
    #[arg(long)]
    pub dashboard_host: Option<String>,
    /// Local loopback port for the gateway control page.
    #[arg(long)]
    pub dashboard_port: Option<u16>,
    /// Desktop agent display name.
    #[arg(long)]
    pub device_name: Option<String>,
    /// Start only the local pairing page for diagnostics.
    #[arg(long)]
    pub pairing_page_only: bool,
    /// Keep the browser closed after gateway startup.
    #[arg(long)]
    pub no_open: bool,
    /// Add `--disable plugins` to the spawned Codex app-server command.
    #[arg(long)]
    pub disable_codex_plugins: bool,
}

#[derive(Debug, Subcommand)]
pub enum ServiceCommands {
    /// Install the LaunchAgent plist.
    Install(ServiceInstallArgs),
    /// Start the installed LaunchAgent.
    Start,
    /// Stop the installed LaunchAgent.
    Stop,
    /// Restart the installed LaunchAgent.
    Restart,
    /// Stop and remove the LaunchAgent plist.
    Uninstall,
    /// Print launchd plus gateway status.
    Status,
}

#[derive(Debug, Args, Clone)]
pub struct ServiceInstallArgs {
    /// Install a gateway service that does not auto-open the pairing page.
    #[arg(long)]
    pub no_open: bool,
    /// Start the service immediately after writing the plist.
    #[arg(long)]
    pub start: bool,
}

#[derive(Debug, Args, Clone)]
pub struct StatusArgs {
    /// Local gateway host to query.
    #[arg(long)]
    pub dashboard_host: Option<String>,
    /// Local gateway port to query.
    #[arg(long)]
    pub dashboard_port: Option<u16>,
}

#[derive(Debug, Args, Clone)]
pub struct ResetArgs {
    /// Confirm destructive deletion of local Niuma state.
    #[arg(long)]
    pub yes: bool,
}
