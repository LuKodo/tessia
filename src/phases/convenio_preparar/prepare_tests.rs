//! Executable spec of the rule engine: ports of Go `prepare_test.go` and
//! `mipres_test.go` (same synthetic data, same expected strings).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use rust_decimal::Decimal;

use super::catalog::{Catalog, Item};
use super::mipres::{
    CODE_MIPRES_NO_VERIFICADO, Delivery, MipresChecker, MipresError, REASON_MIPRES_SIN_ENTREGA,
    needs_mipres,
};
use super::prepare::{CODE_MULTIPLE_AFFILIATES, PrepareOptions, PrepareResult, prepare};
use super::subplan::AutomaticSubplans;
use crate::domain::dataset::Line;
use crate::domain::normalize_invoice;
use crate::util::decimal::parse_max_amount;

fn test_catalog() -> Catalog {
    let mut catalog = Catalog::new("v-test");
    let mut add = |code: &str, cum: &str, product_type: &str, price: &str, rips: [&str; 3]| {
        catalog.insert(Item {
            cum: cum.into(),
            product_type: product_type.into(),
            generic_code: code.into(),
            price: price.into(),
            form: rips[0].into(),
            concentration: rips[1].into(),
            unit: rips[2].into(),
        });
    };
    add(
        "20010",
        "19950404-1",
        "714",
        "1500",
        ["TABLETA", "500", "MG"],
    );
    add("30020", "12345", "799", "0", ["", "", ""]);
    add("40040", "55555", "ABC", "0", ["", "", ""]);
    catalog
}

fn line(invoice: &str, plu: &str, cum: &str, unit: &str, total: &str) -> Line {
    Line {
        invoice: normalize_invoice(invoice),
        invoice_raw: invoice.into(),
        plu: plu.into(),
        agreement_cum: cum.into(),
        cum: cum.into(),
        unit_value: unit.into(),
        total_value: total.into(),
        subplan: "EVENTO POS".into(),
        ..Line::default()
    }
}

fn line_sub(invoice: &str, subplan: &str) -> Line {
    let mut l = line(invoice, "20010_01", "19950404-1", "1500,00", "1500");
    l.subplan = subplan.into();
    l
}

fn with_doc(mut l: Line, doc_type: &str, number: &str) -> Line {
    l.doc_type = doc_type.into();
    l.doc_number = number.into();
    l
}

fn max(text: &str) -> Decimal {
    parse_max_amount(text).expect("max")
}

struct Run<'a> {
    max: Decimal,
    only: Vec<String>,
    limit: usize,
    finished: HashSet<String>,
    automatic: AutomaticSubplans,
    mipres: Option<&'a dyn MipresChecker>,
}

impl Run<'_> {
    fn new(max_text: &str) -> Self {
        Run {
            max: max(max_text),
            only: vec![],
            limit: 0,
            finished: HashSet::new(),
            automatic: AutomaticSubplans::defaults(),
            mipres: None,
        }
    }

    fn only(mut self, only: &[&str]) -> Self {
        self.only = only.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    fn limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    fn finished(mut self, finished: &[&str]) -> Self {
        self.finished = finished.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    fn automatic(mut self, list: &[&str]) -> Self {
        let list: Vec<String> = list.iter().map(|s| (*s).to_owned()).collect();
        self.automatic = AutomaticSubplans::from_list(&list);
        self
    }

    fn mipres<'b>(self, checker: &'b dyn MipresChecker) -> Run<'b> {
        Run {
            max: self.max,
            only: self.only,
            limit: self.limit,
            finished: self.finished,
            automatic: self.automatic,
            mipres: Some(checker),
        }
    }

    fn go(&self, lines: &[Line]) -> Result<PrepareResult, MipresError> {
        prepare(
            lines,
            &test_catalog(),
            &PrepareOptions {
                max: self.max,
                only: &self.only,
                limit: self.limit,
                finished: &self.finished,
                automatic: &self.automatic,
                mipres: self.mipres,
            },
        )
    }

    fn ok(&self, lines: &[Line]) -> PrepareResult {
        self.go(lines).expect("prepare")
    }
}

