//! `facturacion config check [clave...]`.

use anyhow::Result;
use clap::{Args, Subcommand};

use super::Settings;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Valida la configuración y lista qué claves están definidas (nunca sus valores)
    Check(CheckArgs),
}

#[derive(Debug, Args)]
pub struct CheckArgs {
    /// Claves requeridas (p. ej. database.url); falla si alguna falta
    #[arg(value_name = "CLAVE")]
    pub keys: Vec<String>,
}

pub fn run(settings: &Settings, command: ConfigCommand) -> Result<()> {
    match command {
        ConfigCommand::Check(args) => check(settings, &args.keys),
    }
}

/// Prints the table of keys and fails when a requested key is missing.
fn check(settings: &Settings, required: &[String]) -> Result<()> {
    let file = settings
        .file()
        .map_or_else(|| "(ninguno)".to_owned(), |p| p.display().to_string());
    println!("archivo de configuración: {file}\n");
    let rows = settings.describe();
    let key_width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0);
    let env_width = rows.iter().map(|r| r.env.len()).max().unwrap_or(0);
    println!(
        "{:<key_width$}  {:<env_width$}  {:<6}  TIPO",
        "CLAVE", "VARIABLE", "ESTADO"
    );
    for row in &rows {
        let status = if row.set { "set" } else { "unset" };
        let kind = if row.secret { "secreto" } else { "" };
        println!(
            "{:<key_width$}  {:<env_width$}  {status:<6}  {kind}",
            row.key, row.env
        );
    }
    if !required.is_empty() {
        let keys: Vec<&str> = required.iter().map(String::as_str).collect();
        settings.require(&keys)?;
        println!("\nlas {} claves requeridas están definidas", keys.len());
    }
    Ok(())
}
