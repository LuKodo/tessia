//! The prepared convenio dataset (port of Go `convenio.Dataset`, `Invoice`,
//! `Line`, `Excluded`, `ManualReview`, `Stats`).
//!
//! `convenio preparar` produces it, the other phases read it, `conciliar`
//! rewrites invoices of it. It lives in Postgres ([`crate::stores::dataset`]);
//! this module only holds the pure types.
//!
//! # Shape
//!
//! ```text
//! Dataset { cargue (cargues.seq), preparation: Preparation, invoices, excluded, manual_review }
//! Preparation { load, nt_version, max_amount, automatic_subplans, prepared_format, stats, .. }
//! Invoice { invoice, lines: Vec<Line>, conciliacion, id_mipres, version }
//! ```
//!
//! * [`Line`] keeps **exactly** the Go text fields (`quantity`, `unit_value`,
//!   `total_value`, ... are the verbatim, trimmed CSV strings such as
//!   `"1.234,56"`) because RIPS, soporte and conciliar print or compare them
//!   as written. Use the `*_amount()` accessors to get a [`Decimal`]
//!   ([`parse_agreement_amount`](crate::util::decimal::parse_agreement_amount)
//!   rules). Columns a later phase adds without a typed field travel in
//!   [`Line::extra`].
//! * [`Invoice::version`] is the optimistic-locking version the store returned
//!   when the invoice was read (`0` on freshly built invoices and ignored by
//!   `replace`); pass it back to `update_invoices`.
//! * Invoice names are the normalized key
//!   ([`normalize_invoice`](crate::domain::normalize_invoice)).
//!
//! ```
//! use tessia::domain::dataset::{Invoice, Line};
//! let mut line = Line::default();
//! line.invoice = "EVS1".into();
//! line.total_value = "1.234,50".into();
//! let inv = Invoice::new("EVS1").with_line(line);
//! assert_eq!(inv.agreement_total().unwrap().to_string(), "1234.50");
//! ```

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::normalize_invoice;
use crate::util::decimal::{AmountError, parse_agreement_amount, sum};

pub use crate::domain::cargue::{LoadInfo, LoadOrigin};

/// Stored state of an invoice in the dataset (`dataset_factura.estado`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum InvoiceState {
    /// `PREPARADA`: in [`Dataset::invoices`], with lines.
    Prepared,
    /// `EXCLUIDA`: in [`Dataset::excluded`].
    Excluded,
    /// `REVISION_MANUAL`: in [`Dataset::manual_review`].
    ManualReview,
}

impl InvoiceState {
    /// The stored text (Go `estadoPrepared` ...).
    pub fn as_str(self) -> &'static str {
        match self {
            InvoiceState::Prepared => "PREPARADA",
            InvoiceState::Excluded => "EXCLUIDA",
            InvoiceState::ManualReview => "REVISION_MANUAL",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "PREPARADA" => Some(InvoiceState::Prepared),
            "EXCLUIDA" => Some(InvoiceState::Excluded),
            "REVISION_MANUAL" => Some(InvoiceState::ManualReview),
            _ => None,
        }
    }
}

