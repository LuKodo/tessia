//! Orchestration of `convenio preparar`: resolve the CSV, the cap and the
//! allowlist, build the catalog from the Redis NT, run the rule engine, then
//! store the dataset and the per-invoice phase states.
//!
//! Split in [`plan_preparation`] (reads, side-effect free except registering
//! the cargue and uploading its CSV) and [`commit_preparation`] (dataset +
//! `proceso_estado`), so the command can print the summary in between and
//! `proceso run` can use either half.

use std::collections::HashSet;
use std::io::BufReader;

use anyhow::{Context, Result, anyhow, bail};
use rust_decimal::Decimal;

use super::csv::{ReadResult, read_csv};
use super::mipres::{MipresChecker, MipresError, needs_mipres};
use super::mipres_client::{MipresClient, MipresOptions};
use super::nt_catalog::{distinct_cums, load_catalog, resolve_version};
use super::prepare::{PrepareOptions, PrepareResult, prepare};
use super::source::{register, resolve_source};
use super::subplan::{AutomaticSubplans, candidate_invoices, selected_set};
use crate::context::AppContext;
use crate::domain::cargue::LoadInfo;
use crate::domain::dataset::{Dataset, Preparation};
use crate::domain::{Outcome, Phase};
use crate::stores::parametros::default_automatic_subplans;
use crate::stores::proceso::SaveRow;
use crate::util::decimal::parse_max_amount;

/// What the user asked for.
#[derive(Debug, Clone, Default)]
pub struct PrepareRequest {
    /// CSV file path or existing cargue id.
    pub source: String,
    /// `--monto-maximo` (text, validated here).
    pub max_amount: Option<String>,
    /// `--invoices` (any spelling; normalized here).
    pub only: Vec<String>,
    /// `--limit` (0 = all).
    pub limit: usize,
}

/// A computed, not yet stored preparation.
#[derive(Debug, Clone)]
pub struct Plan {
    pub load: LoadInfo,
    /// `cargues.seq` of the CSV that was prepared.
    pub cargue: i64,
    pub csv: ReadResult,
    /// NT version used (pinned in the stored preparation).
    pub nt_version: String,
    /// Catalog entries crossed with the CSV.
    pub nt_entries: usize,
    pub max: Decimal,
    pub automatic_subplans: Vec<String>,
    pub only: Vec<String>,
    pub result: PrepareResult,
}

/// Cap and allowlist: `--monto-maximo` else the stored parameters else
/// `convenio.max_invoice_amount`; the allowlist comes from the stored
/// parameters whenever they exist. A failing parameters lookup only warns.
fn resolve_options(ctx: &AppContext, explicit: Option<&str>) -> Result<(Decimal, Vec<String>)> {
    let stored = match ctx.parametros().get() {
        Ok(stored) => stored,
        Err(err) => {
            tracing::warn!(error = %err, "could not read global preparation parameters; using existing configuration");
            None
        }
    };
    let subplans = stored
        .as_ref()
        .map_or_else(default_automatic_subplans, |p| p.automatic_subplans.clone());
    let explicit = explicit.map(str::trim).filter(|t| !t.is_empty());
    let text = match (explicit, &stored) {
        (Some(text), _) => text.to_owned(),
        (None, Some(stored)) => stored.max_invoice_amount.normalize().to_string(),
        (None, None) => ctx
            .settings
            .get("convenio.max_invoice_amount")
            .unwrap_or("")
            .to_owned(),
    };
    if text.is_empty() {
        bail!(
            "the maximum amount is required: --monto-maximo 100000 or TESSIA_CONVENIO_MAX_INVOICE_AMOUNT"
        );
    }
    Ok((parse_max_amount(&text)?, subplans))
}

/// The MIPRES client when some invoice needs the check; the fail-fast order
/// of Go: configuration, then authentication, before anything heavy.
fn mipres_client(ctx: &AppContext) -> Result<MipresClient> {
    let client =
        MipresClient::new(MipresOptions::from_settings(&ctx.settings)?).context("mipres client")?;
    match client.authenticate() {
        Ok(()) => {}
        Err(MipresError::Unavailable(detail)) => {
            tracing::warn!(error = %detail, "MIPRES is not available; EVENTO NO PBS invoices will be held for the next run");
        }
        Err(other) => return Err(anyhow!(other)),
    }
    Ok(client)
}

/// Invoices already filed or closed (best effort: a failing lookup only warns).
fn finished_invoices(ctx: &AppContext, candidates: &[String]) -> HashSet<String> {
    if candidates.is_empty() {
        return HashSet::new();
    }
    ctx.radicacion().finished(candidates).unwrap_or_else(|err| {
        tracing::warn!(error = %err, "could not look up finished invoices for --limit; continuing without it");
        HashSet::new()
    })
}

