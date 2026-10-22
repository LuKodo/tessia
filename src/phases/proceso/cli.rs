//! Command line of `proceso run`.

use anyhow::{Result, bail};
use clap::{Args as ClapArgs, Subcommand};

use super::config::{DEFAULT_BATCH, RunConfig, Tuning, parse_duration};
use super::events::Mode;
use super::steps::{POOLED, default_workers, select};
use crate::domain::normalize_invoice;
use crate::phases::common::CommonArgs;

/// Proceso completo.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(name = "run", about = "Ejecuta todas las fases de un cargue")]
    Run(Box<RunArgs>),
}

/// Pasa cada factura preparada de un cargue por todas las fases como un
/// pipeline independiente por factura:
///
///   thefactory -> fev -> (conciliar si fev marca VALOR_DIFERENTE) -> derechos
///   -> rips -> cuv -> soporte -> cola de radicación
///
/// Cada fase es un pool de hilos y la factura avanza apenas termina la fase
/// anterior. `siesa` nunca se ejecuta. El argumento es un CSV de convenio
/// local o el id de un cargue que ya existe (como en `convenio preparar`);
/// si el cargue aún no está preparado se prepara primero (paso 0) y solo las
/// facturas preparadas entran al pipeline.
///
/// Varias máquinas pueden correr `proceso run` sobre el mismo cargue a la vez:
/// cada fase reclama cada factura en la base de datos, así que el trabajo se
/// reparte y una factura que otra máquina tiene en curso se omite
/// (EN_PROCESO_OTRA_MAQUINA) sin contarse como falla ni reemplazarse. No hay
/// lease por cargue; solo la radicación toma el suyo y sube un lote a la vez.
///
/// Una factura que termina EXCLUDED / REVISION_MANUAL / FAILED en una fase sale
/// del pipeline y la corrida sigue. Un error sistémico aborta la corrida salvo
/// con --continue-on-error. La radicación es la única compuerta por lotes: las
/// facturas terminadas se encolan y se radican de a --batch-radicacion o antes,
/// cuando la más antigua lleva --radicacion-interval esperando.
///
/// Reemplazo: con --limit N, una factura que se detiene antes de la radicación
/// se reemplaza por la siguiente pendiente del CSV (solo si el paso 0 corrió
/// desde un CSV en esta llamada); tras 10 bajas seguidas sin que ninguna llegue
/// a radicación el reemplazo se pausa. --no-replace lo desactiva.
///
/// Cada fase corre hasta --workers-<fase> facturas a la vez; la concurrencia
/// arranca en ese tope, se reduce a la mitad ante un conflicto transitorio de
/// la base de datos y crece de a una cada 20 facturas limpias tras un enfriamiento.
///
/// Pasos válidos para --from/--to/--skip: convenio, thefactory, fev, conciliar,
/// derechos, rips, cuv, soporte, radicacion.
#[derive(Debug, Clone, ClapArgs)]
#[command(override_usage = "facturacion proceso run [opciones] <CSV_O_CARGUE>")]
pub struct RunArgs {
    #[command(flatten)]
    pub common: CommonArgs,

    /// CSV de convenio local (se registra como cargue y se sube al bucket) o
    /// id de un cargue existente
    #[arg(value_name = "CSV_O_CARGUE")]
    pub source: Option<String>,

    /// CSV de convenio local para el paso 0 (alternativa al argumento posicional)
    #[arg(long, value_name = "ARCHIVO")]
    pub csv: Option<String>,

    /// Vuelve a preparar el CSV aunque el cargue ya esté preparado (reinicia
    /// las facturas del dataset; por defecto se reutiliza el dataset guardado)
    #[arg(long)]
    pub repreparar: bool,

    /// Empieza en este paso (omite los anteriores)
    #[arg(long, value_name = "PASO")]
    pub from: Option<String>,

    /// Termina después de este paso (omite los siguientes)
    #[arg(long, value_name = "PASO")]
    pub to: Option<String>,

    #[command(flatten)]
    pub flags: RunFlags,
}

