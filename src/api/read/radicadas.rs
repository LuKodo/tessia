//! `GET /api/reportes/radicadas`: every invoice with its four filing steps
//! confirmed, with the filing date and the notes recorded for it (Go
//! `internal/dashboard/radicadas.go` and `cmd_serve_reportes.go`). Query:
//! `cargue` (repeatable), `desde`, `hasta`, `q`, `limite` and
//! `formato=json|csv` (the complete result as a download).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Local, NaiveDate, TimeZone, Utc};
use rust_decimal::Decimal;
use serde::Serialize;

use super::informe::{cargue_not_found, rfc3339, valid_cargue_id};
use crate::api::csv::{CsvWriter, download, safe_file_name};
use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::domain::normalize_invoice;
use crate::stores::radicacion::{CargueRef, FiledInvoice, LineFacts, PhaseNote, ReportStore};
use crate::util::decimal::parse_agreement_amount;

const DEFAULT_LIMIT: usize = 500;
const MAX_LIMIT: usize = 5000;
/// Longest invoice search accepted (the invoice column holds 60 characters).
const MAX_QUERY: usize = 60;

const READ_FAILED: &str = "no se pudo leer el reporte de radicadas";

/// The estado of an invoice that is fully filed.
const ESTADO_FILED: &str = "TERMINADO";

#[derive(Debug, Default, PartialEq, Eq)]
struct Query {
    /// The requested cargues in request order (repeats kept for the file name).
    cargues: Vec<String>,
    /// `YYYY-MM-DD` or empty.
    from: String,
    to: String,
    /// Normalized invoice fragment, empty for none.
    invoice: String,
    /// Rows of the JSON preview; `None` for no cap (the CSV).
    limit: Option<usize>,
}

fn parse_day(text: &str, name: &str) -> Result<String, ApiError> {
    // Go's layout `2006-01-02` is strict: two-digit month and day.
    if text.is_empty() {
        return Ok(String::new());
    }
    let valid = text.len() == 10 && NaiveDate::parse_from_str(text, "%Y-%m-%d").is_ok();
    if valid {
        Ok(text.to_owned())
    } else {
        Err(ApiError::bad_request(format!("{name} debe ser YYYY-MM-DD")))
    }
}

/// Validates the query string; the flag is `formato=csv`.
fn parse_query(req: &Request) -> Result<(Query, bool), ApiError> {
    let csv = match req.query_one("formato").unwrap_or_default() {
        "" | "json" => false,
        "csv" => true,
        _ => return Err(ApiError::bad_request("formato debe ser json o csv")),
    };
    let mut query = Query::default();
    for id in req.query_all("cargue") {
        if id.is_empty() {
            continue;
        }
        if !valid_cargue_id(id) {
            return Err(ApiError::bad_request(format!(
                "cargue invalido \"{}\"",
                id.replace('\\', "\\\\").replace('"', "\\\"")
            )));
        }
        query.cargues.push(id.to_owned());
    }
    query.from = parse_day(req.query_one("desde").unwrap_or_default(), "desde")?;
    query.to = parse_day(req.query_one("hasta").unwrap_or_default(), "hasta")?;
    if !query.from.is_empty() && !query.to.is_empty() && query.to < query.from {
        return Err(ApiError::bad_request("hasta es anterior a desde"));
    }
    let search = req.query_one("q").unwrap_or_default().trim();
    if search.len() > MAX_QUERY {
        return Err(ApiError::bad_request(format!(
            "q no puede superar {MAX_QUERY} caracteres"
        )));
    }
    query.invoice = normalize_invoice(search);
    let mut limit = DEFAULT_LIMIT;
    if let Some(text) = req.query_one("limite").filter(|t| !t.is_empty()) {
        match text.parse::<i64>() {
            Ok(n) if n >= 1 => limit = usize::try_from(n).unwrap_or(MAX_LIMIT).min(MAX_LIMIT),
            _ => {
                return Err(ApiError::bad_request("limite debe ser un entero positivo"));
            }
        }
    }
    query.limit = (!csv).then_some(limit);
    Ok((query, csv))
}

/// The filing day in `tz` is within `from..=to` (an empty bound is open).
fn day_in_range<Tz: TimeZone>(at: DateTime<Utc>, tz: &Tz, from: &str, to: &str) -> bool
where
    Tz::Offset: std::fmt::Display,
{
    let day = at.with_timezone(tz).format("%Y-%m-%d").to_string();
    (from.is_empty() || day.as_str() >= from) && (to.is_empty() || day.as_str() <= to)
}

