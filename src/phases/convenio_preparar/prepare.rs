//! The per-invoice rule engine (port of Go `convenio/prepare.go`).
//!
//! Pure over its inputs: lines, catalog, cap, selection, limit, the set of
//! finished invoices and an optional MIPRES checker. The rules run in this
//! order (first failure wins; the order is Go's code, which the docs list
//! slightly differently):
//!
//! 1. excluded when its subplan is not automatic and it is not named in
//!    `--invoices`;
//! 2. held for review when its lines name more than one affiliate
//!    (`MULTIPLES_AFILIADOS_EN_FACTURA`);
//! 3. excluded when the total is unparseable or above the cap;
//! 4. excluded when a line is not in the catalog, or its unit price is above
//!    the agreed price;
//! 5. excluded when a matched line has no 714/799 classification;
//! 6. EVENTO NO PBS only: the MIPRES check ([`super::mipres`]).
//!
//! `--limit N` counts only PREPARED pending invoices: excluded and held
//! invoices are replaced by the next pending one, finished invoices never
//! consume it, and the walk stops once N are prepared (the invoices not
//! reached are `ignored` and never evaluated).

use std::collections::{HashMap, HashSet};

use rust_decimal::Decimal;

use super::amount::{format_decimal2, parse_loose_number};
use super::catalog::{Catalog, generic_code_from_plu};
use super::csv::has_prepared_format;
use super::mipres::{MipresChecker, MipresError, MipresRun, Verdict, is_evento_no_pbs};
use super::subplan::{AutomaticSubplans, selected_set, subplan_of};
use crate::domain::dataset::{Excluded, Invoice, Line, ManualReview, Stats};
use crate::util::decimal::parse_agreement_amount;

/// Code of an invoice whose lines name more than one affiliate.
pub const CODE_MULTIPLE_AFFILIATES: &str = "MULTIPLES_AFILIADOS_EN_FACTURA";

/// Outcome of [`prepare`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrepareResult {
    pub invoices: Vec<Invoice>,
    pub excluded: Vec<Excluded>,
    pub manual_review: Vec<ManualReview>,
    pub stats: Stats,
    /// Requested invoices absent from the CSV.
    pub unknown_invoices: Vec<String>,
    /// Pending invoices (CSV order) the limit stopped the walk before.
    pub remaining: Vec<String>,
    /// The whole CSV is a manual prepared-format load.
    pub prepared_format: bool,
}

/// Inputs of [`prepare`] besides the lines and the catalog.
pub struct PrepareOptions<'a> {
    /// Per-invoice cap (exact).
    pub max: Decimal,
    /// `--invoices`: only these (also forces them past the subplan filter).
    pub only: &'a [String],
    /// `--limit` (0 = all).
    pub limit: usize,
    /// Invoices already filed or closed with a note.
    pub finished: &'a HashSet<String>,
    pub automatic: &'a AutomaticSubplans,
    /// `None` skips the MIPRES check.
    pub mipres: Option<&'a dyn MipresChecker>,
}

/// Validates and enriches the convenio per invoice. An `Err` is a systemic
/// MIPRES failure: the result would be partial and must not be used.
pub fn prepare(
    lines: &[Line],
    catalog: &Catalog,
    options: &PrepareOptions<'_>,
) -> Result<PrepareResult, MipresError> {
    let mut result = PrepareResult {
        prepared_format: has_prepared_format(lines),
        ..PrepareResult::default()
    };
    let selected = selected_set(options.only);
    let (groups, order) = group_lines(lines, &selected, &mut result);
    if !selected.is_empty() {
        note_selection(lines, &selected, &groups, &mut result);
    }
    if options.limit > 0 {
        result.stats.limit = to_i64(options.limit);
    }
    let interrupted = crate::shutdown::requested;
    let mut mipres = options
        .mipres
        .map(|checker| MipresRun::new(checker, &interrupted));
    let context = Context {
        catalog,
        options,
        prepared_format: result.prepared_format,
    };

    for (position, invoice) in order.iter().enumerate() {
        if options.limit > 0 && result.stats.prepared_pending >= to_i64(options.limit) {
            result.stats.ignored += to_i64(order.len() - position);
            result.remaining.extend(
                order[position..]
                    .iter()
                    .filter(|rest| !options.finished.contains(*rest))
                    .cloned(),
            );
            break;
        }
        let finished = options.finished.contains(invoice);
        if finished {
            result.stats.finished += 1;
        } else {
            result.stats.evaluated += 1;
        }
        let invoice_lines = &groups[invoice.as_str()];
        let forced = selected.contains(invoice);
        let outcome = evaluate(&context, invoice, invoice_lines, forced, &mut mipres)?;
        if apply(outcome, invoice, &mut result) && !finished {
            result.stats.prepared_pending += 1;
        }
    }
    Ok(result)
}

