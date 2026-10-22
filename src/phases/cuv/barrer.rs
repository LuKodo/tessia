//! `cuv barrer`: the folder-wide sweep of the CUV TXTs (Java
//! `Vista_ActualizarCUVMinisterioLocal`, Go `cmd_cuv_barrer.go`).
//!
//! Every top-level `*.txt` artifact of the cargue that is a JSON object is
//! rewritten with [`super::sweep::sweep_local`]; other TXTs are left alone and
//! the ones Java could not process are reported as `ERROR` and not written. A
//! rewritten `<F>.txt` that now carries a CUV is recorded `DONE` in phase `cuv`
//! (motivo `RVG18: CUV existente` for RVG18 fixes). Sequential, like Java.
//!
//! As in Go, only `<F>.txt` files (not `<F>_ERROR.txt`) can be rewritten: the
//! other names have no invoice to claim and are skipped with a warning.

use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::{Result, anyhow};
use regex::Regex;

use super::sispro::{SisproClient, SisproOptions};
use super::sweep::{ProcesoIdLookup, sweep_local};
use super::txt::txt_has_cuv;
use super::{LEASE_TTL, SKIP_OTHER_MACHINE, lease_name};
use crate::context::AppContext;
use crate::domain::{Outcome, Phase, normalize_invoice};
use crate::phases::common::PhaseReport;
use crate::stores::proceso::ClaimItem;

/// `<F>.txt`, the only name that belongs to an invoice.
static TXT_INVOICE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^([A-Za-z]{2,4}[0-9]+)\.[Tt][Xx][Tt]$")
        .unwrap_or_else(|_| unreachable!("constant regex"))
});

/// The reason stored for an invoice completed by an RVG18 fix.
pub const RVG18_REASON: &str = "RVG18: CUV existente";

/// What a sweep did.
#[derive(Debug, Default)]
pub struct BarrerExecution {
    pub lines: Vec<String>,
    /// `DONE` outcomes stored (rewritten TXTs that now carry a CUV).
    pub report: PhaseReport,
    /// Failures that stopped the sweep (the others were stored).
    pub errors: Vec<anyhow::Error>,
}

struct Item {
    name: String,
    content: Vec<u8>,
    rvg18: bool,
}

/// The invoice of a `<F>.txt` name (not `_ERROR` or other names).
fn txt_invoice(name: &str) -> Option<String> {
    TXT_INVOICE
        .captures(name)
        .and_then(|c| c.get(1))
        .map(|m| normalize_invoice(m.as_str()))
}

#[derive(Default)]
struct Counts {
    unchanged: usize,
    skipped: usize,
    failed: usize,
    rvg18: usize,
}

/// Classifies every TXT: what to update and what to report.
fn plan(ctx: &AppContext, cargue_id: &str, lines: &mut Vec<String>) -> Result<(Vec<Item>, Counts)> {
    let artifacts = ctx.artifacts();
    let mut names: Vec<String> = artifacts
        .list(cargue_id)?
        .into_iter()
        .map(|a| a.rel_path)
        .filter(|rel| !rel.contains('/') && rel.to_lowercase().ends_with(".txt"))
        .collect();
    names.sort();
    let mut items = Vec::new();
    let mut counts = Counts::default();
    for name in names {
        let content = artifacts.fetch_bytes(cargue_id, &name)?;
        match sweep_local(&content, None) {
            Err(err) => {
                counts.failed += 1;
                lines.push(format!("  {name} ERROR {err}"));
            }
            Ok(swept) => match swept.out {
                None => {
                    counts.skipped += 1;
                    lines.push(format!("  {name} omitido (no es un JSON)"));
                }
                Some(out) if !swept.rvg18 && out.as_bytes() == content.as_slice() => {
                    counts.unchanged += 1;
                }
                Some(_) => {
                    let note = if swept.rvg18 {
                        counts.rvg18 += 1;
                        " (RVG18: CUV existente, ProcesoId por ConsultarCUV)"
                    } else {
                        ""
                    };
                    lines.push(format!("  {name} actualizar{note}"));
                    items.push(Item {
                        name,
                        content,
                        rvg18: swept.rvg18,
                    });
                }
            },
        }
    }
    lines.push(format!(
        "cuv barrer: actualizar={} rvg18={} sin_cambios={} omitidos={} errores={}",
        items.len(),
        counts.rvg18,
        counts.unchanged,
        counts.skipped,
        counts.failed
    ));
    Ok((items, counts))
}