/// One dispensed item plus the enrichment `convenio preparar` adds
/// (Go `convenio.Line`). All text fields are trimmed; an absent value is `""`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Line {
    /// 1-based line number in the source CSV.
    pub source_line: i32,
    pub nit: String,
    pub plan: String,
    pub subplan: String,
    /// Normalized invoice (upper case, no dashes).
    pub invoice: String,
    /// Invoice as exported.
    pub invoice_raw: String,
    pub doc_type: String,
    pub doc_number: String,
    pub beneficiary: String,
    pub point_of_sale: String,
    pub formula_number: String,
    pub ssc: String,
    pub formula_type: String,
    pub date: String,
    pub lot: String,
    pub plu: String,
    pub article: String,
    pub quantity: String,
    pub unit_value: String,
    pub unit_cost: String,
    /// "Vlr. total" text, summed by the agreement total.
    pub total_value: String,
    pub copay: String,
    pub authorization: String,
    pub mipres: String,
    /// CUM as exported by the convenio.
    pub agreement_cum: String,
    pub diagnosis_code: String,
    pub diagnosis_name: String,
    pub causal_exento: String,
    pub ips: String,
    pub prescriber_id: String,
    pub prescriber_name: String,
    pub gender: String,
    pub birth_date: String,
    pub department: String,
    pub municipality: String,
    /// The file's own CLASIFICACION column (prepared-format exports only).
    pub classification: String,
    /// Billing CUM (nota técnica); `agreement_cum` until enriched.
    pub cum: String,
    /// `"714"` medicine, `"799"` other service.
    pub product_type: String,
    pub pharmaceutical_form: String,
    pub concentration: String,
    pub measure_unit: String,
    pub agreed_price: String,
    /// Set on lines rebuilt by `conciliar`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conciliacion: Option<LineAudit>,
    /// Anything else a phase wants to keep on the line (no typed field).
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl Line {
    /// `unit_value` as an amount (`1.234,56` or `1234.56`).
    pub fn unit_value_amount(&self) -> Result<Decimal, AmountError> {
        parse_agreement_amount(&self.unit_value)
    }

    /// `total_value` as an amount.
    pub fn total_value_amount(&self) -> Result<Decimal, AmountError> {
        parse_agreement_amount(&self.total_value)
    }

    /// `copay` as an amount.
    pub fn copay_amount(&self) -> Result<Decimal, AmountError> {
        parse_agreement_amount(&self.copay)
    }

    /// `agreed_price` as an amount.
    pub fn agreed_price_amount(&self) -> Result<Decimal, AmountError> {
        parse_agreement_amount(&self.agreed_price)
    }

    /// Go `AffiliateKey`: `"<DOCTYPE>|<number>"`, upper-cased and trimmed.
    pub fn affiliate_key(&self) -> String {
        format!(
            "{}|{}",
            self.doc_type.trim().to_uppercase(),
            self.doc_number.trim()
        )
    }
}

/// Origin of a line rebuilt by `conciliar` (Go `LineAudit`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LineAudit {
    /// `CSV_CONCILIADO_XML` or `LINEA_AGREGADA_DESDE_XML`.
    pub origin: String,
    pub xml_identifier: String,
    /// `MEDICAR`.
    pub date_source: String,
    /// `XML`.
    pub value_source: String,
}

/// Evidence of a reconciliation (Go `InvoiceAudit`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InvoiceAudit {
    pub state: String,
    pub formula_date: String,
    pub ssc: String,
    pub xml_total: String,
    pub previous_total: String,
    pub reconciled_total: String,
    pub reconciled_at: Option<DateTime<Utc>>,
    /// Lines before the first reconciliation.
    pub original_lines: Vec<Line>,
    /// Indexes of original lines no XML line confirmed.
    pub unconfirmed: Vec<i64>,
}

/// A prepared invoice and its lines (Go `convenio.Invoice`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Invoice {
    /// Normalized invoice number.
    pub invoice: String,
    pub lines: Vec<Line>,
    /// Reconciliation evidence (set by `conciliar`).
    pub conciliacion: Option<InvoiceAudit>,
    /// MIPRES delivery ids joined by `", "` (EVENTO NO PBS only).
    pub id_mipres: String,
    /// Optimistic-locking version as read from the store (`0` when built in
    /// memory). `replace` ignores it; `update_invoices` takes it as the
    /// expected version.
    pub version: i64,
}

impl Invoice {
    pub fn new(invoice: &str) -> Self {
        Self {
            invoice: invoice.to_owned(),
            ..Self::default()
        }
    }

    #[must_use]
    pub fn with_line(mut self, line: Line) -> Self {
        self.lines.push(line);
        self
    }

    /// Sum of the line totals (Go `PublishAgreementTotals` rules: any line
    /// with an unreadable total fails). An invoice without lines totals `0`.
    pub fn agreement_total(&self) -> Result<Decimal, AmountError> {
        sum_line_totals(self.lines.iter().map(|l| l.total_value.as_str()))
    }

    /// Go `subplanOf`: the first non-blank subplan of the lines.
    pub fn subplan(&self) -> &str {
        self.lines
            .iter()
            .map(|l| l.subplan.as_str())
            .find(|s| !s.trim().is_empty())
            .unwrap_or("")
    }
}

/// Sums line total texts as the agreement total does.
pub fn sum_line_totals<'a>(
    totals: impl IntoIterator<Item = &'a str>,
) -> Result<Decimal, AmountError> {
    let mut parsed = Vec::new();
    for text in totals {
        parsed.push(parse_agreement_amount(text)?);
    }
    Ok(sum(parsed))
}

/// An invoice removed by preparation; `reason` is the legacy observation text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Excluded {
    pub invoice: String,
    pub reason: String,
}

/// An invoice held for a person to review.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ManualReview {
    pub invoice: String,
    pub code: String,
    pub reason: String,
}