/// Groups the lines by invoice in first-appearance order; lines without an
/// invoice are dropped (counted), others outside `--invoices` are skipped.
fn group_lines<'a>(
    lines: &'a [Line],
    selected: &HashSet<String>,
    result: &mut PrepareResult,
) -> (HashMap<&'a str, Vec<&'a Line>>, Vec<String>) {
    let mut groups: HashMap<&str, Vec<&Line>> = HashMap::new();
    let mut order = Vec::new();
    for line in lines {
        if line.invoice.is_empty() {
            result.stats.dropped_no_invoice += 1;
            continue;
        }
        if !selected.is_empty() && !selected.contains(&line.invoice) {
            continue;
        }
        if !groups.contains_key(line.invoice.as_str()) {
            order.push(line.invoice.clone());
        }
        groups.entry(line.invoice.as_str()).or_default().push(line);
    }
    (groups, order)
}

/// With `--invoices`: counts the invoices left out and lists the unknown ones.
fn note_selection(
    lines: &[Line],
    selected: &HashSet<String>,
    groups: &HashMap<&str, Vec<&Line>>,
    result: &mut PrepareResult,
) {
    let all: HashSet<&str> = lines
        .iter()
        .map(|line| line.invoice.as_str())
        .filter(|invoice| !invoice.is_empty())
        .collect();
    result.stats.ignored += to_i64(
        all.iter()
            .filter(|invoice| !selected.contains(**invoice))
            .count(),
    );
    let mut unknown: Vec<String> = selected
        .iter()
        .filter(|invoice| !groups.contains_key(invoice.as_str()))
        .cloned()
        .collect();
    unknown.sort();
    result.unknown_invoices = unknown;
}

#[allow(clippy::large_enum_variant)]
enum Outcome {
    Prepared(Invoice),
    Excluded(String),
    Review(ManualReview),
    /// Held because MIPRES was not available.
    Unavailable(ManualReview),
}

/// Stores the outcome in the result; true when the invoice was prepared.
fn apply(outcome: Outcome, invoice: &str, result: &mut PrepareResult) -> bool {
    match outcome {
        Outcome::Excluded(reason) => result.excluded.push(Excluded {
            invoice: invoice.to_owned(),
            reason,
        }),
        Outcome::Review(review) => result.manual_review.push(review),
        Outcome::Unavailable(review) => {
            result.manual_review.push(review);
            result.stats.mipres_unavailable += 1;
        }
        Outcome::Prepared(prepared) => {
            count_lines(&prepared.lines, &mut result.stats);
            result.invoices.push(prepared);
            return true;
        }
    }
    false
}

fn to_i64(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn count_lines(lines: &[Line], stats: &mut Stats) {
    for line in lines {
        match line.product_type.as_str() {
            "714" => stats.medicines += 1,
            "799" => stats.other_services += 1,
            _ => {}
        }
        if parse_loose_number(&line.copay) > 0.0 {
            stats.copay_lines += 1;
        }
    }
}

/// What every invoice evaluation shares.
struct Context<'a> {
    catalog: &'a Catalog,
    options: &'a PrepareOptions<'a>,
    prepared_format: bool,
}

fn evaluate(
    context: &Context<'_>,
    invoice: &str,
    lines: &[&Line],
    forced: bool,
    mipres: &mut Option<MipresRun<'_>>,
) -> Result<Outcome, MipresError> {
    let subplan = subplan_of(lines.iter().copied());
    if !forced && !context.options.automatic.allows(&subplan) {
        let shown = if subplan.is_empty() {
            "vacío"
        } else {
            &subplan
        };
        return Ok(Outcome::Excluded(format!(
            "Subplan '{shown}' no se radica automaticamente; use --invoices para forzarlo."
        )));
    }
    let affiliates = distinct_affiliates(lines);
    if affiliates > 1 {
        return Ok(Outcome::Review(ManualReview {
            invoice: invoice.to_owned(),
            code: CODE_MULTIPLE_AFFILIATES.to_owned(),
            reason: format!(
                "Factura con lineas de mas de un afiliado ({affiliates} documentos distintos); revisar en Medicar."
            ),
        }));
    }
    if let Some(reason) = validate_invoice(lines, context.catalog, context.options.max) {
        return Ok(Outcome::Excluded(reason));
    }
    let enriched = enrich(lines, context.catalog, context.prepared_format);
    if let Some(reason) = classification_gap(&enriched) {
        return Ok(Outcome::Excluded(reason));
    }
    let mut id_mipres = String::new();
    if let Some(run) = mipres.as_mut().filter(|_| is_evento_no_pbs(&subplan)) {
        let verdict = run.check_invoice(invoice, &enriched);
        if let Some(fatal) = run.fatal() {
            return Err(fatal.clone());
        }
        match verdict {
            Verdict::Pass(ids) => id_mipres = ids,
            Verdict::Exclude(reason) => return Ok(Outcome::Excluded(reason)),
            Verdict::Hold {
                review,
                unavailable: true,
            } => return Ok(Outcome::Unavailable(review)),
            Verdict::Hold { review, .. } => return Ok(Outcome::Review(review)),
        }
    }
    Ok(Outcome::Prepared(Invoice {
        lines: enriched,
        id_mipres,
        ..Invoice::new(invoice)
    }))
}

