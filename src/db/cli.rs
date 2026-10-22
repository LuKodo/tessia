//! `facturacion db migrate`.

use anyhow::Result;
use clap::Subcommand;

use super::migrate;
use crate::config::Settings;

#[derive(Debug, Subcommand)]
pub enum DbCommand {
    /// Aplica las migraciones pendientes (seguro de repetir y de ejecutar desde varias máquinas)
    Migrate,
}

pub fn run(settings: &Settings, command: DbCommand) -> Result<()> {
    match command {
        DbCommand::Migrate => {
            let url = settings.require_one("database.url")?;
            let applied = migrate::migrate_url(url)?;
            if applied.is_empty() {
                println!("la base de datos ya está al día");
            } else {
                println!("migraciones aplicadas: {}", applied.len());
                for version in applied {
                    println!("  {version}");
                }
            }
            Ok(())
        }
    }
}
