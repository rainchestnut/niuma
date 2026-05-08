//! Niuma desktop gateway CLI.
//!
//! This binary replaces the Python plugin runtime with a user-installed
//! command that can run in the foreground or through macOS launchd.

mod bindings;
mod cli;
mod codex;
mod codex_app_server;
mod config;
mod crypto;
mod diff_summary;
mod gateway;
mod identity;
mod metadata;
mod pairing;
mod paths;
mod realtime;
mod server;
mod service;
mod status;
mod tasks;
mod thread_status;
mod transfers;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, ServiceCommands};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "niuma=info,tower_http=warn".into()),
        )
        .init();

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