fn prepared_ids(result: &PrepareResult) -> String {
    result
        .invoices
        .iter()
        .map(|i| i.invoice.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn excluded_ids(result: &PrepareResult) -> String {
    result
        .excluded
        .iter()
        .map(|e| e.invoice.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

fn reasons(result: &PrepareResult) -> HashMap<String, String> {
    result
        .excluded
        .iter()
        .map(|e| (e.invoice.clone(), e.reason.clone()))
        .collect()
}

#[test]
fn validates_and_enriches() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500,00", "3000"),
        line("EVS-1", "30020_02", "12345", "100", "100"),
        line("EVS-2", "20010_01", "19950404-1", "1600", "1600"),
        line("EVS-3", "99999_01", "000", "1", "1"),
        line("EVS-4", "20010_01", "19950404-1", "10", "abc"),
        line("EVS-5", "20010_01", "19950404-1", "10", "250001"),
        line("EVS-6", "40040_01", "55555", "1", "1"),
        line("", "20010_01", "19950404-1", "1", "1"),
    ];
    let result = Run::new("250000").ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1");
    let enriched = &result.invoices[0].lines[0];
    assert_eq!(enriched.cum, "19950404-1");
    assert_eq!(enriched.product_type, "714");
    assert_eq!(enriched.pharmaceutical_form, "TABLETA");
    assert_eq!(enriched.concentration, "500");
    assert_eq!(enriched.measure_unit, "MG");
    assert_eq!(enriched.agreed_price, "1500");
    assert_eq!(
        enriched.unit_value, "1500,00",
        "prices stay as in the convenio"
    );
    let expected: HashMap<String, String> = [
        ("EVS2", "PRECIO_MAYOR_NOTA_TECNICA: precio convenio COP=1600, precio pactado COP=1500. Requiere nota credito y refacturacion; no se genera JSON ni se radica."),
        ("EVS3", "No cruza nota tecnica: PLU=99999_01, codigoGenerico=99999, CUM convenio=000"),
        ("EVS4", "Factura excluida: valor total ausente o invalido"),
        ("EVS5", "MONTO_MAXIMO_SUPERADO: total COP=250001, maximo COP=250000"),
        ("EVS6", "SIN_CLASIFICACION_NOTA_TECNICA: la nota tecnica no alimento CLASIFICACION 714/799. PLU=40040_01, CUM=55555, clasificacion=ABC"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    assert_eq!(reasons(&result), expected);
    assert_eq!(result.stats.medicines, 1);
    assert_eq!(result.stats.other_services, 1);
    assert_eq!(result.stats.dropped_no_invoice, 1);
    assert_eq!(
        excluded_ids(&result),
        "EVS2,EVS3,EVS4,EVS5,EVS6",
        "source order"
    );
}

#[test]
fn the_configured_automatic_subplans_replace_the_defaults() {
    let mut custom = line("EVS9", "20010_01", "19950404-1", "100", "100");
    custom.subplan = "CUSTOM PLAN".into();
    let lines = [custom];
    let result = Run::new("250000").automatic(&["custom plan"]).ok(&lines);
    assert_eq!((result.invoices.len(), result.excluded.len()), (1, 0));
    let result = Run::new("250000").automatic(&["OTHER PLAN"]).ok(&lines);
    assert_eq!((result.invoices.len(), result.excluded.len()), (0, 1));
    assert!(result.excluded[0].reason.contains("CUSTOM PLAN"));
}

#[test]
fn the_amount_rule_comes_before_the_catalog_rule() {
    let result = Run::new("100").ok(&[line("EVS9", "1_1", "1", "1", "999999")]);
    assert!(reasons(&result)["EVS9"].starts_with("MONTO_MAXIMO_SUPERADO"));
}

#[test]
fn the_cap_message_prints_the_amounts_with_their_written_scale() {
    let result = Run::new("3000.4").ok(&[
        line("EVS1", "20010_01", "19950404-1", "1", "1.000,00"),
        line("EVS1", "20010_01", "19950404-1", "1", "2.000,50"),
    ]);
    assert_eq!(
        reasons(&result)["EVS1"],
        "MONTO_MAXIMO_SUPERADO: total COP=3000.50, maximo COP=3000.4"
    );
    // 1000 + 2000.5 = 3000.50 equals the cap 3000.5: not above it.
    let at_cap = Run::new("3000.5").ok(&[
        line("EVS1", "20010_01", "19950404-1", "1", "1000"),
        line("EVS1", "20010_01", "19950404-1", "1", "2000,5"),
    ]);
    assert_eq!(prepared_ids(&at_cap), "EVS1");
}

#[test]
fn limit_counts_only_prepared_invoices() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500,00", "3000"),
        line("EVS-2", "20010_01", "19950404-1", "1600", "1600"),
        line("EVS-3", "99999_01", "000", "1", "1"),
        line("EVS-4", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-5", "99999_01", "000", "1", "1"),
    ];
    let result = Run::new("250000").limit(2).ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1,EVS4");
    assert_eq!(
        excluded_ids(&result),
        "EVS2,EVS3",
        "EVS5 is never evaluated"
    );
    assert_eq!(
        (
            result.stats.ignored,
            result.stats.evaluated,
            result.stats.limit
        ),
        (1, 4, 2)
    );
    let full = Run::new("250000").limit(5).ok(&lines);
    assert_eq!(full.invoices.len() + full.excluded.len(), 5);
    assert_eq!((full.stats.ignored, full.stats.evaluated), (0, 5));
    let all = Run::new("250000").ok(&lines);
    assert_eq!((all.invoices.len(), all.excluded.len()), (2, 3));
    assert_eq!(
        (all.stats.ignored, all.stats.evaluated, all.stats.limit),
        (0, 5, 0)
    );
}

#[test]
fn limit_replaces_exclusions_and_manual_reviews() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500", "300000"),
        line("EVS-2", "20010_01", "19950404-1", "1500", "1500"),
        line_sub("EVS-3", "OTHER PLAN"),
        line("EVS-4", "99999_01", "000", "1", "1"),
        line("EVS-5", "20010_01", "19950404-1", "1500", "1500"),
        with_doc(
            line("EVS-6", "20010_01", "19950404-1", "1500", "1500"),
            "CC",
            "1",
        ),
        with_doc(
            line("EVS-6", "20010_01", "19950404-1", "1500", "1500"),
            "CC",
            "2",
        ),
        line("EVS-7", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-8", "99999_01", "000", "1", "1"),
        line("EVS-9", "20010_01", "19950404-1", "1500", "1500"),
    ];
    let result = Run::new("250000").limit(3).ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS2,EVS5,EVS7");
    assert_eq!(excluded_ids(&result), "EVS1,EVS3,EVS4");
    assert_eq!(result.manual_review.len(), 1);
    assert_eq!(result.manual_review[0].invoice, "EVS6");
    assert_eq!(result.manual_review[0].code, CODE_MULTIPLE_AFFILIATES);
    let stats = &result.stats;
    assert_eq!(
        (
            stats.evaluated,
            stats.prepared_pending,
            stats.limit,
            stats.ignored,
            stats.finished
        ),
        (7, 3, 3, 2, 0)
    );
}

#[test]
fn fewer_preparable_invoices_than_the_limit_is_normal() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-2", "99999_01", "000", "1", "1"),
        line("EVS-3", "20010_01", "19950404-1", "1500", "1500"),
    ];
    let result = Run::new("250000").limit(5).ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1,EVS3");
    assert_eq!(excluded_ids(&result), "EVS2");
    let stats = &result.stats;
    assert_eq!(
        (
            stats.evaluated,
            stats.prepared_pending,
            stats.limit,
            stats.ignored
        ),
        (3, 2, 5, 0)
    );
    assert!(result.remaining.is_empty());
}

