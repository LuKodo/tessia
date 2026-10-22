//! The reconciliation rules (CONC-05, CONC-06, CONC-09): pure functions over
//! the convenio invoice, the XML lines and the nota técnica catalog.
//!
//! Port of Go `internal/conciliacion`. Differences: amounts are exact
//! `Decimal`s (Go used `float64` and rounded to cents), which only matters on
//! exact half-cent ties.

use std::collections::{HashMap, HashSet, VecDeque};

use chrono::{DateTime, Utc};
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::{Decimal, RoundingStrategy};
use thiserror::Error;

use super::catalog::{Catalog, Item, normalize_code};
use crate::domain::dataset::{Invoice, InvoiceAudit, Line, LineAudit};
use crate::fevxml::InvoiceLine;
use crate::util::decimal::{parse_plain, round2};

/// Origin of a line updated from the XML (`LineAudit::origin`).
pub const ORIGIN_MATCHED: &str = "CSV_CONCILIADO_XML";
/// Origin of a line the convenio did not have.
pub const ORIGIN_ADDED: &str = "LINEA_AGREGADA_DESDE_XML";

/// Reconciliation states (`rips_medicar_conciliaciones.estado`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Estado {
    /// The XML lines add up to the XML total.
    Conciliada,
    /// The XML lines do not add up to the XML total.
    Diferencia,
    /// An XML price is above the nota técnica.
    Tarifa,
    /// The rebuilt total is above the per-invoice cap.
    Monto,
}

impl Estado {
    /// The name stored in the audit and printed (`CONCILIADA`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            Estado::Conciliada => "CONCILIADA",
            Estado::Diferencia => "DIFERENCIA_PERSISTE",
            Estado::Tarifa => "BLOQUEADA_TARIFA",
            Estado::Monto => "BLOQUEADA_MONTO",
        }
    }
}

/// Why an invoice cannot be analyzed (`ERROR_XML: <message>`).
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RuleError {
    /// The convenio invoice has no line to copy the patient data from.
    #[error("no hay una linea base del convenio para completar la factura")]
    SinLineaBase,
    /// An XML `<PLU>-<CUM>` is not in the nota técnica.
    #[error("el PLU-CUM {0} del XML no cruza con la nota tecnica")]
    NoCruza(String),
}

/// One XML line matched to the nota técnica.
#[derive(Debug, Clone, PartialEq)]
pub struct MatchedLine {
    pub xml: InvoiceLine,
    pub plu: String,
    pub cum: String,
    pub item: Item,
    pub price_higher: bool,
    /// Index of the convenio line it updates; `None` = added from the XML.
    pub base: Option<usize>,
}

/// The analysis of one invoice.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub state: Estado,
    pub detail: String,
    /// Rebuilt total in cents (sum of the XML line totals).
    pub total: i64,
    /// XML invoice total in cents.
    pub xml_total: i64,
    /// Cap in cents, 0 = not configured.
    pub max: i64,
    pub lines: Vec<MatchedLine>,
    /// Convenio lines no XML line confirmed.
    pub unconfirmed: Vec<usize>,
}

/// `Decimal` amount to cents, half away from zero.
pub fn cents(v: Decimal) -> i64 {
    (round2(v) * Decimal::ONE_HUNDRED).to_i64().unwrap_or(0)
}

/// Cents as a plain amount without trailing zeros (`"2350"`, `"1000.5"`), the
/// form `rips generar` parses.
pub fn money(c: i64) -> String {
    let neg = c < 0;
    let c = c.abs();
    let mut s = (c / 100).to_string();
    let frac = c % 100;
    if frac != 0 {
        let f = format!(".{frac:02}");
        s.push_str(f.trim_end_matches('0'));
    }
    if neg { format!("-{s}") } else { s }
}

/// An integer when whole, else up to four decimals.
pub fn quantity(v: Decimal) -> String {
    let whole = v.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero);
    if (v - whole).abs() < Decimal::new(1, 5) {
        return whole.trunc().to_string();
    }
    v.round_dp_with_strategy(4, RoundingStrategy::MidpointAwayFromZero)
        .normalize()
        .to_string()
}

/// Matches the XML lines against the nota técnica and the convenio lines (by
/// billing CUM, in order) and decides the state: `BLOQUEADA_MONTO` over
/// `BLOQUEADA_TARIFA` over `CONCILIADA` / `DIFERENCIA_PERSISTE`. A price is
/// higher when it exceeds the nota técnica price by more than 0.0001 (a price
/// of 0 blocks, like the PHP). `max_cents` `None` = no cap.
pub fn analyze(
    inv: &Invoice,
    xml_total: i64,
    xml: &[InvoiceLine],
    cat: &Catalog,
    max_cents: Option<i64>,
) -> Result<Analysis, RuleError> {
    if inv.lines.is_empty() {
        return Err(RuleError::SinLineaBase);
    }
    let mut queue: HashMap<String, VecDeque<usize>> = HashMap::new();
    for (i, l) in inv.lines.iter().enumerate() {
        queue
            .entry(normalize_code(&l.cum))
            .or_default()
            .push_back(i);
    }
    let tolerance = Decimal::new(1, 4);
    let mut used = HashSet::new();
    let mut lines = Vec::with_capacity(xml.len());
    let mut total = 0;
    let mut tariff = false;
    for x in xml {
        let (plu, cum, item) = cat
            .match_identifier(&x.identifier)
            .ok_or_else(|| RuleError::NoCruza(x.identifier.clone()))?;
        let contract = parse_plain(&item.price).unwrap_or(Decimal::ZERO);
        let price_higher = x.unit_price > contract + tolerance;
        tariff |= price_higher;
        let base = queue
            .get_mut(&normalize_code(&cum))
            .and_then(VecDeque::pop_front);
        if let Some(b) = base {
            used.insert(b);
        }
        total += cents(x.line_total);
        lines.push(MatchedLine {
            xml: x.clone(),
            plu,
            cum,
            item: item.clone(),
            price_higher,
            base,
        });
    }
    let unconfirmed = (0..inv.lines.len()).filter(|i| !used.contains(i)).collect();
    let max = max_cents.unwrap_or(0);
    let (state, detail) = decide(total, xml_total, max, tariff, &lines);
    Ok(Analysis {
        state,
        detail,
        total,
        xml_total,
        max,
        lines,
        unconfirmed,
    })
}

