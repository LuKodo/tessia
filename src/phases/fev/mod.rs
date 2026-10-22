//! `fev ajustar` and `fev verificar`: invoice XML adjustment and XML-vs-convenio
//! value check (catalog FEV-01..07).
//!
//! * [`ajustar`]: edits the XML artifacts of a cargue (`--op cucon|cuota`).
//! * [`verificar`]: compares each invoice's XML total with its convenio total and
//!   records phase `fev_valor`; `proceso run` calls [`run_verificar_phase`].

pub mod ajustar;
pub mod cli;
pub mod verificar;
mod xmlsrc;

use anyhow::{Result, anyhow, bail};

use crate::context::AppContext;
use crate::phases::common::{PhaseReport, PhaseRequest};

pub use ajustar::{AjustarOp, AjustarRun, cuv_lease_name};
pub use cli::{Args, Command};
pub use verificar::{ConvenioInvoice, Verdict, VerificarRun, compare, convenio_total};

/// Entry point of the command line.
pub fn run(ctx: &AppContext, args: Args) -> Result<()> {
    match args.command {
        Command::Ajustar(args) => run_ajustar(ctx, &args),
        Command::Verificar(args) => run_verificar(ctx, &args),
    }
}

fn run_ajustar(ctx: &AppContext, args: &cli::AjustarArgs) -> Result<()> {
    let req = args.common.to_request()?;
    let op = resolve_op(ctx, args)?;
    let run = ajustar::ajustar(ctx, &req, &op)?;
    for line in &run.lines {
        println!("{line}");
    }
    if run.write_errors.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(run.write_errors.join("; ")))
    }
}

/// The operation of `--op`; `cucon` needs `--cucon` or `minsalud.numero_contrato`.
fn resolve_op(ctx: &AppContext, args: &cli::AjustarArgs) -> Result<AjustarOp> {
    match args.op {
        None => bail!("--op debe ser cucon o cuota"),
        Some(cli::OpName::Cuota) => Ok(AjustarOp::Cuota),
        Some(cli::OpName::Cucon) => {
            let given = args.cucon.as_deref().map(str::trim).unwrap_or_default();
            if !given.is_empty() {
                return Ok(AjustarOp::Cucon(given.to_owned()));
            }
            let configured = ctx
                .settings
                .require_one("minsalud.numero_contrato")
                .map_err(|err| anyhow!("--cucon o {err}"))?;
            Ok(AjustarOp::Cucon(configured.trim().to_owned()))
        }
    }
}

fn run_verificar(ctx: &AppContext, args: &cli::VerificarArgs) -> Result<()> {
    let req = args.common.to_request()?;
    let invoices = verificar::load_convenio_invoices(ctx, &req)?;
    let run = verificar::verify(ctx, &req, &invoices)?;
    for line in &run.lines {
        println!("{line}");
    }
    if run.mismatches() > 0 {
        bail!(
            "{} factura(s) sin valor coincidente entre XML y convenio",
            run.mismatches()
        );
    }
    Ok(())
}

/// `fev ajustar` as a phase (`Phase::FevAjuste`). A standalone command in Go;
/// `proceso run` does not call it.
pub fn run_ajustar_phase(
    ctx: &AppContext,
    req: &PhaseRequest,
    op: &AjustarOp,
) -> Result<PhaseReport> {
    Ok(ajustar::ajustar(ctx, req, op)?.report)
}

/// `fev verificar` as a phase (`Phase::FevValor`), called by `proceso run`.
/// Invoices that differ from the convenio are `REVISION_MANUAL` (not an error:
/// `conciliar` repairs them next).
pub fn run_verificar_phase(ctx: &AppContext, req: &PhaseRequest) -> Result<PhaseReport> {
    let invoices = verificar::load_convenio_invoices(ctx, req)?;
    Ok(verificar::verify(ctx, req, &invoices)?.report)
}