#[test]
fn finished_invoices_never_consume_the_limit_and_are_still_processed() {
    let lines = vec![
        line_sub("EVS-1", "EVENTO POS"),
        line_sub("EVS-2", "OTHER PLAN"),
        line("EVS-3", "99999_01", "000", "1", "1"),
        line_sub("EVS-4", "EVENTO POS"),
        line_sub("EVS-5", "EVENTO POS"),
        line_sub("EVS-6", "EVENTO POS"),
        line_sub("EVS-7", "EVENTO POS"),
    ];
    let result = Run::new("250000")
        .limit(2)
        .finished(&["EVS1", "EVS3", "EVS6"])
        .ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1,EVS4,EVS5");
    assert_eq!(excluded_ids(&result), "EVS2,EVS3");
    assert_eq!(result.stats.ignored, 2, "EVS6 and EVS7 are past the cut");
    assert_eq!(result.stats.finished, 2, "only the finished ones reached");
    assert_eq!(
        (result.stats.evaluated, result.stats.prepared_pending),
        (3, 2)
    );
    assert_eq!(result.remaining, ["EVS7"], "finished EVS6 is not pending");

    let nothing_finished = Run::new("250000").limit(2).ok(&lines);
    assert_eq!(prepared_ids(&nothing_finished), "EVS1,EVS4");
    assert_eq!(
        (
            nothing_finished.stats.ignored,
            nothing_finished.stats.finished,
            nothing_finished.stats.evaluated
        ),
        (3, 0, 4)
    );

    let no_limit = Run::new("250000")
        .finished(&["EVS1", "EVS3", "EVS6"])
        .ok(&lines);
    assert_eq!(no_limit.invoices.len(), 5);
    assert_eq!(
        (
            no_limit.stats.ignored,
            no_limit.stats.finished,
            no_limit.stats.evaluated,
            no_limit.stats.limit
        ),
        (0, 3, 4, 0)
    );
}

#[test]
fn limit_honors_only_and_repeated_lines_count_once() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-2", "99999_01", "000", "1", "1"),
        line("EVS-3", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-3", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-4", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-5", "20010_01", "19950404-1", "1500", "1500"),
    ];
    let result = Run::new("250000")
        .only(&["EVS2", "EVS3", "EVS4", "EVS77"])
        .limit(2)
        .ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS3,EVS4");
    assert_eq!(excluded_ids(&result), "EVS2");
    assert_eq!((result.stats.ignored, result.stats.evaluated), (2, 3));
    assert_eq!(result.unknown_invoices, ["EVS77"]);
}

#[test]
fn only_selected_invoices_are_prepared_and_unknown_ones_reported() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-2", "99999_01", "000", "1", "1"),
        line("EVS-3", "20010_01", "19950404-1", "1500", "1500"),
    ];
    let result = Run::new("250000")
        .only(&["evs-3", "EVS2", "EVS77"])
        .ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS3");
    assert_eq!(excluded_ids(&result), "EVS2");
    assert_eq!(result.unknown_invoices, ["EVS77"]);
    assert_eq!(result.stats.ignored, 1);
}