/// Flags shared by `proceso run` and `diario`.
#[derive(Debug, Clone, ClapArgs)]
pub struct RunFlags {
    /// Prepara/procesa hasta N facturas pendientes (0 = todas); las excluidas o en
    /// revisión manual no cuentan y las ya terminadas se omiten sin contar
    #[arg(
        long,
        value_name = "N",
        default_value_t = 0,
        allow_negative_numbers = true
    )]
    pub limit: i64,

    /// No reemplaza las facturas que se detienen antes de la radicación
    #[arg(long)]
    pub no_replace: bool,

    /// Monto máximo por factura en COP para la preparación
    #[arg(long, value_name = "COP")]
    pub monto_maximo: Option<String>,

    /// Código REPS con sede (12 dígitos) para `rips generar`
    #[arg(long, value_name = "CODIGO")]
    pub cod_prestador: Option<String>,

    /// Omite estos pasos (separados por coma)
    #[arg(long, value_name = "PASOS", value_delimiter = ',')]
    pub skip: Vec<String>,

    /// No aborta ante un error sistémico; lo informa al final
    #[arg(long)]
    pub continue_on_error: bool,

    /// Facturas por subida de radicación
    #[arg(long, value_name = "N", default_value_t = DEFAULT_BATCH as i64, allow_negative_numbers = true)]
    pub batch_radicacion: i64,

    /// Radica también un lote parcial cuando su factura más antigua lleva este
    /// tiempo esperando (p. ej. 30s, 1m; 0 lo desactiva)
    #[arg(
        long,
        value_name = "DURACION",
        default_value = "1m",
        allow_hyphen_values = true
    )]
    pub radicacion_interval: String,

    /// Pool de thefactory (por defecto 30)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_thefactory: Option<i64>,
    /// Pool de fev (por defecto 8)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_fev: Option<i64>,
    /// Pool de conciliar (por defecto 8)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_conciliar: Option<i64>,
    /// Pool de derechos (por defecto 30)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_derechos: Option<i64>,
    /// Pool de rips (por defecto 8)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_rips: Option<i64>,
    /// Pool de cuv (por defecto 40)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_cuv: Option<i64>,
    /// Pool de soporte (por defecto 20)
    #[arg(long, value_name = "N", allow_negative_numbers = true)]
    pub workers_soporte: Option<i64>,

    /// Emite un evento JSON por línea en stdout
    #[arg(long)]
    pub json: bool,

    /// Panel de progreso en vivo (líneas simples si stdout no es una terminal)
    #[arg(long)]
    pub panel: bool,
}

impl RunFlags {
    fn pool(&self, step: &str) -> Option<i64> {
        match step {
            "thefactory" => self.workers_thefactory,
            "fev" => self.workers_fev,
            "conciliar" => self.workers_conciliar,
            "derechos" => self.workers_derechos,
            "rips" => self.workers_rips,
            "cuv" => self.workers_cuv,
            "soporte" => self.workers_soporte,
            _ => None,
        }
    }

    /// Validates the shared flags (Go `runProcesoRun`) and applies them to
    /// `cfg`; `default_pool` is the generic `--workers`.
    pub fn apply(&self, cfg: &mut RunConfig, default_pool: Option<usize>) -> Result<()> {
        if self.json && self.panel {
            bail!("--json and --panel are mutually exclusive");
        }
        if default_pool == Some(0) {
            bail!("--workers must be >= 1");
        }
        for step in POOLED {
            let size = match self.pool(step) {
                Some(n) if n < 1 => bail!("--workers-{step} must be >= 1"),
                Some(n) => usize::try_from(n)?,
                None => default_pool.unwrap_or_else(|| default_workers(step)),
            };
            cfg.workers.insert(step, size);
        }
        if self.limit < 0 {
            bail!("--limit must be >= 0");
        }
        if self.batch_radicacion < 1 {
            bail!("--batch-radicacion must be >= 1");
        }
        cfg.limit = usize::try_from(self.limit)?;
        cfg.batch_radicacion = usize::try_from(self.batch_radicacion)?;
        cfg.radicacion_interval =
            parse_duration("--radicacion-interval", &self.radicacion_interval)?;
        cfg.no_replace = self.no_replace;
        cfg.max_amount = self.monto_maximo.clone();
        cfg.cod_prestador = self.cod_prestador.clone();
        cfg.continue_on_error = self.continue_on_error;
        cfg.mode = match (self.json, self.panel) {
            (true, _) => Mode::Json,
            (_, true) => Mode::Panel,
            _ => Mode::Text,
        };
        cfg.tuning = Tuning::default();
        Ok(())
    }
}

