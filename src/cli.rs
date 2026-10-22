//! Top-level command line. Each phase owns its `Args` in `phases/<name>/cli.rs`.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::config::cli::ConfigCommand;
use crate::db::cli::DbCommand;
use crate::phases::{
    conciliar, convenio_descargar, convenio_preparar, cuv, derechos, fev, proceso, radicacion,
    rips, soporte, thefactory,
};

#[derive(Debug, Parser)]
#[command(
    name = "facturacion",
    version,
    about = "Automatización de facturación: convenio, fases por factura y radicación en Mutual Ser"
)]
pub struct Cli {
    /// Archivo CLAVE=VALOR de configuración (también TESSIA_CONFIG); las variables TESSIA_* lo sobrescriben
    #[arg(long, global = true, value_name = "ARCHIVO")]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Convenio: descarga de Medicar y preparación de facturas
    Convenio {
        #[command(subcommand)]
        command: ConvenioCommand,
    },
    /// Proceso completo de un cargue
    Proceso(proceso::Args),
    // The description (about) comes from `proceso::diario::Args`.
    Diario(proceso::diario::Args),
    // The description (about) comes from `conciliar::Args`.
    Conciliar(conciliar::Args),
    /// The Factory: XML, PDF y soporte CPScan
    Thefactory(thefactory::Args),
    /// Derechos de afiliados
    Derechos(derechos::Args),
    /// XML de factura electrónica
    Fev(fev::Args),
    /// RIPS
    Rips(rips::Args),
    /// Código Único de Validación
    Cuv(cuv::Args),
    /// Soporte CRC
    Soporte(soporte::Args),
    /// Radicación en Mutual Ser
    Radicacion(radicacion::Args),
    /// Configuración
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Base de datos
    Db {
        #[command(subcommand)]
        command: DbCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ConvenioCommand {
    /// Descarga el CSV de convenio de Medicar
    Descargar(convenio_descargar::Args),
    /// Prepara las facturas de un cargue
    Preparar(convenio_preparar::Args),
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_command_tree_is_consistent() {
        Cli::command().debug_assert();
    }

    #[test]
    fn every_go_command_parses() {
        let commands: &[&[&str]] = &[
            &["convenio", "descargar"],
            &["convenio", "preparar", "--cargue", "C1"],
            &[
                "proceso",
                "run",
                "--cargue",
                "C1",
                "--invoices",
                "FEV1,FEV2",
                "--workers",
                "4",
            ],
            &["diario"],
            &["diario", "--fecha", "2026-10-21", "--limit", "3", "--json"],
            &["conciliar", "--cargue", "C1"],
            &["thefactory", "descargar"],
            &["derechos", "validar"],
            &["fev", "ajustar"],
            &["fev", "verificar"],
            &["rips", "generar"],
            &["cuv", "generar"],
            &["cuv", "barrer"],
            &["soporte", "generar"],
            &["radicacion", "run"],
            &["radicacion", "recheck-step2"],
            &["config", "check", "database.url"],
            &["db", "migrate"],
        ];
        for args in commands {
            let argv: Vec<&str> = std::iter::once("facturacion")
                .chain(args.iter().copied())
                .collect();
            if let Err(err) = Cli::try_parse_from(&argv) {
                panic!("{args:?} no parsea: {err}");
            }
        }
    }

    #[test]
    fn dry_run_and_live_do_not_exist() {
        for flag in ["--dry-run", "--live", "--yes"] {
            assert!(
                Cli::try_parse_from(["facturacion", "rips", "generar", flag]).is_err(),
                "{flag}"
            );
        }
    }
}
