mod cli;
mod config;
mod db;
mod fevxml;
mod importers;
mod load;
mod models;
mod nt_store;
mod repositories;
mod services;
mod timing;

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands, NtCommands};

fn main() -> ExitCode {
    init_tracing();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Nt {
            command: NtCommands::Import { file },
        } => nt_store::service::run(&file),
        Commands::Load => load::service::run(),
    }
}

/// Logs to stderr; level from `RUST_LOG`, `warn` by default.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