fn distinct_affiliates(lines: &[&Line]) -> usize {
    lines
        .iter()
        .map(|line| {
            format!(
                "{}|{}",
                line.doc_type.trim().to_uppercase(),
                line.doc_number.trim()
            )
        })
        .collect::<HashSet<_>>()
        .len()
}

fn validate_invoice(lines: &[&Line], catalog: &Catalog, max: Decimal) -> Option<String> {
    if let Some(reason) = validate_amount(lines, max) {
        return Some(reason);
    }
    for line in lines {
        let code = generic_code_from_plu(&line.plu);
        let Some(item) = catalog.lookup(&code, &line.agreement_cum) else {
            return Some(format!(
                "No cruza nota tecnica: PLU={}, codigoGenerico={}, CUM convenio={}",
                line.plu, code, line.agreement_cum
            ));
        };
        if let Some(reason) = validate_agreed_price(&line.unit_value, &item.price) {
            return Some(reason);
        }
    }
    None
}

/// `validarMontoFactura`: the sum of the totals against the cap (exact).
fn validate_amount(lines: &[&Line], max: Decimal) -> Option<String> {
    let mut total = Decimal::ZERO;
    for line in lines {
        match parse_agreement_amount(&line.total_value) {
            Ok(amount) => total += amount,
            Err(_) => return Some("Factura excluida: valor total ausente o invalido".to_owned()),
        }
    }
    (total > max).then(|| format!("MONTO_MAXIMO_SUPERADO: total COP={total}, maximo COP={max}"))
}

/// `validarPrecioPactado`: only when the agreed price is above 0.
fn validate_agreed_price(unit_value: &str, agreed: &str) -> Option<String> {
    let price = parse_loose_number(unit_value);
    let agreed_price = parse_loose_number(agreed);
    if agreed_price <= 0.0 || price <= agreed_price + 0.0001 {
        return None;
    }
    Some(format!(
        "PRECIO_MAYOR_NOTA_TECNICA: precio convenio COP={}, precio pactado COP={}. Requiere nota credito y refacturacion; no se genera JSON ni se radica.",
        format_decimal2(price),
        format_decimal2(agreed_price)
    ))
}

fn enrich(lines: &[&Line], catalog: &Catalog, prepared_format: bool) -> Vec<Line> {
    lines
        .iter()
        .map(|source| {
            let mut line = (*source).clone();
            let item = catalog
                .lookup(&generic_code_from_plu(&line.plu), &line.agreement_cum)
                .cloned()
                .unwrap_or_default();
            line.cum = item.cum;
            line.product_type = item.product_type;
            if prepared_format {
                // The file's own drug fields win; the catalog back-fills blanks.
                fill_blank(&mut line.pharmaceutical_form, item.form);
                fill_blank(&mut line.concentration, item.concentration);
                fill_blank(&mut line.measure_unit, item.unit);
            } else {
                line.pharmaceutical_form = item.form;
                line.concentration = item.concentration;
                line.measure_unit = item.unit;
            }
            line.agreed_price = item.price;
            line
        })
        .collect()
}

fn fill_blank(current: &mut String, fallback: String) {
    if current.trim().is_empty() {
        *current = fallback;
    }
}

fn classification_gap(lines: &[Line]) -> Option<String> {
    lines
        .iter()
        .find(|line| line.product_type != "714" && line.product_type != "799")
        .map(|line| {
            format!(
                "SIN_CLASIFICACION_NOTA_TECNICA: la nota tecnica no alimento CLASIFICACION 714/799. PLU={}, CUM={}, clasificacion={}",
                line.plu, line.cum, line.product_type
            )
        })
}