/// Rewrites one TXT; returns the new content.
fn rewrite(
    ctx: &AppContext,
    cargue_id: &str,
    item: &Item,
    client: Option<&SisproClient>,
) -> Result<String> {
    let lookup = |cuv: &str| client.and_then(|c| c.proceso_id(cuv));
    let lookup_ref: Option<ProcesoIdLookup<'_>> = item.rvg18.then_some(&lookup);
    let swept =
        sweep_local(&item.content, lookup_ref).map_err(|e| anyhow!("{}: {e}", item.name))?;
    let out = swept
        .out
        .ok_or_else(|| anyhow!("{}: ya no es un JSON", item.name))?;
    ctx.artifacts()
        .publish_bytes(cargue_id, &item.name, out.as_bytes())?;
    Ok(out)
}

/// Runs `cuv barrer` for the cargue.
pub fn execute(ctx: &AppContext, cargue_id: &str) -> Result<BarrerExecution> {
    let mut execution = BarrerExecution::default();
    let (items, counts) = plan(ctx, cargue_id, &mut execution.lines)?;
    if items.is_empty() {
        return Ok(execution);
    }
    let client = if counts.rvg18 > 0 {
        let options = SisproOptions::from_settings(&ctx.settings)?;
        Some(SisproClient::new(options, ctx.limiters.get("sispro"))?)
    } else {
        None
    };
    let proceso = ctx.proceso();
    let keys: Vec<String> = items.iter().filter_map(|i| txt_invoice(&i.name)).collect();
    let previous = proceso.load(Phase::Cuv, Some(&keys))?;
    let claim_items: Vec<ClaimItem> = keys
        .iter()
        .map(|k| ClaimItem::from_loaded(k, previous.get(k)))
        .collect();
    let claimed =
        proceso.claim_available(Phase::Cuv, &claim_items, &ctx.claim_options(cargue_id))?;

    let mut done: HashMap<String, String> = HashMap::new();
    for item in &items {
        let unavailable = || {
            format!(
                "warning: skipping {} because phase ownership is unavailable",
                item.name
            )
        };
        let Some(key) = txt_invoice(&item.name).filter(|k| claimed.contains_key(k)) else {
            execution.lines.push(unavailable());
            continue;
        };
        let lease = ctx
            .leases()
            .guard(&lease_name(cargue_id, &key), &ctx.owner, LEASE_TTL);
        let _lease = match lease {
            Ok(Some(guard)) => guard,
            Ok(None) => {
                execution.lines.push(unavailable());
                execution.report.skip(&key, SKIP_OTHER_MACHINE);
                continue;
            }
            Err(err) => {
                execution.errors.push(err.into());
                break;
            }
        };
        match rewrite(ctx, cargue_id, item, client.as_ref()) {
            Ok(out) => {
                execution.lines.push(format!("  {} actualizado", item.name));
                // A rewritten <F>.txt that now carries a CUV completes the cuv phase.
                if txt_has_cuv(out.as_bytes()) {
                    let reason = if item.rvg18 { RVG18_REASON } else { "" };
                    done.insert(key, reason.to_owned());
                }
            }
            Err(err) => {
                execution.errors.push(err);
                break;
            }
        }
    }
    settle(ctx, &claimed, &previous, &done, &mut execution);
    Ok(execution)
}

/// Stores the `DONE` outcomes and gives back the claims that were not used
/// (Go `Record` + `CancelStarted`).
fn settle(
    ctx: &AppContext,
    claimed: &HashMap<String, i64>,
    previous: &HashMap<String, crate::stores::proceso::ProcesoState>,
    done: &HashMap<String, String>,
    execution: &mut BarrerExecution,
) {
    let proceso = ctx.proceso();
    let mut unused = HashMap::new();
    let mut ordered: Vec<(&String, &i64)> = claimed.iter().collect();
    ordered.sort();
    for (key, version) in ordered {
        let Some(reason) = done.get(key) else {
            unused.insert(key.clone(), *version);
            continue;
        };
        let outcome = Outcome::new(crate::domain::PhaseState::Done, reason.clone());
        match proceso.save_outcome(Phase::Cuv, key, &outcome, *version) {
            Ok(true) => execution.report.push(key, outcome),
            Ok(false) => execution.report.skip(key, SKIP_OTHER_MACHINE),
            Err(err) => execution.errors.push(err.into()),
        }
    }
    if let Err(err) = proceso.release(Phase::Cuv, &unused, previous) {
        execution.errors.push(err.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_invoice_txt_names_belong_to_an_invoice() {
        assert_eq!(txt_invoice("FEV10.txt").as_deref(), Some("FEV10"));
        assert_eq!(txt_invoice("fev10.TXT").as_deref(), Some("FEV10"));
        for name in [
            "FEV10_ERROR.txt",
            "FEV10_RESPUESTA_NO_CORRESPONDE.txt",
            "notes.txt",
            "A1.txt",
            "FEV10.json",
        ] {
            assert_eq!(txt_invoice(name), None, "{name}");
        }
    }
}