impl ManualReview {
    /// The stored message: `"<code>: <reason>"`.
    pub fn text(&self) -> String {
        format!("{}: {}", self.code, self.reason)
    }
}

/// Summary counters of a preparation (Go `Stats`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    pub medicines: i64,
    pub other_services: i64,
    pub copay_lines: i64,
    pub dropped_no_invoice: i64,
    pub ignored: i64,
    pub finished: i64,
    pub evaluated: i64,
    pub prepared_pending: i64,
    pub limit: i64,
    pub mipres_unavailable: i64,
}

/// The header of a dataset (Go `Dataset` minus the invoice lists, plus what
/// the Rust port pins: `nt_version`, `automatic_subplans`, `skipped_lines`).
#[derive(Debug, Clone, PartialEq)]
pub struct Preparation {
    /// Identity of the load (`load.id` is the cargue id, e.g.
    /// `Convenio_21102026_21102026`).
    pub load: LoadInfo,
    /// Source file name (Go `archivo_fuente`).
    pub source: String,
    /// Per-invoice cap used (Go `monto_maximo_factura`); `None` = unset, the
    /// reader falls back to `parametros`.
    pub max_amount: Option<Decimal>,
    /// Automatic subplan allowlist used by the preparation.
    pub automatic_subplans: Vec<String>,
    /// Redis NT version pinned at prepare time (replaces Go's
    /// `NotaTecnicaVersionID`); `""` when none.
    pub nt_version: String,
    /// A manual load already in the legacy "prepared" format (some line with
    /// CLASIFICACION 714/799): `derechos validar` takes the rights from the file.
    pub prepared_format: bool,
    /// When the dataset was stored; `None` on write means "now" (database
    /// clock). Always `Some` on read.
    pub prepared_at: Option<DateTime<Utc>>,
    pub stats: Stats,
    /// 1-based CSV line numbers skipped by the reader (wrong field count).
    pub skipped_lines: Vec<i64>,
}

impl Default for Preparation {
    fn default() -> Self {
        Self {
            load: LoadInfo {
                name: String::new(),
                id: String::new(),
                from: None,
                to: None,
                origin: LoadOrigin::Unknown,
            },
            source: String::new(),
            max_amount: None,
            automatic_subplans: Vec::new(),
            nt_version: String::new(),
            prepared_format: false,
            prepared_at: None,
            stats: Stats::default(),
            skipped_lines: Vec::new(),
        }
    }
}

/// The prepared dataset of one cargue.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Dataset {
    /// `cargues.seq` of the upload this dataset belongs to.
    pub cargue: i64,
    pub preparation: Preparation,
    /// Prepared invoices (with lines), in stored order.
    pub invoices: Vec<Invoice>,
    pub excluded: Vec<Excluded>,
    pub manual_review: Vec<ManualReview>,
}

impl Dataset {
    /// Names of every invoice of the dataset (prepared, excluded, manual).
    pub fn all_names(&self) -> Vec<String> {
        self.invoices
            .iter()
            .map(|i| i.invoice.clone())
            .chain(self.excluded.iter().map(|e| e.invoice.clone()))
            .chain(self.manual_review.iter().map(|m| m.invoice.clone()))
            .collect()
    }

    /// The prepared invoice called `name` (normalized before comparing).
    pub fn invoice(&self, name: &str) -> Option<&Invoice> {
        let key = normalize_invoice(name);
        self.invoices.iter().find(|i| i.invoice == key)
    }

    /// Go `FilterDataset`: only the named invoices (normalized), header kept;
    /// an empty list keeps the header only.
    #[must_use]
    pub fn filtered(&self, names: &[String]) -> Dataset {
        let want: HashSet<String> = subset_names(names).into_iter().collect();
        Dataset {
            cargue: self.cargue,
            preparation: self.preparation.clone(),
            invoices: self
                .invoices
                .iter()
                .filter(|i| want.contains(&i.invoice))
                .cloned()
                .collect(),
            excluded: self
                .excluded
                .iter()
                .filter(|e| want.contains(&e.invoice))
                .cloned()
                .collect(),
            manual_review: self
                .manual_review
                .iter()
                .filter(|m| want.contains(&m.invoice))
                .cloned()
                .collect(),
        }
    }
}

/// Normalized, distinct, non-blank names of a subset request, in first-seen
/// order (Go `subsetNames`, without its "nil means everything" rule: callers
/// pass `Option<&[String]>`).
pub fn subset_names(names: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    names
        .iter()
        .map(|n| normalize_invoice(n))
        .filter(|n| !n.is_empty() && seen.insert(n.clone()))
        .collect()
}

