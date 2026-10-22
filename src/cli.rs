use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "facturacion", version, about)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Administra la Nota Técnica global
    Nt {
        #[command(subcommand)]
        command: NtCommands,
    },
    /// Crea un cargue con una copia de la Nota Técnica vigente
    Load,
}

#[derive(Debug, Subcommand)]
pub enum NtCommands {
    /// Importa una Nota Técnica (.xlsx) y archiva la versión anterior
    Import {
        /// Ruta de la Nota Técnica (.xlsx)
        #[arg(value_name = "ARCHIVO")]
        file: PathBuf,
    },
}