#[test]
fn a_zero_agreed_price_skips_the_price_rule() {
    let result = Run::new("1000000").ok(&[line("EVS1", "30020_02", "12345", "999999", "999999")]);
    assert_eq!(result.invoices.len(), 1, "{:?}", result.excluded);
}

#[test]
fn an_invoice_with_several_affiliates_is_held_without_printing_documents() {
    let lines = vec![
        with_doc(
            line("EVS-1", "20010_01", "19950404-1", "1500", "1500"),
            "CC",
            "111",
        ),
        with_doc(
            line("EVS-1", "30020_02", "12345", "100", "100"),
            " cc ",
            "111 ",
        ),
        with_doc(line("EVS-2", "99999_01", "000", "1", "1"), "CC", "222"),
        with_doc(
            line("EVS-2", "20010_01", "19950404-1", "1500", "1500"),
            "TI",
            "222",
        ),
        with_doc(
            line("EVS-2", "20010_01", "19950404-1", "1500", "1500"),
            "CC",
            "333",
        ),
    ];
    let result = Run::new("250000").ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1");
    assert!(
        result.excluded.is_empty(),
        "the affiliate rule precedes the catalog rule"
    );
    let held = &result.manual_review[0];
    assert_eq!(held.invoice, "EVS2");
    assert_eq!(held.code, CODE_MULTIPLE_AFFILIATES);
    assert_eq!(
        held.reason,
        "Factura con lineas de mas de un afiliado (3 documentos distintos); revisar en Medicar."
    );
    assert_eq!(
        held.text(),
        format!("MULTIPLES_AFILIADOS_EN_FACTURA: {}", held.reason)
    );
    assert!(!held.text().contains("222") && !held.text().contains("333"));
}

#[test]
fn repeated_spellings_of_one_invoice_are_one_outcome() {
    let lines = vec![
        line("EVS-2", "20010_01", "19950404-1", "1600", "1600"),
        line("evs2", "20010_01", "19950404-1", "1600", "1600"),
        line(" EVS-2 ", "20010_01", "19950404-1", "1600", "1600"),
    ];
    let result = Run::new("250000").only(&["EVS2", "evs-2"]).ok(&lines);
    assert_eq!((result.excluded.len(), result.invoices.len()), (1, 0));
}

#[test]
fn the_subplan_allowlist_is_tolerant_and_empty_is_not_automatic() {
    let lines = vec![
        line_sub("EVS-1", "EVENTO POS"),
        line_sub("EVS-2", "EVENTO NO PBS"),
        line_sub("EVS-3", "Transferencia de Pendientes con Autorizacion"),
        line_sub("EVS-4", "EVENTO SAS SIN AUTORIZACION"),
        line_sub("EVS-5", ""),
        line_sub("EVS-6", "PLAN NO LISTADO"),
    ];
    let result = Run::new("250000").ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1,EVS2,EVS3,EVS4");
    let excluded = reasons(&result);
    assert_eq!(
        excluded["EVS6"],
        "Subplan 'PLAN NO LISTADO' no se radica automaticamente; use --invoices para forzarlo."
    );
    assert_eq!(
        excluded["EVS5"],
        "Subplan 'vacío' no se radica automaticamente; use --invoices para forzarlo."
    );
}

#[test]
fn naming_the_invoice_forces_a_foreign_subplan_through() {
    let result = Run::new("250000")
        .only(&["EVS-4"])
        .ok(&[line_sub("EVS-4", "EVENTO SAS SIN AUTORIZACION")]);
    assert_eq!(prepared_ids(&result), "EVS4");
}