/// Builders for synthetic datasets in tests (feature `test-support` / unit
/// tests). Every other phase needs them; keep them tiny and obvious.
///
/// ```
/// use tessia::domain::dataset::test_support::{DatasetBuilder, InvoiceBuilder};
/// let ds = DatasetBuilder::new(1)
///     .load("Convenio_21102026_21102026")
///     .nt_version("2026-10-21")
///     .invoice(InvoiceBuilder::new("EVS1").line("10.50").line("5"))
///     .excluded("EVS2", "VALOR_SUPERA_MONTO")
///     .manual_review("EVS3", "MULTIPLES_AFILIADOS_EN_FACTURA", "dos afiliados")
///     .build();
/// assert_eq!(ds.all_names(), ["EVS1", "EVS2", "EVS3"]);
/// assert_eq!(ds.invoices[0].agreement_total().unwrap().to_string(), "15.50");
/// ```
#[cfg(any(test, feature = "test-support"))]
pub mod test_support {
    use super::*;
    use crate::domain::cargue::load_context;

    /// A plausible line of `invoice`: pharmacy medicine, one unit, `total` as
    /// both unit and total value. `n` is the 1-based CSV line number.
    pub fn sample_line(invoice: &str, n: i32, total: &str) -> Line {
        Line {
            source_line: n,
            nit: "900123456".into(),
            plan: "PLAN".into(),
            subplan: "EVENTO POS".into(),
            invoice: invoice.to_owned(),
            invoice_raw: invoice.to_owned(),
            doc_type: "CC".into(),
            doc_number: "1000000001".into(),
            beneficiary: "PACIENTE DE PRUEBA".into(),
            date: "2026-10-21".into(),
            plu: format!("PLU{n}"),
            article: format!("ARTICULO {n}"),
            quantity: "1".into(),
            unit_value: total.to_owned(),
            total_value: total.to_owned(),
            copay: "0".into(),
            agreement_cum: format!("1990000{n}-1"),
            cum: format!("1990000{n}-1"),
            product_type: "714".into(),
            ..Line::default()
        }
    }

    /// Fluent [`Invoice`] builder.
    pub struct InvoiceBuilder {
        invoice: Invoice,
    }

    impl InvoiceBuilder {
        pub fn new(name: &str) -> Self {
            Self {
                invoice: Invoice::new(&normalize_invoice(name)),
            }
        }

        /// Appends a [`sample_line`] with this total value.
        #[must_use]
        pub fn line(mut self, total_value: &str) -> Self {
            let n = i32::try_from(self.invoice.lines.len() + 1).unwrap_or(i32::MAX);
            let name = self.invoice.invoice.clone();
            self.invoice.lines.push(sample_line(&name, n, total_value));
            self
        }

        /// Appends your own line (its `invoice` is forced to this invoice).
        #[must_use]
        pub fn with_line(mut self, mut line: Line) -> Self {
            line.invoice = self.invoice.invoice.clone();
            self.invoice.lines.push(line);
            self
        }

        #[must_use]
        pub fn mipres(mut self, ids: &str) -> Self {
            self.invoice.id_mipres = ids.to_owned();
            self
        }

        #[must_use]
        pub fn audit(mut self, audit: InvoiceAudit) -> Self {
            self.invoice.conciliacion = Some(audit);
            self
        }

        pub fn build(self) -> Invoice {
            self.invoice
        }
    }

    impl From<InvoiceBuilder> for Invoice {
        fn from(b: InvoiceBuilder) -> Self {
            b.build()
        }
    }

    /// Fluent [`Dataset`] builder. `new(cargue_seq)` takes the `cargues.seq`
    /// the dataset will be stored under (see
    /// `stores::dataset::test_support::seed_cargue`; any number works for pure
    /// in-memory tests).
    pub struct DatasetBuilder {
        ds: Dataset,
    }

    impl DatasetBuilder {
        pub fn new(cargue_seq: i64) -> Self {
            let mut ds = Dataset {
                cargue: cargue_seq,
                ..Dataset::default()
            };
            ds.preparation.load = load_context("Convenio_21102026_21102026.csv");
            ds.preparation.source = ds.preparation.load.name.clone();
            ds.preparation.automatic_subplans = vec![
                "EVENTO POS".into(),
                "EVENTO NO PBS".into(),
                "TRANSFERENCIA DE PENDIENTES CON AUTORIZACION".into(),
            ];
            Self { ds }
        }

