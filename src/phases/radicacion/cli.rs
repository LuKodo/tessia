//! Command line of `radicacion run` and `radicacion recheck-step2`.

use std::time::Duration;

use clap::{Args as ClapArgs, Subcommand};

use super::engine::Wait;

/// Radicación en Mutual Ser.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(
        name = "run",
        about = "Radica las facturas de un cargue en Mutual Ser (pipeline de 4 pasos por factura)",
        long_about = "Radica las facturas del cargue en Mutual Ser: 1 ZIP XML/PDF, 2 TXT CUV, 3 JSON \
                      (RIPS) y 4 ZIP CRC. Cada factura avanza por su cuenta apenas Mutual Ser confirma \
                      un paso. El avance queda en radicacion_estado: una corrida interrumpida continúa \
                      donde quedó (las cargas enviadas se consultan con el mismo id, nunca se reenvían). \
                      Sin --invoices se radican todas las facturas que tienen archivos en el cargue."
    )]
    Run(RunArgs),
    #[command(
        name = "recheck-step2",
        about = "Revisa de nuevo el paso 2 (CUV) de facturas ya radicadas contra filter-xml",
        long_about = "Reverifica el paso 2 (TXT CUV) de facturas ya radicadas, solo contra filter-xml. \
                      Un statusType=3 registra una nota de ajuste en radicacion_estado; un resultado \
                      negativo nunca borra un paso confirmado. Imprime \
                      total= confirmed= not_confirmed= rejected= errors="
    )]
    RecheckStep2(RecheckStep2Args),
}

/// A duration with a sign (`-1s` disables some caps), parsed Go-style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignedDuration {
    pub negative: bool,
    pub value: Duration,
}

impl SignedDuration {
    /// Zero = default, negative = disabled.
    pub fn wait(self) -> Wait {
        if self.negative && !self.value.is_zero() {
            Wait::Disabled
        } else if self.value.is_zero() {
            Wait::Default
        } else {
            Wait::Fixed(self.value)
        }
    }

    /// A duration that cannot be negative; zero means "default".
    pub fn positive(self) -> Duration {
        if self.negative {
            Duration::ZERO
        } else {
            self.value
        }
    }
}

/// Parses a Go duration (`3h`, `10m`, `45s`, `500ms`, `1h30m`, `1.5s`, `-1s`, `0`).
pub fn parse_duration(text: &str) -> Result<SignedDuration, String> {
    let invalid = || format!("duración inválida: {text:?} (use p. ej. 90s, 10m, 3h)");
    let trimmed = text.trim();
    let (negative, rest) = match trimmed.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, trimmed.strip_prefix('+').unwrap_or(trimmed)),
    };
    if rest.is_empty() {
        return Err(invalid());
    }
    if rest == "0" {
        return Ok(SignedDuration {
            negative: false,
            value: Duration::ZERO,
        });
    }
    let mut total = Duration::ZERO;
    let mut remaining = rest;
    while !remaining.is_empty() {
        let number_len = remaining
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .ok_or_else(invalid)?;
        let (number, tail) = remaining.split_at(number_len);
        let unit_len = tail
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(tail.len());
        let (unit, next) = tail.split_at(unit_len);
        let scale = match unit {
            "ns" => 1e-9,
            "us" | "µs" => 1e-6,
            "ms" => 1e-3,
            "s" => 1.0,
            "m" => 60.0,
            "h" => 3600.0,
            _ => return Err(invalid()),
        };
        let amount: f64 = number.parse().map_err(|_| invalid())?;
        total += Duration::try_from_secs_f64(amount * scale).map_err(|_| invalid())?;
        remaining = next;
    }
    Ok(SignedDuration {
        negative,
        value: total,
    })
}

fn parse_step(text: &str) -> Result<u8, String> {
    match text.trim().parse::<u8>() {
        Ok(n @ 1..=4) => Ok(n),
        _ => Err("--step debe estar entre 1 y 4".to_owned()),
    }
}

/// Arguments of the `run` subcommand.
#[derive(Debug, Clone, ClapArgs, Default)]
pub struct RunArgs {
    /// Cargue a radicar (el nombre del CSV sin extensión, p. ej. Convenio_21102026_21102026)
    #[arg(value_name = "CARGUE")]
    pub cargue_positional: Option<String>,
    /// Cargue a radicar (igual que el argumento posicional)
    #[arg(long = "cargue", value_name = "ID")]
    pub cargue: Option<String>,
    /// Radicar solo estas facturas (separadas por coma)
    #[arg(long, value_name = "FACTURAS", value_delimiter = ',')]
    pub invoices: Vec<String>,
    /// Ejecutar solo la etapa N (1-4): las facturas cuyo paso actual es N
    #[arg(long, value_name = "N", value_parser = parse_step)]
    pub step: Option<u8>,
    /// Archivos por carga (por defecto 200, máximo 400)
    #[arg(long, value_name = "N")]
    pub batch_max: Option<usize>,
    /// Facturas de un lote que se revisan a la vez antes de subir (por defecto 20; 1 = una por una)
    #[arg(long, value_name = "N")]
    pub prepare_concurrency: Option<usize>,
    /// Tope de la consulta previa de los pasos 3 y 4 (por defecto 10s; negativo la desactiva)
    #[arg(long, value_name = "DUR", value_parser = parse_duration, allow_hyphen_values = true)]
    pub precheck_timeout: Option<SignedDuration>,
    /// Detiene la corrida pasado este tiempo (por defecto 3h); lo pendiente continúa en la siguiente
    #[arg(long, value_name = "DUR", value_parser = parse_duration)]
    pub deadline: Option<SignedDuration>,
    /// Espera antes de reenviar un paso que falló (por defecto 10m)
    #[arg(long, value_name = "DUR", value_parser = parse_duration)]
    pub retry_interval: Option<SignedDuration>,
    /// Espera tras reenviar el paso 1 para reintentar el paso 2 (por defecto 45s; negativo la desactiva)
    #[arg(long, value_name = "DUR", value_parser = parse_duration, allow_hyphen_values = true)]
    pub reload_wait: Option<SignedDuration>,
    /// Rechaza un paso tras N envíos (por defecto sin límite)
    #[arg(long, value_name = "N")]
    pub max_sends: Option<i32>,
    /// Sigue en segundo plano el resultado del portal de cada JSON RIPS enviado (apagado por defecto)
    #[arg(long)]
    pub portal_follow: bool,
}