fn decide(
    total: i64,
    xml_total: i64,
    max: i64,
    tariff: bool,
    lines: &[MatchedLine],
) -> (Estado, String) {
    if max > 0 && total > max {
        return (
            Estado::Monto,
            format!(
                "Valor de factura supera lo permitido: total COP={}, maximo COP={}",
                money(total),
                money(max)
            ),
        );
    }
    if tariff {
        return (
            Estado::Tarifa,
            format!(
                "El XML fue completado, pero al menos un precio supera la nota tecnica ({}). Requiere nota credito y refacturacion; no se genera JSON/CUV ni se radica.",
                higher_prices(lines)
            ),
        );
    }
    if total == xml_total {
        return (
            Estado::Conciliada,
            "La fecha se obtiene de Medicar y las lineas del XML completan exactamente el valor de la factura."
                .to_owned(),
        );
    }
    (
        Estado::Diferencia,
        format!(
            "Las lineas XML suman COP={}, pero el total del XML es COP={}.",
            money(total),
            money(xml_total)
        ),
    )
}

fn higher_prices(lines: &[MatchedLine]) -> String {
    lines
        .iter()
        .filter(|l| l.price_higher)
        .map(|l| {
            format!(
                "{} precio XML={} nota tecnica={}",
                l.xml.identifier,
                money(cents(l.xml.unit_price)),
                l.item.price
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// Values of a reconciliation that are not in the [`Analysis`].
#[derive(Debug, Clone)]
pub struct RebuildInput<'a> {
    /// Medicar formula date (`yyyy-MM-dd[ HH:mm:ss]`).
    pub formula_date: &'a str,
    /// SSC of the first convenio line.
    pub ssc: &'a str,
    /// XML total as printed (`valor_xml`).
    pub xml_total: &'a str,
    /// Convenio total before the rebuild (`NO_DISPONIBLE` when unreadable).
    pub previous_total: &'a str,
    pub now: DateTime<Utc>,
}

/// Returns `inv` with the lines of `res`: each XML line updates its matched
/// convenio line (or a copy of the first line) with the XML values, the Medicar
/// date and the nota técnica RIPS fields. Unconfirmed lines are dropped from
/// `lines`; all the original lines are kept in the audit (the first
/// reconciliation's originals are never overwritten).
pub fn rebuild(inv: &Invoice, res: &Analysis, input: &RebuildInput<'_>) -> Invoice {
    let originals = match &inv.conciliacion {
        Some(a) if !a.original_lines.is_empty() => a.original_lines.clone(),
        _ => inv.lines.clone(),
    };
    let lines = res
        .lines
        .iter()
        .map(|l| rebuild_line(inv, l, input.formula_date))
        .collect();
    Invoice {
        invoice: inv.invoice.clone(),
        lines,
        id_mipres: inv.id_mipres.clone(),
        version: inv.version,
        conciliacion: Some(InvoiceAudit {
            state: res.state.as_str().to_owned(),
            formula_date: input.formula_date.to_owned(),
            ssc: input.ssc.to_owned(),
            xml_total: input.xml_total.to_owned(),
            previous_total: input.previous_total.to_owned(),
            reconciled_total: money(res.total),
            reconciled_at: Some(input.now),
            original_lines: originals,
            unconfirmed: res.unconfirmed.iter().map(|i| *i as i64).collect(),
        }),
    }
}

fn rebuild_line(inv: &Invoice, l: &MatchedLine, formula_date: &str) -> Line {
    let (mut base, origin) = match l.base {
        Some(i) => (inv.lines[i].clone(), ORIGIN_MATCHED),
        None => (inv.lines[0].clone(), ORIGIN_ADDED),
    };
    base.date = formula_date.to_owned();
    base.plu = l.plu.clone();
    base.cum = l.cum.clone();
    base.agreement_cum = l.cum.clone();
    base.article = l.xml.description.clone();
    base.quantity = quantity(l.xml.quantity);
    base.unit_value = money(cents(l.xml.unit_price));
    base.total_value = money(cents(l.xml.line_total));
    base.product_type = l.item.product_type.clone();
    base.pharmaceutical_form = l.item.form.clone();
    base.concentration = l.item.concentration.clone();
    base.measure_unit = l.item.unit.clone();
    base.agreed_price = l.item.price.clone();
    base.conciliacion = Some(LineAudit {
        origin: origin.to_owned(),
        xml_identifier: l.xml.identifier.clone(),
        date_source: "MEDICAR".to_owned(),
        value_source: "XML".to_owned(),
    });
    base
}