/// The invoice-level fields derived from the lines of one prepared invoice
/// (Go `SummarizeLines`).
#[derive(Debug, Default, PartialEq, Eq)]
struct Summary {
    date: String,
    ssc: String,
    authorization: String,
    value: String,
}

fn first_filled<'a>(values: impl Iterator<Item = &'a str>) -> String {
    values
        .map(str::trim)
        .find(|v| !v.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Every text field comes from the first line whose trimmed value is not
/// empty; the date is cut to the day; the value is the sum of the line totals
/// as a plain decimal without trailing zeros, empty when there are no lines
/// or any total cannot be parsed.
fn summarize(lines: &[&LineFacts]) -> Summary {
    let date = first_filled(lines.iter().map(|l| l.date.as_str()));
    let mut summary = Summary {
        date: date.chars().take(10).collect(),
        ssc: first_filled(lines.iter().map(|l| l.ssc.as_str())),
        authorization: first_filled(lines.iter().map(|l| l.authorization.as_str())),
        value: String::new(),
    };
    let totals: Result<Vec<Decimal>, _> = lines
        .iter()
        .map(|l| parse_agreement_amount(&l.total_value))
        .collect();
    if let (Ok(totals), false) = (totals, lines.is_empty()) {
        summary.value = totals.into_iter().sum::<Decimal>().normalize().to_string();
    }
    summary
}

/// The notes of one invoice, oldest first (ties by text), without repeats.
fn join_notes(mut notes: Vec<(DateTime<Utc>, String)>) -> String {
    notes.sort();
    let mut seen = HashSet::new();
    let texts: Vec<String> = notes
        .into_iter()
        .filter(|(_, text)| seen.insert(text.clone()))
        .map(|(_, text)| text)
        .collect();
    texts.join(" | ")
}

fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct Row {
    cargue: String,
    cargue_nombre: String,
    factura: String,
    /// RFC 3339 UTC: when step 4 was confirmed.
    fecha_radicacion: String,
    estado: &'static str,
    fecha_factura: String,
    ssc: String,
    autorizacion: String,
    /// Convenio total as a plain decimal.
    valor: String,
    /// Notes recorded for the invoice, oldest first, joined with ` | `.
    observaciones: String,
    #[serde(skip)]
    filed_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct Report {
    generado_en: String,
    /// Invoices matching the filters, before `limite`.
    total: usize,
    /// True when `filas` holds fewer rows than `total`.
    truncado: bool,
    filas: Vec<Row>,
}

/// The filed invoices that pass the filters, newest filing first (ties by
/// cargue then invoice).
fn select_filed<Tz: TimeZone>(
    filed: Vec<FiledInvoice>,
    cargues: &HashMap<i64, &CargueRef>,
    query: &Query,
    tz: &Tz,
) -> Vec<FiledInvoice>
where
    Tz::Offset: std::fmt::Display,
{
    let filter_days = !query.from.is_empty() || !query.to.is_empty();
    let mut kept: Vec<FiledInvoice> = filed
        .into_iter()
        .filter(|f| normalize_invoice(&f.invoice).contains(&query.invoice))
        .filter(|f| !filter_days || day_in_range(f.filed_at, tz, &query.from, &query.to))
        .collect();
    let cargue_id = |f: &FiledInvoice| cargues.get(&f.cargue_seq).map(|c| c.id.clone());
    kept.sort_by(|a, b| {
        b.filed_at
            .cmp(&a.filed_at)
            .then_with(|| cargue_id(a).cmp(&cargue_id(b)))
            .then_with(|| a.invoice.cmp(&b.invoice))
    });
    kept
}

/// Adds the dataset summary and the notes to the rows to show.
fn build_rows(
    store: &ReportStore,
    shown: Vec<FiledInvoice>,
    cargues: &HashMap<i64, &CargueRef>,
) -> Result<Vec<Row>, crate::db::DbError> {
    let mut by_cargue: HashMap<i64, Vec<String>> = HashMap::new();
    for f in &shown {
        by_cargue
            .entry(f.cargue_seq)
            .or_default()
            .push(f.invoice.clone());
    }
    let mut lines: HashMap<(i64, String), Vec<LineFacts>> = HashMap::new();
    for (seq, invoices) in &by_cargue {
        for fact in store.line_facts(*seq, invoices)? {
            lines
                .entry((*seq, fact.invoice.clone()))
                .or_default()
                .push(fact);
        }
    }
    let norms: Vec<String> = {
        let mut seen = HashSet::new();
        shown
            .iter()
            .map(|f| normalize_invoice(&f.invoice))
            .filter(|n| seen.insert(n.clone()))
            .collect()
    };
    let mut notes: HashMap<String, Vec<(DateTime<Utc>, String)>> = HashMap::new();
    for PhaseNote {
        invoice,
        reason,
        at,
    } in store.phase_notes(&norms)?
    {
        let text = collapse(&reason);
        if !text.is_empty() {
            notes.entry(invoice).or_default().push((at, text));
        }
    }
    Ok(shown
        .into_iter()
        .map(|f| {
            let facts: Vec<&LineFacts> = lines
                .get(&(f.cargue_seq, f.invoice.clone()))
                .map(|l| l.iter().collect())
                .unwrap_or_default();
            let summary = summarize(&facts);
            let mut invoice_notes = notes
                .get(&normalize_invoice(&f.invoice))
                .cloned()
                .unwrap_or_default();
            let detail = collapse(&f.last_detail);
            if !detail.is_empty() {
                invoice_notes.push((f.filed_at, detail));
            }
            let cargue = cargues.get(&f.cargue_seq);
            Row {
                cargue: cargue.map(|c| c.id.clone()).unwrap_or_default(),
                cargue_nombre: cargue.map(|c| c.name.clone()).unwrap_or_default(),
                factura: f.invoice,
                fecha_radicacion: rfc3339(f.filed_at),
                estado: ESTADO_FILED,
                fecha_factura: summary.date,
                ssc: summary.ssc,
                autorizacion: summary.authorization,
                valor: summary.value,
                observaciones: join_notes(invoice_notes),
                filed_at: f.filed_at,
            }
        })
        .collect())
}

fn report_csv<Tz: TimeZone>(rows: &[Row], tz: &Tz) -> Vec<u8>
where
    Tz::Offset: std::fmt::Display,
{
    let mut csv = CsvWriter::new();
    csv.row(&[
        "Cargue",
        "Nombre cargue",
        "Factura",
        "Fecha factura",
        "Fecha radicacion",
        "Estado",
        "SSC",
        "Autorizacion",
        "Valor convenio",
        "Observaciones",
    ]);
    for row in rows {
        let filed = row
            .filed_at
            .with_timezone(tz)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string();
        csv.row(&[
            row.cargue.as_str(),
            row.cargue_nombre.as_str(),
            row.factura.as_str(),
            row.fecha_factura.as_str(),
            filed.as_str(),
            row.estado,
            row.ssc.as_str(),
            row.autorizacion.as_str(),
            row.valor.as_str(),
            row.observaciones.as_str(),
        ]);
    }
    csv.finish()
}

/// What a download covers in its file name: `todos`, the cargue id, or `varios`.
fn scope(cargues: &[String]) -> String {
    let distinct: Vec<&String> = {
        let mut seen = HashSet::new();
        cargues.iter().filter(|c| seen.insert(c.as_str())).collect()
    };
    match distinct.as_slice() {
        [] => "todos".to_owned(),
        [one] => safe_file_name(one),
        _ => "varios".to_owned(),
    }
}

fn read_failed(err: &dyn std::fmt::Display) -> ApiError {
    tracing::warn!(error = %err, "filed invoices report failed");
    ApiError::internal(READ_FAILED)
}

/// The cargues to read: the requested ones (unknown is a 404), else every
/// load's current cargue.
fn resolve_cargues(store: &ReportStore, requested: &[String]) -> Result<Vec<CargueRef>, ApiError> {
    if requested.is_empty() {
        return store.current_cargues().map_err(|e| read_failed(&e));
    }
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in requested.iter().filter(|id| seen.insert(id.as_str())) {
        let cargue = store
            .current_cargue(id)
            .map_err(|e| read_failed(&e))?
            .ok_or_else(|| cargue_not_found(id))?;
        out.push(cargue);
    }
    Ok(out)
}

pub fn get(state: &ApiState, req: &Request) -> ApiResult {
    let (query, csv) = parse_query(req)?;
    let now = Utc::now();
    let store = ReportStore::new(state.pool.clone());
    let cargues = resolve_cargues(&store, &query.cargues)?;
    let by_seq: HashMap<i64, &CargueRef> = cargues.iter().map(|c| (c.seq, c)).collect();
    let seqs: Vec<i64> = cargues.iter().map(|c| c.seq).collect();
    let filed = store.filed_invoices(&seqs).map_err(|e| read_failed(&e))?;
    let mut selected = select_filed(filed, &by_seq, &query, &Local);
    let total = selected.len();
    let truncated = query.limit.is_some_and(|limit| total > limit);
    if let Some(limit) = query.limit {
        selected.truncate(limit);
    }
    let rows = build_rows(&store, selected, &by_seq).map_err(|e| read_failed(&e))?;
    if !csv {
        return Ok(Response::json(&Report {
            generado_en: rfc3339(now),
            total,
            truncado: truncated,
            filas: rows,
        }));
    }
    let filename = format!(
        "radicadas-{}-{}.csv",
        scope(&query.cargues),
        now.with_timezone(&Local).format("%Y%m%d")
    );
    Ok(download(&filename, report_csv(&rows, &Local)))
}

#[cfg(test)]
mod tests {
    use chrono::FixedOffset;

    use super::*;

    fn at(day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, hour, 0, 0).unwrap()
    }

    fn request(query: &[(&str, &str)]) -> Request {
        let mut req = Request::new("GET", "/api/reportes/radicadas");
        for (k, v) in query {
            req = req.with_query(k, v);
        }
        req
    }

    fn rejected(query: &[(&str, &str)]) -> String {
        parse_query(&request(query)).expect_err("rejected").message
    }

    fn fact(invoice: &str, date: &str, ssc: &str, auth: &str, total: &str) -> LineFacts {
        LineFacts {
            invoice: invoice.into(),
            date: date.into(),
            ssc: ssc.into(),
            authorization: auth.into(),
            total_value: total.into(),
        }
    }

    #[test]
    fn the_query_defaults_to_a_500_row_preview() {
        let (query, csv) = parse_query(&request(&[])).expect("valid");
        assert!(!csv);
        assert_eq!(
            query,
            Query {
                limit: Some(500),
                ..Query::default()
            }
        );
        let (csv_query, csv) =
            parse_query(&request(&[("formato", "csv"), ("limite", "7")])).expect("valid");
        assert!(csv);
        assert_eq!(csv_query.limit, None, "the CSV ignores limite");
    }

    #[test]
    fn the_query_normalizes_and_caps_what_it_accepts() {
        let (query, _) = parse_query(&request(&[
            ("cargue", "A"),
            ("cargue", ""),
            ("cargue", "B"),
            ("cargue", "A"),
            ("desde", "2026-09-01"),
            ("hasta", "2026-09-01"),
            ("q", " fev-12 "),
            ("limite", "999999"),
        ]))
        .expect("valid");
        assert_eq!(query.cargues, ["A", "B", "A"]);
        assert_eq!(
            (query.from.as_str(), query.to.as_str()),
            ("2026-09-01", "2026-09-01")
        );
        assert_eq!(query.invoice, "FEV12");
        assert_eq!(query.limit, Some(5000));
    }

    #[test]
    fn the_query_is_rejected_with_the_go_messages() {
        assert_eq!(
            rejected(&[("formato", "xml")]),
            "formato debe ser json o csv"
        );
        assert_eq!(rejected(&[("cargue", "a/b")]), "cargue invalido \"a/b\"");
        assert_eq!(rejected(&[("cargue", "..")]), "cargue invalido \"..\"");
        for bad in [
            "2026-9-1",
            "26-09-01",
            "2026-02-30",
            "x",
            "2026/09/01",
            "2026-09-011",
        ] {
            assert_eq!(
                rejected(&[("desde", bad)]),
                "desde debe ser YYYY-MM-DD",
                "{bad}"
            );
            assert_eq!(
                rejected(&[("hasta", bad)]),
                "hasta debe ser YYYY-MM-DD",
                "{bad}"
            );
        }
        assert_eq!(
            rejected(&[("desde", "2026-09-02"), ("hasta", "2026-09-01")]),
            "hasta es anterior a desde"
        );
        assert_eq!(
            rejected(&[("q", &"a".repeat(61))]),
            "q no puede superar 60 caracteres"
        );
        assert!(parse_query(&request(&[("q", &"a".repeat(60))])).is_ok());
        for bad in ["0", "-1", "x", "1.5", " 3"] {
            assert_eq!(
                rejected(&[("limite", bad)]),
                "limite debe ser un entero positivo",
                "{bad}"
            );
        }
        // Validation order: formato, cargue, desde, hasta, range, q, limite.
        assert_eq!(
            rejected(&[("limite", "0"), ("q", &"a".repeat(61))]),
            "q no puede superar 60 caracteres"
        );
    }

    #[test]
    fn the_filing_day_is_compared_in_the_given_zone() {
        let bogota = FixedOffset::west_opt(5 * 3600).unwrap();
        // 03:00 UTC is still the previous day in UTC-5.
        let early = at(2, 3);
        assert!(day_in_range(early, &bogota, "2026-09-01", "2026-09-01"));
        assert!(!day_in_range(early, &bogota, "2026-09-02", ""));
        assert!(day_in_range(early, &Utc, "2026-09-02", "2026-09-02"));
        assert!(day_in_range(early, &Utc, "", ""));
        assert!(!day_in_range(early, &Utc, "", "2026-09-01"));
    }

    #[test]
    fn the_summary_takes_the_first_filled_values_and_sums_the_totals() {
        let a = fact("F1", " ", "", " AUT1 ", "1.234,50");
        let b = fact("F1", "2026-09-03 11:28:31", "SSC9", "AUT2", "0,50");
        let summary = summarize(&[&a, &b]);
        assert_eq!(
            summary,
            Summary {
                date: "2026-09-03".into(),
                ssc: "SSC9".into(),
                authorization: "AUT1".into(),
                value: "1235".into()
            }
        );
        let decimals = summarize(&[
            &fact("F1", "", "", "", "10,25"),
            &fact("F1", "", "", "", "0,50"),
        ]);
        assert_eq!(decimals.value, "10.75");
    }

    #[test]
    fn one_unparsable_total_or_no_lines_blank_the_value() {
        let good = fact("F1", "", "", "", "10");
        let bad = fact("F1", "", "", "", "abc");
        assert_eq!(summarize(&[&good, &bad]).value, "");
        assert_eq!(summarize(&[]), Summary::default());
    }

    #[test]
    fn notes_are_distinct_and_oldest_first() {
        let notes = vec![
            (at(3, 0), "tercero".to_owned()),
            (at(1, 0), "primero".to_owned()),
            (at(2, 0), "b".to_owned()),
            (at(2, 0), "a".to_owned()),
            (at(4, 0), "primero".to_owned()),
        ];
        assert_eq!(join_notes(notes), "primero | a | b | tercero");
        assert_eq!(join_notes(Vec::new()), "");
        assert_eq!(collapse("  uno \n\t dos  "), "uno dos");
    }

    #[test]
    fn downloads_are_named_after_their_scope() {
        assert_eq!(scope(&[]), "todos");
        assert_eq!(scope(&["Carga_1".into(), "Carga_1".into()]), "Carga_1");
        assert_eq!(scope(&["a b".into()]), "a_b");
        assert_eq!(scope(&["A".into(), "B".into()]), "varios");
    }

    #[test]
    fn rows_are_ordered_newest_first_then_by_cargue_and_invoice() {
        let first = CargueRef {
            seq: 1,
            id: "A".into(),
            name: "A".into(),
        };
        let second = CargueRef {
            seq: 2,
            id: "B".into(),
            name: "B".into(),
        };
        let cargues: HashMap<i64, &CargueRef> = [(1, &first), (2, &second)].into();
        let filed = |seq: i64, invoice: &str, day: u32| FiledInvoice {
            cargue_seq: seq,
            invoice: invoice.into(),
            filed_at: at(day, 12),
            last_detail: String::new(),
        };
        let rows = select_filed(
            vec![
                filed(2, "FEV1", 5),
                filed(1, "FEV9", 5),
                filed(1, "FEV2", 5),
                filed(1, "FEV7", 6),
                filed(1, "XYZ1", 4),
            ],
            &cargues,
            &Query {
                invoice: "FEV".into(),
                ..Query::default()
            },
            &Utc,
        );
        let order: Vec<&str> = rows.iter().map(|r| r.invoice.as_str()).collect();
        assert_eq!(order, ["FEV7", "FEV2", "FEV9", "FEV1"]);
    }
}