impl RunArgs {
    /// The cargue id from the positional argument or `--cargue`.
    pub fn cargue_id(&self) -> anyhow::Result<&str> {
        match (self.cargue_positional.as_deref(), self.cargue.as_deref()) {
            (Some(a), Some(b)) if a != b => {
                anyhow::bail!("el cargue se indicó dos veces con valores distintos: {a} y {b}")
            }
            (Some(id), _) | (None, Some(id)) if !id.trim().is_empty() => Ok(id.trim()),
            _ => anyhow::bail!("uso: facturacion radicacion run [opciones] <CARGUE>"),
        }
    }
}

/// Arguments of the `recheck-step2` subcommand.
#[derive(Debug, Clone, ClapArgs, Default)]
pub struct RecheckStep2Args {
    /// Facturas a reverificar (separadas por coma)
    #[arg(long, value_name = "FACTURAS", value_delimiter = ',')]
    pub invoices: Vec<String>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: Args,
    }

    fn run_args(argv: &[&str]) -> Result<RunArgs, clap::Error> {
        let mut full = vec!["x", "run"];
        full.extend_from_slice(argv);
        match Wrapper::try_parse_from(full)?.args.command {
            Command::Run(args) => Ok(args),
            Command::RecheckStep2(_) => unreachable!(),
        }
    }

    #[test]
    fn go_durations_parse_with_sign() {
        let secs = |t: &str| parse_duration(t).map(|d| (d.negative, d.value.as_secs_f64()));
        assert_eq!(secs("3h"), Ok((false, 10800.0)));
        assert_eq!(secs("10m"), Ok((false, 600.0)));
        assert_eq!(secs("1h30m"), Ok((false, 5400.0)));
        assert_eq!(secs("1.5s"), Ok((false, 1.5)));
        assert_eq!(secs("500ms"), Ok((false, 0.5)));
        assert_eq!(secs("-1s"), Ok((true, 1.0)));
        assert_eq!(secs("0"), Ok((false, 0.0)));
        for bad in ["", "10", "s", "5x", "1h-2m", "--1s"] {
            assert!(parse_duration(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn negative_means_disabled_and_zero_means_default() {
        let wait = |t: &str| parse_duration(t).expect("duration").wait();
        assert_eq!(wait("-1s"), Wait::Disabled);
        assert_eq!(wait("0"), Wait::Default);
        assert_eq!(wait("5s"), Wait::Fixed(Duration::from_secs(5)));
    }

    #[test]
    fn run_accepts_every_documented_flag() {
        let args = run_args(&[
            "Convenio_21102026_21102026",
            "--invoices",
            "FEV1,FEV2",
            "--step",
            "3",
            "--batch-max",
            "50",
            "--prepare-concurrency",
            "5",
            "--precheck-timeout",
            "-1s",
            "--deadline",
            "2h",
            "--retry-interval",
            "5m",
            "--reload-wait",
            "30s",
            "--max-sends",
            "4",
            "--portal-follow",
        ])
        .expect("parse");
        assert_eq!(
            args.cargue_id().expect("cargue"),
            "Convenio_21102026_21102026"
        );
        assert_eq!(args.invoices, ["FEV1", "FEV2"]);
        assert_eq!(
            (args.step, args.batch_max, args.prepare_concurrency),
            (Some(3), Some(50), Some(5))
        );
        assert_eq!(
            args.precheck_timeout.map(SignedDuration::wait),
            Some(Wait::Disabled)
        );
        assert_eq!(
            args.deadline.map(SignedDuration::positive),
            Some(Duration::from_secs(7200))
        );
        assert_eq!(args.max_sends, Some(4));
        assert!(args.portal_follow);
    }

    #[test]
    fn step_outside_one_to_four_is_an_error() {
        for bad in ["0", "5", "x"] {
            let err = run_args(&["C1", "--step", bad]).expect_err("must fail");
            assert!(
                err.to_string().contains("--step debe estar entre 1 y 4"),
                "{err}"
            );
        }
    }

    #[test]
    fn cargue_comes_from_the_positional_or_the_flag() {
        assert_eq!(
            run_args(&["--cargue", "C1"])
                .expect("parse")
                .cargue_id()
                .expect("id"),
            "C1"
        );
        assert_eq!(
            run_args(&["C1", "--cargue", "C1"])
                .expect("parse")
                .cargue_id()
                .expect("id"),
            "C1"
        );
        assert!(
            run_args(&["C1", "--cargue", "C2"])
                .expect("parse")
                .cargue_id()
                .is_err()
        );
        assert!(run_args(&[]).expect("parse").cargue_id().is_err());
    }
}