        /// Sets the load identity from a file stem or name
        /// (`"Convenio_21102026_21102026"`).
        #[must_use]
        pub fn load(mut self, file_name: &str) -> Self {
            self.ds.preparation.load = load_context(file_name);
            self.ds.preparation.source = self.ds.preparation.load.name.clone();
            self
        }

        #[must_use]
        pub fn nt_version(mut self, version: &str) -> Self {
            self.ds.preparation.nt_version = version.to_owned();
            self
        }

        #[must_use]
        pub fn max_amount(mut self, amount: Decimal) -> Self {
            self.ds.preparation.max_amount = Some(amount);
            self
        }

        #[must_use]
        pub fn prepared_format(mut self, on: bool) -> Self {
            self.ds.preparation.prepared_format = on;
            self
        }

        #[must_use]
        pub fn prepared_at(mut self, at: DateTime<Utc>) -> Self {
            self.ds.preparation.prepared_at = Some(at);
            self
        }

        #[must_use]
        pub fn stats(mut self, stats: Stats) -> Self {
            self.ds.preparation.stats = stats;
            self
        }

        #[must_use]
        pub fn invoice(mut self, invoice: impl Into<Invoice>) -> Self {
            self.ds.invoices.push(invoice.into());
            self
        }

        #[must_use]
        pub fn excluded(mut self, invoice: &str, reason: &str) -> Self {
            self.ds.excluded.push(Excluded {
                invoice: normalize_invoice(invoice),
                reason: reason.to_owned(),
            });
            self
        }

        #[must_use]
        pub fn manual_review(mut self, invoice: &str, code: &str, reason: &str) -> Self {
            self.ds.manual_review.push(ManualReview {
                invoice: normalize_invoice(invoice),
                code: code.to_owned(),
                reason: reason.to_owned(),
            });
            self
        }

        pub fn build(self) -> Dataset {
            self.ds
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    #[test]
    fn line_amounts_use_the_agreement_rules() {
        let mut line = sample_line("EVS1", 1, "1.234,50");
        assert_eq!(line.total_value_amount().unwrap().to_string(), "1234.50");
        line.total_value = "abc".into();
        assert!(line.total_value_amount().is_err());
    }

    #[test]
    fn agreement_total_sums_lines_and_fails_on_a_bad_one() {
        let ok = InvoiceBuilder::new("evs-1")
            .line("10.50")
            .line("0,25")
            .build();
        assert_eq!(ok.invoice, "EVS1");
        assert_eq!(ok.agreement_total().unwrap().to_string(), "10.75");
        assert_eq!(
            Invoice::new("EVS2").agreement_total().unwrap(),
            Decimal::ZERO
        );
        let bad = InvoiceBuilder::new("EVS3").line("10").line("x").build();
        assert!(bad.agreement_total().is_err());
    }

    #[test]
    fn filtered_keeps_the_header_and_normalizes_names() {
        let ds = DatasetBuilder::new(1)
            .invoice(InvoiceBuilder::new("EVS1").line("1"))
            .invoice(InvoiceBuilder::new("EVS2").line("2"))
            .excluded("EVS3", "X")
            .build();
        let only = ds.filtered(&["evs-2".into(), "EVS3".into(), "EVS2".into()]);
        assert_eq!(only.all_names(), ["EVS2", "EVS3"]);
        assert_eq!(only.preparation, ds.preparation);
        assert!(ds.filtered(&[]).all_names().is_empty());
    }

    #[test]
    fn line_json_round_trips_with_extra_fields() {
        let mut line = sample_line("EVS1", 1, "5");
        line.extra
            .insert("futuro".into(), serde_json::json!({"a": 1}));
        line.conciliacion = Some(LineAudit {
            origin: "CSV_CONCILIADO_XML".into(),
            ..LineAudit::default()
        });
        let json = serde_json::to_value(&line).unwrap();
        assert_eq!(json["futuro"]["a"], 1);
        let back: Line = serde_json::from_value(json).unwrap();
        assert_eq!(back, line);
    }

    #[test]
    fn states_round_trip_their_text() {
        for s in [
            InvoiceState::Prepared,
            InvoiceState::Excluded,
            InvoiceState::ManualReview,
        ] {
            assert_eq!(InvoiceState::parse(s.as_str()), Some(s));
        }
        assert_eq!(InvoiceState::parse("otra"), None);
    }
}