#[test]
fn remaining_lists_the_unreached_pending_invoices_in_csv_order() {
    let lines = vec![
        line("EVS-1", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-2", "99999_01", "000", "1", "1"),
        line("EVS-3", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-4", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-5", "99999_01", "000", "1", "1"),
        line("EVS-6", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-6", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-7", "20010_01", "19950404-1", "1500", "1500"),
        line("EVS-8", "20010_01", "19950404-1", "1500", "1500"),
    ];
    let result = Run::new("250000")
        .only(&["EVS1", "EVS2", "EVS3", "EVS4", "EVS5", "EVS6", "EVS8"])
        .limit(2)
        .finished(&["EVS4"])
        .ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS1,EVS3");
    assert_eq!(result.remaining, ["EVS5", "EVS6", "EVS8"]);

    let three = &lines[..3];
    assert!(Run::new("250000").ok(three).remaining.is_empty());
    assert!(Run::new("250000").limit(5).ok(three).remaining.is_empty());
    assert!(Run::new("250000").limit(2).ok(three).remaining.is_empty());
}

#[test]
fn a_prepared_format_load_keeps_the_file_drug_fields_and_back_fills_blanks() {
    let mut with_file = line("EVS1", "20010_01", "19950404-1", "1500", "1500");
    with_file.classification = "714".into();
    with_file.pharmaceutical_form = "JARABE".into();
    let result = Run::new("250000").ok(&[with_file]);
    assert!(result.prepared_format);
    let enriched = &result.invoices[0].lines[0];
    assert_eq!(enriched.pharmaceutical_form, "JARABE", "the file wins");
    assert_eq!(
        enriched.concentration, "500",
        "blank: back-filled from the NT"
    );
    assert_eq!(enriched.measure_unit, "MG");

    let mut other_invoice = line("EVS2", "20010_01", "19950404-1", "1500", "1500");
    other_invoice.pharmaceutical_form = "JARABE".into();
    let result = Run::new("250000").ok(&[other_invoice]);
    assert!(!result.prepared_format);
    assert_eq!(
        result.invoices[0].lines[0].pharmaceutical_form, "TABLETA",
        "outside a prepared-format load the NT always wins"
    );
}

#[test]
fn the_prepared_format_flag_looks_at_the_whole_csv_even_with_only() {
    let mut marked = line("EVS1", "20010_01", "19950404-1", "1500", "1500");
    marked.classification = "799".into();
    let other = line("EVS2", "20010_01", "19950404-1", "1500", "1500");
    let result = Run::new("250000").only(&["EVS2"]).ok(&[marked, other]);
    assert!(result.prepared_format);
}

#[test]
fn copay_lines_are_counted_for_prepared_invoices() {
    let mut paying = line("EVS1", "20010_01", "19950404-1", "1500", "1500");
    paying.copay = "1.500,50".into();
    let free = line("EVS1", "30020_02", "12345", "1", "1");
    let result = Run::new("250000").ok(&[paying, free]);
    assert_eq!(result.stats.copay_lines, 1);
}

// ----- MIPRES -------------------------------------------------------------

/// Answers per prescription and records every call.
#[derive(Default)]
struct FakeMipres {
    deliveries: HashMap<String, Vec<Delivery>>,
    errors: HashMap<String, MipresError>,
    calls: Mutex<Vec<String>>,
}

impl FakeMipres {
    fn with(mut self, prescription: &str, deliveries: &[(&str, i32, &str)]) -> Self {
        self.deliveries.insert(
            prescription.into(),
            deliveries
                .iter()
                .map(|(id, state, delivery_id)| Delivery {
                    id: (*id).into(),
                    delivery_id: (*delivery_id).into(),
                    state: *state,
                })
                .collect(),
        );
        self
    }

    fn failing(mut self, prescription: &str, error: MipresError) -> Self {
        self.errors.insert(prescription.into(), error);
        self
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls").clone()
    }
}

impl MipresChecker for FakeMipres {
    fn entregas(&self, prescription: &str) -> Result<Vec<Delivery>, MipresError> {
        self.calls.lock().expect("calls").push(prescription.into());
        if let Some(error) = self.errors.get(prescription) {
            return Err(error.clone());
        }
        Ok(self
            .deliveries
            .get(prescription)
            .cloned()
            .unwrap_or_default())
    }
}

fn no_pbs(invoice: &str, prescription: &str, formula: &str) -> Line {
    let mut l = line(invoice, "20010_01", "19950404-1", "1500", "1500");
    l.subplan = "EVENTO NO PBS".into();
    l.mipres = prescription.into();
    l.formula_number = formula.into();
    l
}

fn manual_codes(result: &PrepareResult) -> HashMap<String, String> {
    result
        .manual_review
        .iter()
        .map(|m| (m.invoice.clone(), m.code.clone()))
        .collect()
}

#[test]
fn a_delivered_invoice_is_prepared_with_the_matched_id() {
    let fake = FakeMipres::default().with("P1", &[("999", 2, "E999"), ("123", 2, "E123")]);
    let result = Run::new("250000")
        .mipres(&fake)
        .ok(&[no_pbs("EVS-1", "P1", "123")]);
    assert_eq!(result.invoices.len(), 1);
    assert_eq!(result.invoices[0].id_mipres, "E123");
    assert!(result.excluded.is_empty() && result.manual_review.is_empty());
}

#[test]
fn the_delivery_state_decides_the_outcome() {
    type Case = (
        &'static str,
        Vec<(&'static str, i32, &'static str)>,
        bool,
        &'static str,
    );
    let cases: Vec<Case> = vec![
        ("delivered", vec![("7", 2, "D7")], true, ""),
        ("annulled", vec![("7", 0, "A7")], false, "MIPRES ANULADO"),
        (
            "not delivered",
            vec![("7", 1, "N7")],
            false,
            "MIPRES NO ENTREGADO",
        ),
        (
            "delivered beats annulled",
            vec![("7", 0, "A7"), ("7", 1, "N7"), ("7", 2, "D7")],
            true,
            "",
        ),
        (
            "annulled beats not delivered",
            vec![("7", 1, "N7"), ("7", 0, "A7")],
            false,
            "MIPRES ANULADO",
        ),
        (
            "other ids ignored",
            vec![("8", 2, "E8"), ("7", 1, "N7")],
            false,
            "MIPRES NO ENTREGADO",
        ),
    ];
    for (name, answer, prepared, reason) in cases {
        let fake = FakeMipres::default().with("P1", &answer);
        let result = Run::new("250000")
            .mipres(&fake)
            .ok(&[no_pbs("EVS-1", "P1", "7")]);
        if prepared {
            assert_eq!(result.invoices.len(), 1, "{name}");
            assert_eq!(result.invoices[0].id_mipres, "D7", "{name}");
        } else {
            assert!(
                result.invoices.is_empty() && result.manual_review.is_empty(),
                "{name}"
            );
            assert_eq!(reasons(&result)["EVS1"], reason, "{name}");
        }
    }
}

#[test]
fn delivery_ids_compare_numerically_when_both_are_numbers() {
    let fake = FakeMipres::default()
        .with("P1", &[("007", 2, "700")])
        .with("P2", &[(" 12 ", 2, " 812 ")])
        .with("P3", &[("A7", 2, "D3")]);
    let result = Run::new("250000").mipres(&fake).ok(&[
        no_pbs("EVS-1", "P1", "7"),
        no_pbs("EVS-2", "P2", "12"),
        no_pbs("EVS-3", "P3", "a7"),
    ]);
    assert_eq!(result.invoices.len(), 2);
    assert_eq!(result.invoices[0].id_mipres, "700");
    assert_eq!(result.invoices[1].id_mipres, "812");
    assert!(result.manual_review.is_empty());
    assert_eq!(result.excluded.len(), 1);
    assert_eq!(result.excluded[0].invoice, "EVS3");
    assert!(
        result.excluded[0]
            .reason
            .starts_with(REASON_MIPRES_SIN_ENTREGA)
    );
}

#[test]
fn no_matching_delivery_excludes_because_there_is_nothing_to_bill() {
    let fake = FakeMipres::default()
        .with("P1", &[("8", 2, "D8")])
        .with("P2", &[]);
    let result = Run::new("250000")
        .mipres(&fake)
        .ok(&[no_pbs("EVS-1", "P1", "7"), no_pbs("EVS-2", "P2", "7")]);
    assert!(result.invoices.is_empty() && result.manual_review.is_empty());
    assert_eq!(result.excluded.len(), 2);
    assert_eq!(
        result.excluded[0].reason,
        "MIPRES SIN ENTREGA: prescripcion=P1, No. Formula=7 (MIPRES no lista una entrega con ese ID)"
    );
    assert!(result.excluded[1].reason.contains("prescripcion=P2"));
}

#[test]
fn a_failed_query_or_an_unknown_state_holds_the_invoice() {
    let fake = FakeMipres::default()
        .failing("P1", MipresError::Other("mipres: HTTP 500".into()))
        .with("P2", &[("7", 5, "D7")])
        .with("P3", &[("7", -1, "D7")]);
    let result = Run::new("250000").mipres(&fake).ok(&[
        no_pbs("EVS-1", "P1", "7"),
        no_pbs("EVS-2", "P2", "7"),
        no_pbs("EVS-3", "P3", "7"),
    ]);
    assert!(result.invoices.is_empty() && result.excluded.is_empty());
    let codes = manual_codes(&result);
    for invoice in ["EVS1", "EVS2", "EVS3"] {
        assert_eq!(codes[invoice], CODE_MIPRES_NO_VERIFICADO);
    }
    assert_eq!(
        result.manual_review[0].reason,
        "No se pudo consultar la entrega MIPRES (prescripcion=P1, No. Formula=7): mipres: HTTP 500"
    );
    assert_eq!(
        result.manual_review[1].reason,
        "MIPRES devolvio un estado de entrega desconocido (prescripcion=P2, No. Formula=7, estado=5)."
    );
    assert_eq!(result.stats.mipres_unavailable, 0);
}

#[test]
fn the_first_failing_line_decides() {
    let fake = FakeMipres::default()
        .with("P1", &[("1", 2, "D1")])
        .with("P2", &[("2", 1, "D2")])
        .with("P3", &[("3", 0, "D3")]);
    let result = Run::new("250000").mipres(&fake).ok(&[
        no_pbs("EVS-1", "P1", "1"),
        no_pbs("EVS-1", "P2", "2"),
        no_pbs("EVS-1", "P3", "3"),
    ]);
    assert!(result.invoices.is_empty());
    assert_eq!(reasons(&result)["EVS1"], "MIPRES NO ENTREGADO");
}

#[test]
fn delivery_ids_are_distinct_and_in_line_order() {
    let fake = FakeMipres::default()
        .with("P1", &[("20", 2, "D20")])
        .with("P2", &[("10", 2, "D10"), ("10", 2, "D10")]);
    let result = Run::new("250000").mipres(&fake).ok(&[
        no_pbs("EVS-1", "P1", "20"),
        no_pbs("EVS-1", "P2", "10"),
        no_pbs("EVS-1", "P1", "20"),
    ]);
    assert_eq!(result.invoices[0].id_mipres, "D20, D10");
}

#[test]
fn lines_without_prescription_or_formula_are_not_checked() {
    let fake = FakeMipres::default().with("P1", &[("1", 2, "D1")]);
    let result = Run::new("250000").mipres(&fake).ok(&[
        no_pbs("EVS-1", "P1", "1"),
        no_pbs("EVS-1", "", "5"),
        no_pbs("EVS-1", "P9", ""),
    ]);
    assert_eq!(result.invoices[0].id_mipres, "D1");
    assert_eq!(fake.calls(), ["P1"]);

    let idle = FakeMipres::default();
    let result = Run::new("250000")
        .mipres(&idle)
        .ok(&[no_pbs("EVS-2", "", ""), no_pbs("EVS-2", " ", "9")]);
    assert_eq!(result.invoices.len(), 1);
    assert_eq!(result.invoices[0].id_mipres, "");
    assert!(idle.calls().is_empty());
}

#[test]
fn only_no_pbs_invoices_that_passed_every_local_rule_are_checked() {
    let fake = FakeMipres::default().with("P1", &[("1", 2, "D1")]);
    let mut pos = line("EVS-1", "20010_01", "19950404-1", "1500", "1500");
    pos.mipres = "P1".into();
    pos.formula_number = "1".into();
    let mut too_expensive = no_pbs("EVS-2", "P1", "1");
    too_expensive.unit_value = "1600".into();
    let mut accented = no_pbs("EVS-3", "P1", "1");
    accented.subplan = "  Evento no PBS ".into();
    let result = Run::new("250000")
        .mipres(&fake)
        .ok(&[pos, too_expensive, accented]);
    assert_eq!(
        fake.calls(),
        ["P1"],
        "one call: the NO PBS invoice that passed"
    );
    assert_eq!(result.invoices.len(), 2);
    assert_eq!(result.invoices[0].id_mipres, "");
    assert_eq!(result.invoices[1].id_mipres, "D1");
    assert!(reasons(&result)["EVS2"].starts_with("PRECIO_MAYOR_NOTA_TECNICA"));
}

#[test]
fn answers_are_cached_per_prescription_failures_included() {
    let fake = FakeMipres::default()
        .with("P1", &[("1", 2, "D1"), ("2", 2, "D2")])
        .failing("P2", MipresError::Other("boom".into()));
    let result = Run::new("250000").mipres(&fake).ok(&[
        no_pbs("EVS-1", "P1", "1"),
        no_pbs("EVS-2", "P1", "2"),
        no_pbs("EVS-3", "P1", "1"),
        no_pbs("EVS-4", "P2", "1"),
        no_pbs("EVS-5", "P2", "1"),
    ]);
    assert_eq!(fake.calls(), ["P1", "P2"]);
    assert_eq!((result.invoices.len(), result.manual_review.len()), (3, 2));
}

#[test]
fn rejected_invoices_do_not_count_in_stats_nor_consume_the_limit() {
    let fake = FakeMipres::default()
        .with("P1", &[("1", 0, "A1")])
        .with("P2", &[("2", 2, "D2")])
        .with("P3", &[("3", 2, "D3")]);
    let lines = vec![
        no_pbs("EVS-1", "P1", "1"),
        no_pbs("EVS-2", "P2", "2"),
        no_pbs("EVS-3", "P3", "3"),
    ];
    let result = Run::new("250000").mipres(&fake).ok(&lines);
    assert_eq!(result.stats.medicines, 2, "only prepared invoices count");

    let fake = FakeMipres::default()
        .with("P1", &[("1", 0, "A1")])
        .with("P2", &[("2", 2, "D2")])
        .with("P3", &[("3", 2, "D3")]);
    let result = Run::new("250000").limit(1).mipres(&fake).ok(&lines);
    assert_eq!(prepared_ids(&result), "EVS2");
    assert_eq!(reasons(&result)["EVS1"], "MIPRES ANULADO");
    assert_eq!(
        (
            result.stats.ignored,
            result.stats.evaluated,
            result.stats.limit
        ),
        (1, 2, 1)
    );
    assert_eq!(
        fake.calls(),
        ["P1", "P2"],
        "never asked about the unreached EVS3"
    );
}

#[test]
fn systemic_errors_abort_the_preparation() {
    let lines = vec![
        no_pbs("EVS-1", "P1", "1"),
        no_pbs("EVS-2", "P2", "2"),
        no_pbs("EVS-3", "P3", "3"),
    ];
    let fake = FakeMipres::default().failing("P1", MipresError::Abort("HTTP 403".into()));
    let error = Run::new("250000")
        .mipres(&fake)
        .go(&lines)
        .expect_err("abort");
    assert_eq!(error.to_string(), "MIPRES check aborted: HTTP 403");
    assert_eq!(
        fake.calls(),
        ["P1"],
        "the run stops at the first systemic error"
    );
}

#[test]
fn ordinary_errors_stay_per_invoice_manual_review() {
    let fake = FakeMipres::default()
        .with("P2", &[("2", 2, "D2")])
        .failing("P1", MipresError::Other("mipres: HTTP 404".into()));
    let result = Run::new("250000")
        .mipres(&fake)
        .ok(&[no_pbs("EVS-1", "P1", "1"), no_pbs("EVS-2", "P2", "2")]);
    assert_eq!(result.invoices.len(), 1);
    assert_eq!(manual_codes(&result).len(), 1);
    assert_eq!(
        fake.calls().len(),
        2,
        "a per-prescription failure does not trip the breaker"
    );
}

#[test]
fn without_a_checker_the_mipres_check_is_skipped() {
    let result = Run::new("250000").ok(&[no_pbs("EVS-1", "P1", "1")]);
    assert_eq!(result.invoices.len(), 1);
    assert_eq!(result.invoices[0].id_mipres, "");
}

#[test]
fn an_outage_trips_the_breaker_and_holds_only_the_no_pbs_invoices() {
    let down = || MipresError::Unavailable("mipres: token request: i/o timeout".into());
    let fake = FakeMipres::default()
        .with("P0", &[("7", 2, "D7")])
        .failing("P1", down())
        .failing("P2", down())
        .failing("P3", down());
    let lines = vec![
        no_pbs("EVS-0", "P0", "7"),
        no_pbs("EVS-1", "P1", "1"),
        no_pbs("EVS-2", "P2", "2"),
        no_pbs("EVS-3", "P3", "3"),
        line("EVS-4", "20010_01", "19950404-1", "1500", "1500"),
    ];
    let result = Run::new("250000").mipres(&fake).ok(&lines);
    assert_eq!(
        fake.calls(),
        ["P0", "P1"],
        "the first failure trips the breaker"
    );
    assert_eq!(prepared_ids(&result), "EVS0,EVS4");
    let codes = manual_codes(&result);
    assert_eq!(codes.len(), 3);
    assert!(codes.values().all(|c| c == CODE_MIPRES_NO_VERIFICADO));
    for held in &result.manual_review {
        assert!(held.reason.contains("MIPRES no estaba disponible"));
        assert!(held.reason.contains("proxima ejecucion"));
    }
    assert!(result.manual_review[0].reason.ends_with(
        "(prescripcion=P1, No. Formula=1): MIPRES unavailable: mipres: token request: i/o timeout"
    ));
    assert_eq!(result.stats.mipres_unavailable, 3);
}

#[test]
fn needs_mipres_follows_the_selection_and_the_allowlist() {
    let a = no_pbs("EVS-1", "P1", "1");
    let b = line("EVS-2", "20010_01", "19950404-1", "1", "1");
    let unnamed = no_pbs("", "P1", "1");
    let defaults = AutomaticSubplans::defaults();
    let only = |names: &[&str]| -> Vec<String> { names.iter().map(|s| (*s).to_owned()).collect() };
    assert!(!needs_mipres(&[], &[], &defaults));
    assert!(!needs_mipres(std::slice::from_ref(&b), &[], &defaults));
    assert!(needs_mipres(&[b.clone(), a.clone()], &[], &defaults));
    assert!(!needs_mipres(
        &[b.clone(), a.clone()],
        &only(&["evs-2"]),
        &defaults
    ));
    assert!(needs_mipres(
        &[b.clone(), a.clone()],
        &only(&["EVS1"]),
        &defaults
    ));
    assert!(!needs_mipres(&[unnamed], &[], &defaults));

    let without = AutomaticSubplans::from_list(&["EVENTO POS".to_owned()]);
    assert!(!needs_mipres(&[b.clone(), a.clone()], &[], &without));
    assert!(
        needs_mipres(std::slice::from_ref(&a), &only(&["EVS1"]), &without),
        "--invoices forces it"
    );
    let custom = AutomaticSubplans::from_list(&["evento no pbs".to_owned()]);
    assert!(needs_mipres(&[b, a.clone()], &[], &custom));
    assert!(!needs_mipres(&[a], &[], &AutomaticSubplans::from_list(&[])));
}

#[test]
fn the_allowlist_composes_with_the_mipres_check() {
    let delivered = || FakeMipres::default().with("P1", &[("123", 2, "E123")]);
    let lines = [no_pbs("EVS-1", "P1", "123")];

    let fake = delivered();
    let result = Run::new("250000")
        .automatic(&["EVENTO NO PBS"])
        .mipres(&fake)
        .ok(&lines);
    assert_eq!(result.invoices[0].id_mipres, "E123");
    assert_eq!(fake.calls(), ["P1"]);

    let fake = delivered();
    let result = Run::new("250000")
        .automatic(&["EVENTO POS"])
        .mipres(&fake)
        .ok(&lines);
    assert!(result.invoices.is_empty());
    assert!(
        result.excluded[0]
            .reason
            .contains("no se radica automaticamente")
    );
    assert!(
        fake.calls().is_empty(),
        "never asked for a non-automatic subplan"
    );

    let fake = delivered();
    let result = Run::new("250000")
        .automatic(&["EVENTO POS"])
        .only(&["EVS1"])
        .mipres(&fake)
        .ok(&lines);
    assert_eq!(result.invoices.len(), 1);
    assert_eq!(fake.calls(), ["P1"], "forced invoices are still verified");
}