/// Reads everything and applies the rules. `checker` replaces the MIPRES
/// client built from the settings (tests).
pub fn plan_preparation(
    ctx: &AppContext,
    request: &PrepareRequest,
    checker: Option<&dyn MipresChecker>,
) -> Result<Plan> {
    let (max, subplans) = resolve_options(ctx, request.max_amount.as_deref())?;
    let resolved = resolve_source(ctx, &request.source)?;
    let automatic = AutomaticSubplans::for_origin(resolved.info.origin, &subplans);
    let csv = read_csv(BufReader::new(resolved.bytes.as_slice()))?;
    let only = crate::domain::invoice::normalize_all(&request.only);

    let client;
    let mut mipres: Option<&dyn MipresChecker> = checker;
    if needs_mipres(&csv.lines, &only, &automatic) && mipres.is_none() {
        client =
            mipres_client(ctx).context("EVENTO NO PBS invoices need the MIPRES delivery check")?;
        mipres = Some(&client);
    }

    // Only now is the CSV registered (and uploaded): a bad cap, CSV or MIPRES
    // configuration leaves nothing behind.
    let loaded = register(ctx, resolved)?;
    let nt_version = resolve_version(ctx.nt.as_ref(), None)?;
    let catalog = load_catalog(ctx.nt.as_ref(), &nt_version, distinct_cums(&csv.lines))?;
    let finished = finished_invoices(ctx, &candidate_invoices(&csv.lines, &only));
    let result = prepare(
        &csv.lines,
        &catalog,
        &PrepareOptions {
            max,
            only: &only,
            limit: request.limit,
            finished: &finished,
            automatic: &automatic,
            mipres,
        },
    )
    .map_err(|err| anyhow!("preparing convenio: {err}"))?;
    check_selection(&only, &result)?;
    if csv.skipped > 0 {
        tracing::warn!(lines = ?csv.skipped_lines, "CSV lines skipped (wrong field count)");
    }
    Ok(Plan {
        load: loaded.info,
        cargue: loaded.cargue,
        csv,
        nt_version,
        nt_entries: catalog.len(),
        max,
        automatic_subplans: subplans,
        only,
        result,
    })
}

/// Unknown `--invoices` only warn; the command fails when none matches.
fn check_selection(only: &[String], result: &PrepareResult) -> Result<()> {
    if only.is_empty() {
        return Ok(());
    }
    if !result.unknown_invoices.is_empty() {
        tracing::warn!(invoices = %result.unknown_invoices.join(","), "requested invoices not found in the CSV");
    }
    if result.unknown_invoices.len() == selected_set(only).len() {
        bail!("none of the --invoices is in the CSV");
    }
    Ok(())
}

/// The dataset of this preparation alone, before any merge with a stored one.
pub fn build_dataset(plan: &Plan) -> Dataset {
    let result = &plan.result;
    Dataset {
        cargue: plan.cargue,
        preparation: Preparation {
            load: plan.load.clone(),
            source: plan.load.name.clone(),
            max_amount: Some(plan.max),
            automatic_subplans: plan.automatic_subplans.clone(),
            nt_version: plan.nt_version.clone(),
            prepared_format: result.prepared_format,
            prepared_at: None,
            stats: result.stats.clone(),
            skipped_lines: plan.csv.skipped_lines.iter().map(|&n| n as i64).collect(),
        },
        invoices: result.invoices.clone(),
        excluded: result.excluded.clone(),
        manual_review: result.manual_review.clone(),
    }
}

/// The outcome stored for each invoice of the result, in Go's order
/// (excluded, manual review, prepared).
pub fn outcomes(result: &PrepareResult) -> Vec<(String, Outcome)> {
    let mut out = Vec::new();
    for excluded in &result.excluded {
        out.push((
            excluded.invoice.clone(),
            Outcome::excluded(&excluded.reason),
        ));
    }
    for review in &result.manual_review {
        out.push((
            review.invoice.clone(),
            Outcome::manual_review(review.text()),
        ));
    }
    for invoice in &result.invoices {
        out.push((invoice.invoice.clone(), Outcome::done()));
    }
    out
}

/// Stores the dataset (`--invoices` merges into the stored one) and then
/// publishes the per-invoice states of phase `convenio` (Go `Publish`:
/// every row is written with `intentos = 1`).
pub fn commit_preparation(ctx: &AppContext, plan: &Plan) -> Result<()> {
    let dataset = build_dataset(plan);
    let store = ctx.dataset();
    if plan.only.is_empty() {
        store.replace(&dataset, false)?;
    } else {
        store.replace_touching(&dataset, &plan.only)?;
    }
    let rows: Vec<SaveRow> = outcomes(&plan.result)
        .into_iter()
        .map(|(invoice, outcome)| SaveRow {
            invoice,
            phase: Phase::Convenio,
            outcome,
            intentos: 1,
            cargue_id: Some(plan.load.id.clone()),
        })
        .collect();
    ctx.proceso().save_all(&rows)?;
    Ok(())
}
