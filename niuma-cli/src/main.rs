//! Niuma desktop gateway CLI.
//!
//! This binary replaces the Python plugin runtime with a user-installed
//! command that can run in the foreground or through macOS launchd.

mod bindings;
mod cli;
mod codex;
mod codex_app_server;
mod codex_projection;
mod config;
mod crypto;
mod diff_summary;
mod gateway;
mod identity;
mod logging;
mod metadata;
mod pairing;
mod paths;
mod process_summary;
mod realtime;
mod server;
mod service;
mod status;
mod tasks;
mod thread_status;
mod transfers;

use clap::Parser;
use cli::{Cli, Commands, ServiceCommands};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _log_guard = logging::init()?;
    let cli = Cli::parse();
    match cli.command {
        Commands::Gateway(args) => gateway::run(args).await,
        Commands::Status(args) => status::print_gateway_status(args).await,
        Commands::Reset(args) => service::reset(args).await,
        Commands::Service { command } => match command {
            ServiceCommands::Install(args) => service::install(args).await,
            ServiceCommands::Start => service::start().await,
            ServiceCommands::Stop => service::stop().await,
            ServiceCommands::Restart => service::restart().await,
            ServiceCommands::Uninstall => service::uninstall().await,
            ServiceCommands::Status => service::print_status().await,
        },
    }
}
