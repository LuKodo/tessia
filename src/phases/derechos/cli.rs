//! Command line of `derechos validar`.

use std::time::Duration;

use clap::{Args as ClapArgs, Subcommand};

use crate::phases::common::{CommonArgs, PhaseRequest};

/// Default parallel validator calls (Go `tessia.derechos.pool`).
pub const DEFAULT_CONCURRENCY: usize = 30;
/// Default freshness of the rights cache.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(24 * 3600);

/// Derechos de afiliados.
#[derive(Debug, Clone, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Clone, Subcommand)]
pub enum Command {
    #[command(
        name = "validar",
        about = "Valida los derechos de los afiliados (régimen, zona, DANE, sexo, fecha de nacimiento)",
        long_about = "Lee el dataset del cargue (guardado por `convenio preparar`) y consulta una vez por afiliado \
el validador de derechos de Mutual Ser. Los resultados quedan en `derechos/<factura>.json` (un archivo por \
factura, en el bucket) y en proceso_estado (fase derechos). Un afiliado no encontrado, con datos incompletos \
o un fallo del validador deja la factura en REVISION_MANUAL; esas facturas solo se reintentan con --invoices. \
Si el validador falló técnicamente para más de un porcentaje de los afiliados consultados (parámetros: 50 % \
por defecto, mínimo 4 consultados) el comando también falla, después de guardar. Un 401 renueva el token una \
vez (compartido por todos los hilos) y reenvía la solicitud; un token que sigue rechazado, o un inicio de \
sesión fallido, detiene la corrida: se guarda lo resuelto, las demás facturas quedan sin tocar y el comando \
falla. El CSV nunca se usa como respaldo.\n\n\
Un cargue manual ya en el formato preparado (algún renglón con CLASIFICACION 714/799) no consulta el \
validador: el régimen, DANE, zona, sexo y fecha de nacimiento salen de las columnas del archivo (origen \
\"archivo\"); los datos incompletos o inconsistentes van a REVISION_MANUAL."
    )]
    Validar(ValidarArgs),
}

/// Arguments of the `validar` subcommand.
#[derive(Debug, Clone, ClapArgs)]
pub struct ValidarArgs {
    #[command(flatten)]
    pub common: CommonArgs,
    /// Llamadas paralelas al validador (también acotan la escritura de resultados)
    #[arg(long, value_name = "N", default_value_t = DEFAULT_CONCURRENCY)]
    pub concurrency: usize,
    /// Reutiliza afiliados validados hace menos de D (p. ej. 24h, 90m; 0 desactiva la lectura del caché)
    #[arg(long, value_name = "D", default_value = "24h", value_parser = parse_duration)]
    pub cache_ttl: Duration,
}

impl ValidarArgs {
    /// The request of the command; `--workers`, when given, wins over `--concurrency`.
    pub fn request(&self) -> anyhow::Result<PhaseRequest> {
        let mut request = self.common.to_request()?;
        request.workers = Some(request.workers.unwrap_or(self.concurrency).max(1));
        Ok(request)
    }
}

/// Parses a Go-style duration: a sequence of `<number><unit>` with units
/// `h`, `m`, `s`, `ms`; a bare `0` is zero.
pub fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    if text == "0" {
        return Ok(Duration::ZERO);
    }
    if text.is_empty() {
        return Err("duración vacía".into());
    }
    let mut rest = text;
    let mut total = Duration::ZERO;
    while !rest.is_empty() {
        let digits = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let (number, tail) = rest.split_at(digits);
        let unit_len = tail
            .find(|c: char| c.is_ascii_digit() || c == '.')
            .unwrap_or(tail.len());
        let (unit, next) = tail.split_at(unit_len);
        let value: f64 = number
            .parse()
            .map_err(|_| format!("duración inválida: {text:?}"))?;
        let seconds = match unit {
            "h" => 3600.0,
            "m" => 60.0,
            "s" => 1.0,
            "ms" => 0.001,
            _ => return Err(format!("duración inválida (unidad {unit:?}): {text:?}")),
        };
        total += Duration::try_from_secs_f64(value * seconds)
            .map_err(|_| format!("duración inválida: {text:?}"))?;
        rest = next;
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Wrapper {
        #[command(flatten)]
        args: ValidarArgs,
    }

    #[test]
    fn durations_follow_go_syntax() {
        assert_eq!(parse_duration("24h"), Ok(Duration::from_secs(86_400)));
        assert_eq!(parse_duration("1h30m"), Ok(Duration::from_secs(5_400)));
        assert_eq!(parse_duration("90s"), Ok(Duration::from_secs(90)));
        assert_eq!(parse_duration("1.5h"), Ok(Duration::from_secs(5_400)));
        assert_eq!(parse_duration("250ms"), Ok(Duration::from_millis(250)));
        assert_eq!(parse_duration("0"), Ok(Duration::ZERO));
        assert_eq!(parse_duration("0s"), Ok(Duration::ZERO));
        for bad in ["", "h", "5", "5d", "-1h", "1h2"] {
            assert!(parse_duration(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn defaults_match_go() {
        let wrapper = Wrapper::try_parse_from(["x"]).expect("parse");
        assert_eq!(wrapper.args.concurrency, 30);
        assert_eq!(wrapper.args.cache_ttl, DEFAULT_CACHE_TTL);
        let request = wrapper.args.request();
        assert!(request.is_err(), "--cargue is required to run");
    }

    #[test]
    fn flags_build_the_request() {
        let wrapper = Wrapper::try_parse_from([
            "x",
            "--cargue",
            "C1",
            "--invoices",
            "fev-1,evs2",
            "--concurrency",
            "5",
            "--cache-ttl",
            "0",
        ])
        .expect("parse");
        assert_eq!(wrapper.args.cache_ttl, Duration::ZERO);
        let request = wrapper.args.request().expect("request");
        assert_eq!(request.cargue_id, "C1");
        assert_eq!(
            request.invoices,
            Some(vec!["FEV1".to_string(), "EVS2".to_string()])
        );
        assert_eq!(request.workers, Some(5));
    }

    #[test]
    fn workers_wins_over_concurrency() {
        let wrapper =
            Wrapper::try_parse_from(["x", "--cargue", "C1", "--workers", "3"]).expect("parse");
        assert_eq!(wrapper.args.request().expect("request").workers, Some(3));
    }
}