impl RunArgs {
    fn source(&self) -> Result<String> {
        let given: Vec<&str> = [
            self.source.as_deref(),
            self.csv.as_deref(),
            self.common.cargue.as_deref(),
        ]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
        match given.as_slice() {
            [one] => Ok((*one).to_owned()),
            _ => bail!(
                "uso: facturacion proceso run [opciones] <csv|cargue> (uno solo de: argumento, --csv, --cargue)"
            ),
        }
    }

    /// Validates the flags and resolves them.
    pub fn to_config(&self) -> Result<RunConfig> {
        let mut cfg = RunConfig::new(&self.source()?);
        self.flags.apply(&mut cfg, self.common.workers)?;
        cfg.active = select(self.from.as_deref(), self.to.as_deref(), &self.flags.skip)?;
        cfg.invoices = crate::domain::invoice::normalize_all(
            self.common.invoices.iter().map(|i| normalize_invoice(i)),
        );
        cfg.reprepare = self.repreparar;
        Ok(cfg)
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: RunArgs,
    }

    fn parse(argv: &[&str]) -> Result<RunConfig> {
        let all: Vec<&str> = std::iter::once("run").chain(argv.iter().copied()).collect();
        Wrapper::try_parse_from(all)?.args.to_config()
    }

    #[test]
    fn defaults_are_the_go_ones() {
        let cfg = parse(&["C1"]).expect("config");
        assert_eq!(cfg.source, "C1");
        assert_eq!(cfg.limit, 0);
        assert_eq!(cfg.batch_radicacion, 200);
        assert_eq!(cfg.radicacion_interval.as_secs(), 60);
        assert_eq!(cfg.workers_of("derechos"), 30);
        assert_eq!(cfg.mode, Mode::Text);
        assert!(!cfg.no_replace && !cfg.continue_on_error);
    }

    #[test]
    fn flags_map_to_the_config() {
        let cfg = parse(&[
            "--cargue",
            "C1",
            "--invoices",
            "fev-1,evs2",
            "--limit",
            "5",
            "--no-replace",
            "--from",
            "fev",
            "--to",
            "cuv",
            "--skip",
            "conciliar",
            "--continue-on-error",
            "--batch-radicacion",
            "50",
            "--radicacion-interval",
            "0",
            "--workers",
            "3",
            "--workers-cuv",
            "7",
            "--json",
            "--monto-maximo",
            "100000",
            "--cod-prestador",
            "123456789012",
        ])
        .expect("config");
        assert_eq!(cfg.source, "C1");
        assert_eq!(cfg.invoices, ["FEV1", "EVS2"]);
        assert_eq!((cfg.limit, cfg.batch_radicacion), (5, 50));
        assert!(cfg.radicacion_interval.is_zero());
        assert_eq!(cfg.workers_of("fev"), 3);
        assert_eq!(cfg.workers_of("cuv"), 7);
        assert_eq!(cfg.mode, Mode::Json);
        assert!(cfg.active.has("derechos") && !cfg.active.has("conciliar"));
        assert!(!cfg.active.has("soporte") && !cfg.active.has("thefactory"));
        assert_eq!(cfg.max_amount.as_deref(), Some("100000"));
    }

    #[test]
    fn invalid_flags_fail_with_the_go_messages() {
        let msg = |argv: &[&str]| parse(argv).expect_err("error").to_string();
        assert_eq!(
            msg(&["C1", "--json", "--panel"]),
            "--json and --panel are mutually exclusive"
        );
        assert_eq!(msg(&["C1", "--limit", "-1"]), "--limit must be >= 0");
        assert_eq!(
            msg(&["C1", "--batch-radicacion", "0"]),
            "--batch-radicacion must be >= 1"
        );
        assert_eq!(
            msg(&["C1", "--workers-fev", "0"]),
            "--workers-fev must be >= 1"
        );
        assert_eq!(msg(&["C1", "--workers", "0"]), "--workers must be >= 1");
        assert_eq!(
            msg(&["C1", "--radicacion-interval", "-1s"]),
            "--radicacion-interval must be >= 0"
        );
        assert!(msg(&["C1", "--from", "nope"]).starts_with("unknown step"));
        assert!(msg(&[]).starts_with("uso: facturacion proceso run"));
        assert!(msg(&["C1", "--csv", "x.csv"]).starts_with("uso: facturacion proceso run"));
    }

    #[test]
    fn dry_run_flags_do_not_exist() {
        for flag in ["--dry-run", "--live", "--yes"] {
            assert!(
                Wrapper::try_parse_from(["run", "C1", flag]).is_err(),
                "{flag}"
            );
        }
    }
}
