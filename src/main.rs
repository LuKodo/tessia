//! `facturacion`: thin entry point. All logic lives in the `tessia` library.

use std::process::ExitCode;

use anyhow::Result;
use clap::Parser;
use tessia::cli::{Cli, Commands, ConvenioCommand};
use tessia::config::{self, Settings};
use tessia::context::AppContext;
use tessia::phases::{
    conciliar, convenio_descargar, convenio_preparar, cuv, derechos, fev, proceso, radicacion,
    rips, soporte, thefactory,
};
use tessia::{db, shutdown};
use tracing_subscriber::EnvFilter;

/// Exit status of a usage error (unknown flag, missing argument).
const EXIT_USAGE: u8 = 2;

fn main() -> ExitCode {
    init_tracing();
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(err) => {
            let usage_error = err.use_stderr();
            let _ = err.print();
            return if usage_error {
                ExitCode::from(EXIT_USAGE)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    shutdown::install();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    let settings = Settings::load(cli.config.as_deref())?;
    match cli.command {
        // These two need no connection to the object store or Redis.
        Commands::Config { command } => config::cli::run(&settings, command),
        Commands::Db { command } => db::cli::run(&settings, command),
        command => run_phase_command(AppContext::from_settings(settings)?, command),
    }
}

fn run_phase_command(ctx: AppContext, command: Commands) -> Result<()> {
    match command {
        Commands::Convenio {
            command: ConvenioCommand::Descargar(args),
        } => convenio_descargar::run(&ctx, args),
        Commands::Convenio {
            command: ConvenioCommand::Preparar(args),
        } => convenio_preparar::run(&ctx, args),
        Commands::Proceso(args) => proceso::run(&ctx, args),
        Commands::Diario(args) => proceso::diario::run(&ctx, args),
        Commands::Conciliar(args) => conciliar::run(&ctx, args),
        Commands::Thefactory(args) => thefactory::run(&ctx, args),
        Commands::Derechos(args) => derechos::run(&ctx, args),
        Commands::Fev(args) => fev::run(&ctx, args),
        Commands::Rips(args) => rips::run(&ctx, args),
        Commands::Cuv(args) => cuv::run(&ctx, args),
        Commands::Soporte(args) => soporte::run(&ctx, args),
        Commands::Radicacion(args) => radicacion::run(&ctx, args),
        Commands::Config { .. } | Commands::Db { .. } => unreachable!("handled in run"),
    }
}

/// Diagnostics go to stderr; the level comes from `RUST_LOG` (`warn` by
/// default). User-facing output is printed with `println!`, never traced.
fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}
