//! `GET /api/seguimiento/consulta`: the paginated, filtered "Seguimiento de
//! RIPS" query (Go `dashboard.Consulta` and the SQL of
//! `mysqlstore/seguimiento_query.go`).
//!
//! The rows are the invoices of every current cargue, read through the stored
//! summary columns of `dataset_factura`, joined in memory with the native
//! state of the prepared ones and classified by [`derive`]. Filters, sort,
//! cursor, KPIs and facets are then applied to that set with the Go
//! semantics; only the page's rows read the side data (texts, XML total,
//! portal summary). The answer goes through [`derive::cached`].
//!
//! Every text comparison is byte for byte, like Go's `utf8mb4_bin`: text
//! filters are case-insensitive substring tests that stay accent-sensitive;
//! `estado` and `pipeline` compare exactly; dates compare as text.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::{Datelike, Local, NaiveDate};
use serde::{Deserialize, Serialize};

use crate::api::read::derive::{
    ALERTS, Derived, Kpis, PIPELINES, cache_key, cached, derive as derive_row, display_estado,
    mipres_column, parse_day, parse_state, quoted, sortable_time,
};
use crate::api::read::tracking::{TrackingRow, build_row, into_list_row};
use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::domain::dataset::InvoiceState;
use crate::stores::dataset::dashboard::{DatasetRow, NativeScope};
use crate::stores::dataset::{DatasetResult, DatasetStore};

const DEFAULT_LIMIT: usize = 300;
const MAX_LIMIT: usize = 1000;
/// Longest text filter, in characters.
const MAX_TEXT: usize = 200;

const SORT_DATE: &str = "fecha_factura";
const SORT_UPDATED: &str = "actualizado_en";
const SORT_INVOICE: &str = "factura";

/// The keyset position after the last row of a page: the sort key value and
/// the (cargue, invoice) identity that breaks its ties. Serialized as
/// `base64url(JSON{k,c,f,s,d})`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cursor {
    #[serde(default, rename = "k", skip_serializing_if = "String::is_empty")]
    key: String,
    #[serde(default, rename = "c")]
    cargue: String,
    #[serde(default, rename = "f")]
    invoice: String,
    #[serde(default, rename = "s", skip_serializing_if = "String::is_empty")]
    sort: String,
    #[serde(default, rename = "d", skip_serializing_if = "std::ops::Not::not")]
    desc: bool,
}

impl Cursor {
    fn encode(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        URL_SAFE_NO_PAD.encode(json)
    }

    fn decode(text: &str) -> Result<Cursor, ApiError> {
        let invalid = || ApiError::bad_request("cursor invalido");
        let bytes = URL_SAFE_NO_PAD.decode(text).map_err(|_| invalid())?;
        let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.cargue.is_empty() || cursor.invoice.is_empty() {
            return Err(invalid());
        }
        Ok(cursor)
    }
}

/// A validated request. Text filters are already trimmed and lower-cased.
#[derive(Debug, Default)]
struct Params {
    invoice: String,
    cargue: String,
    ssc: String,
    authorization: String,
    mipres: String,
    subplan: String,
    /// Exact display estado and pipeline category.
    estado: String,
    pipeline: String,
    /// Invoice date bounds (`YYYY-MM-DD`, compared as text).
    from: String,
    to: String,
    alerts: Vec<String>,
    kpi_from: String,
    kpi_to: String,
    /// `""` (cargue, invoice) or one of the `SORT_*` keys.
    sort: String,
    desc: bool,
    limit: usize,
    after: Option<Cursor>,
    aggregates: bool,
}

impl Params {
    /// The KPI date bounds: none while a cargue filter is set (the cards then
    /// cover the whole cargue), else the selected month.
    fn kpi_bounds(&self) -> (&str, &str) {
        if self.cargue.is_empty() {
            (&self.kpi_from, &self.kpi_to)
        } else {
            ("", "")
        }
    }
}

fn bad(message: impl Into<String>) -> ApiError {
    ApiError::bad_request(message)
}

/// Validates the query string of the route (Go `ParseConsulta`). `today` fixes
/// the month of the KPI cards when `kpi_desde`/`kpi_hasta` are not given.
fn parse(req: &Request, today: NaiveDate) -> Result<Params, ApiError> {
    let get = |name: &str| req.query_one(name).unwrap_or("");
    let mut p = Params {
        limit: DEFAULT_LIMIT,
        aggregates: true,
        ..Params::default()
    };
    let text = |name: &str| -> Result<String, ApiError> {
        let v = get(name).trim().to_lowercase();
        if v.chars().count() > MAX_TEXT {
            return Err(bad(format!(
                "{name} es demasiado largo (maximo {MAX_TEXT} caracteres)"
            )));
        }
        Ok(v)
    };
    p.invoice = text("factura")?;
    p.cargue = text("cargue")?;
    p.ssc = text("ssc")?;
    p.authorization = text("autorizacion")?;
    p.mipres = text("mipres")?;
    p.subplan = text("subplan")?;
    p.estado = get("estado").trim().to_owned();
    if p.estado.chars().count() > MAX_TEXT {
        return Err(bad("estado es demasiado largo"));
    }
    p.pipeline = get("pipeline").trim().to_owned();
    if !p.pipeline.is_empty() && !PIPELINES.contains(&p.pipeline.as_str()) {
        return Err(bad(format!("pipeline desconocido {}", quoted(&p.pipeline))));
    }
    let day = |name: &str| -> Result<String, ApiError> {
        let v = get(name).trim();
        if v.is_empty() {
            return Ok(String::new());
        }
        parse_day(v)
            .map(|_| v.to_owned())
            .ok_or_else(|| bad(format!("{name} debe ser YYYY-MM-DD")))
    };
    p.from = day("desde")?;
    p.to = day("hasta")?;
    p.kpi_from = day("kpi_desde")?;
    p.kpi_to = day("kpi_hasta")?;
    if p.kpi_from.is_empty() && p.kpi_to.is_empty() {
        let (first, last) = month_bounds(today);
        p.kpi_from = first.to_string();
        p.kpi_to = last.to_string();
    }
    for raw in req.query_all("alerta") {
        for alert in raw.split(',').map(str::trim).filter(|a| !a.is_empty()) {
            if p.alerts.iter().any(|seen| seen == alert) {
                continue;
            }
            if !ALERTS.contains(&alert) {
                return Err(bad(format!("alerta desconocida {}", quoted(alert))));
            }
            p.alerts.push(alert.to_owned());
        }
    }
    p.sort = get("sort").trim().to_owned();
    if ![SORT_DATE, SORT_UPDATED, SORT_INVOICE, ""].contains(&p.sort.as_str()) {
        return Err(bad(format!(
            "sort debe ser {SORT_DATE}, {SORT_UPDATED} o {SORT_INVOICE}"
        )));
    }
    p.desc = true; // a column starts descending, like the table header
    match get("dir").trim().to_lowercase().as_str() {
        "" | "desc" => {}
        "asc" => p.desc = false,
        _ => return Err(bad("dir debe ser asc o desc")),
    }
    if p.sort.is_empty() {
        p.desc = false; // the default order is cargue, invoice ascending
    }
    let limit = get("limit").trim();
    if !limit.is_empty() {
        p.limit = limit
            .parse::<i64>()
            .ok()
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| *n <= MAX_LIMIT)
            .ok_or_else(|| bad(format!("limit debe ser un entero entre 0 y {MAX_LIMIT}")))?;
    }
    match get("agregados").trim().to_lowercase().as_str() {
        "" | "si" => {}
        "no" => p.aggregates = false,
        _ => return Err(bad("agregados debe ser si o no")),
    }
    if p.limit == 0 && !p.aggregates {
        return Err(bad(
            "limit=0 solo pide los agregados: no puede combinarse con agregados=no",
        ));
    }
    let cursor = get("cursor").trim();
    if !cursor.is_empty() {
        let c = Cursor::decode(cursor)?;
        if c.sort != p.sort || c.desc != p.desc {
            return Err(bad("cursor no corresponde al orden pedido"));
        }
        p.after = Some(c);
    }
    Ok(p)
}

/// First and last day of the month of `day`.
fn month_bounds(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = day.with_day(1).unwrap_or(day);
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    };
    let last = next.and_then(|n| n.pred_opt()).unwrap_or(first);
    (first, last)
}

/// One invoice with everything the filters, the sort and the aggregates read.
struct Entry {
    row: DatasetRow,
    d: Derived,
    estado: &'static str,
}

impl Entry {
    fn prepared(&self) -> bool {
        self.d.state == InvoiceState::Prepared
    }

    /// The text of the sort key (what the cursor carries).
    fn sort_key(&self, sort: &str) -> String {
        match sort {
            SORT_DATE => self.row.fecha.clone(),
            SORT_INVOICE => self.row.factura.clone(),
            SORT_UPDATED => self.d.updated_at.map(sortable_time).unwrap_or_default(),
            _ => String::new(),
        }
    }
}

fn contains_ci(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(needle)
}

/// The filters that read the dataset alone: the text filters and the date range.
fn dataset_match(e: &Entry, p: &Params) -> bool {
    let r = &e.row;
    if !p.invoice.is_empty() && !contains_ci(&r.factura, &p.invoice) {
        return false;
    }
    if !p.cargue.is_empty() && !contains_ci(&format!("{} {}", r.load_name, r.load_id), &p.cargue) {
        return false;
    }
    if !p.ssc.is_empty() && !contains_ci(&r.ssc, &p.ssc) {
        return false;
    }
    if !p.authorization.is_empty() && !contains_ci(&r.autorizacion, &p.authorization) {
        return false;
    }
    if !p.mipres.is_empty() {
        let cell = if e.prepared() {
            mipres_column(&r.subplan, &r.id_mipres)
        } else {
            String::new()
        };
        if !contains_ci(&cell, &p.mipres) {
            return false;
        }
    }
    if !p.subplan.is_empty() && !contains_ci(&r.subplan, &p.subplan) {
        return false;
    }
    if (!p.from.is_empty() || !p.to.is_empty()) && r.fecha.is_empty() {
        return false;
    }
    (p.from.is_empty() || r.fecha.as_str() >= p.from.as_str())
        && (p.to.is_empty() || r.fecha.as_str() <= p.to.as_str())
}

/// The filters that read the classification: estado, pipeline and alerts, and
/// the implicit rule that excluded and manual-review rows only show when a
/// pipeline or an estado is asked for.
fn native_match(e: &Entry, p: &Params) -> bool {
    if !p.estado.is_empty() && e.estado != p.estado {
        return false;
    }
    if !p.pipeline.is_empty() && e.d.pipeline != p.pipeline {
        return false;
    }
    if p.pipeline.is_empty() && p.estado.is_empty() && !e.prepared() {
        return false;
    }
    p.alerts.is_empty() || p.alerts.iter().any(|a| e.d.alerts.contains(&a.as_str()))
}

/// Bounds the invoice date of the KPI cards: a row without a date fails a
/// lower bound but passes an upper one.
fn in_kpi_range(e: &Entry, p: &Params) -> bool {
    let (from, to) = p.kpi_bounds();
    (from.is_empty() || e.row.fecha.as_str() >= from)
        && (to.is_empty() || e.row.fecha.as_str() <= to)
}

#[derive(Debug, Serialize)]
struct Facets {
    estados: Vec<&'static str>,
    alertas: BTreeMap<&'static str, usize>,
}

#[derive(Debug, Serialize)]
struct Consulta {
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    filas: Vec<TrackingRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    facets: Option<Facets>,
    #[serde(skip_serializing_if = "Option::is_none")]
    kpis: Option<Kpis>,
    next_cursor: Option<String>,
}

pub fn get(state: &ApiState, req: &Request) -> ApiResult {
    let params = parse(req, Local::now().date_naive())?;
    cached(state, &cache_key("consulta", req), || {
        compute(state, &params)
    })
}

fn compute(state: &ApiState, p: &Params) -> ApiResult {
    let store = DatasetStore::new(state.pool.clone());
    let fail = |e: &dyn std::fmt::Display| {
        tracing::error!(error = %e, "dashboard: consulta");
        ApiError::internal("no se pudo consultar el seguimiento")
    };
    let rows = store.dashboard_rows().map_err(|e| fail(&e))?;
    let native = store
        .dashboard_native(NativeScope::AllCurrent)
        .map_err(|e| fail(&e))?;
    let entries: Vec<Entry> = rows
        .into_iter()
        .map(|row| {
            let d = derive_row(
                parse_state(&row.estado),
                &row.autorizacion,
                native.get(&row.factura_norm),
            );
            let estado = display_estado(&d);
            Entry { row, d, estado }
        })
        .collect();
    drop(native);

    let matched: Vec<&Entry> = entries
        .iter()
        .filter(|e| dataset_match(e, p) && native_match(e, p))
        .collect();
    let mut out = Consulta {
        total: None,
        filas: Vec::new(),
        facets: None,
        kpis: None,
        next_cursor: None,
    };
    if p.aggregates {
        out.total = Some(matched.len());
        out.kpis = Some(kpis(&matched, p));
        out.facets = Some(facets(&entries));
    }
    if p.limit > 0 {
        let (page, next) = page(matched, p);
        out.next_cursor = next;
        out.filas = page_rows(&store, &page).map_err(|e| fail(&e))?;
    }
    Ok(Response::json(&out))
}

/// KPI counts: the prepared rows among the matches inside the KPI bounds.
fn kpis(matched: &[&Entry], p: &Params) -> Kpis {
    let mut k = Kpis::default();
    for e in matched
        .iter()
        .filter(|e| e.prepared() && in_kpi_range(e, p))
    {
        k.add(&e.d);
    }
    k
}

/// The filter options, over every row (no filter applied): the distinct
/// display estados and how many rows raise each alert (non-zero only).
fn facets(entries: &[Entry]) -> Facets {
    let mut estados = BTreeSet::new();
    let mut alertas: BTreeMap<&'static str, usize> = BTreeMap::new();
    for e in entries {
        estados.insert(e.estado);
        for alert in &e.d.alerts {
            *alertas.entry(alert).or_default() += 1;
        }
    }
    Facets {
        estados: estados.into_iter().collect(),
        alertas,
    }
}

/// Orders the matches by the sort key in its direction, then by (cargue,
/// invoice) ascending whatever the direction, drops everything up to the
/// cursor and cuts one page. The cursor of the next page is set when rows
/// follow.
fn page<'a>(matched: Vec<&'a Entry>, p: &Params) -> (Vec<&'a Entry>, Option<String>) {
    let mut keyed: Vec<(String, &Entry)> = matched
        .into_iter()
        .map(|e| (e.sort_key(&p.sort), e))
        .collect();
    keyed.sort_by(|(ka, a), (kb, b)| {
        let by_key = if p.sort.is_empty() {
            Ordering::Equal
        } else if p.desc {
            kb.cmp(ka)
        } else {
            ka.cmp(kb)
        };
        by_key.then_with(|| identity(a).cmp(&identity(b)))
    });
    if let Some(c) = &p.after {
        keyed.retain(|(key, e)| after_cursor(key, e, c, p));
    }
    let more = keyed.len() > p.limit;
    keyed.truncate(p.limit);
    let next = match keyed.last() {
        Some((key, e)) if more => Some(
            Cursor {
                key: key.clone(),
                cargue: e.row.load_id.clone(),
                invoice: e.row.factura.clone(),
                sort: p.sort.clone(),
                desc: p.desc,
            }
            .encode(),
        ),
        _ => None,
    };
    (keyed.into_iter().map(|(_, e)| e).collect(), next)
}

fn identity(e: &Entry) -> (&str, &str) {
    (e.row.load_id.as_str(), e.row.factura.as_str())
}

/// The keyset condition: is the row after the cursor in the page order?
fn after_cursor(key: &str, e: &Entry, c: &Cursor, p: &Params) -> bool {
    let tie = identity(e) > (c.cargue.as_str(), c.invoice.as_str());
    if p.sort.is_empty() {
        return tie;
    }
    match key.cmp(c.key.as_str()) {
        Ordering::Less => p.desc,
        Ordering::Greater => !p.desc,
        Ordering::Equal => tie,
    }
}

/// The wire rows of a page, with the side data read for their invoices only.
fn page_rows(store: &DatasetStore, page: &[&Entry]) -> DatasetResult<Vec<TrackingRow>> {
    let names: Vec<String> = page.iter().map(|e| e.row.factura_norm.clone()).collect();
    let extras = store.dashboard_extras(&names)?;
    Ok(page
        .iter()
        .map(|e| into_list_row(build_row(&e.row, &e.d, &extras)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 30).expect("date")
    }

    fn req(query: &[(&str, &str)]) -> Request {
        let mut r = Request::new("GET", "/api/seguimiento/consulta");
        for (k, v) in query {
            r = r.with_query(k, v);
        }
        r
    }

    fn err(query: &[(&str, &str)]) -> String {
        parse(&req(query), today()).expect_err("must fail").message
    }

    #[test]
    fn defaults_are_the_first_page_with_aggregates_and_the_current_month() {
        let p = parse(&req(&[]), today()).expect("params");
        assert_eq!((p.limit, p.aggregates, p.desc), (300, true, false));
        assert_eq!(
            (p.kpi_from.as_str(), p.kpi_to.as_str()),
            ("2026-09-01", "2026-09-30")
        );
        assert!(p.sort.is_empty() && p.after.is_none());
    }

    #[test]
    fn month_bounds_handle_december_and_leap_february() {
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).expect("date");
        assert_eq!(
            month_bounds(d(2026, 12, 15)),
            (d(2026, 12, 1), d(2026, 12, 31))
        );
        assert_eq!(
            month_bounds(d(2028, 2, 10)),
            (d(2028, 2, 1), d(2028, 2, 29))
        );
    }

    #[test]
    fn text_filters_are_trimmed_lowercased_and_bounded() {
        let p = parse(&req(&[("factura", "  FvS1 "), ("ssc", "ÁB")]), today()).expect("params");
        assert_eq!((p.invoice.as_str(), p.ssc.as_str()), ("fvs1", "áb"));
        let long = "x".repeat(201);
        assert_eq!(
            err(&[("subplan", long.as_str())]),
            "subplan es demasiado largo (maximo 200 caracteres)"
        );
        assert_eq!(
            err(&[("estado", long.as_str())]),
            "estado es demasiado largo"
        );
    }

    #[test]
    fn invalid_parameters_get_the_go_messages() {
        assert_eq!(err(&[("pipeline", "x")]), "pipeline desconocido \"x\"");
        assert_eq!(err(&[("desde", "2026-9-1")]), "desde debe ser YYYY-MM-DD");
        assert_eq!(err(&[("kpi_hasta", "no")]), "kpi_hasta debe ser YYYY-MM-DD");
        assert_eq!(
            err(&[("alerta", "falta_zip,otra")]),
            "alerta desconocida \"otra\""
        );
        assert_eq!(
            err(&[("sort", "x")]),
            "sort debe ser fecha_factura, actualizado_en o factura"
        );
        assert_eq!(err(&[("dir", "up")]), "dir debe ser asc o desc");
        let limit = "limit debe ser un entero entre 0 y 1000";
        assert_eq!(err(&[("limit", "1001")]), limit);
        assert_eq!(err(&[("limit", "-1")]), limit);
        assert_eq!(err(&[("limit", "a")]), limit);
        assert_eq!(err(&[("agregados", "tal")]), "agregados debe ser si o no");
        assert_eq!(
            err(&[("limit", "0"), ("agregados", "no")]),
            "limit=0 solo pide los agregados: no puede combinarse con agregados=no"
        );
        assert_eq!(err(&[("cursor", "###")]), "cursor invalido");
    }

    #[test]
    fn alerts_accept_commas_repeats_and_drop_duplicates() {
        let q = [
            ("alerta", "falta_zip, nota_ajuste"),
            ("alerta", "falta_zip,,sin_autorizacion"),
        ];
        let p = parse(&req(&q), today()).expect("params");
        assert_eq!(p.alerts, ["falta_zip", "nota_ajuste", "sin_autorizacion"]);
    }

    #[test]
    fn a_column_sort_starts_descending_and_the_default_ascending() {
        let p = parse(&req(&[("sort", "factura")]), today()).expect("params");
        assert!(p.desc);
        let p = parse(&req(&[("sort", "factura"), ("dir", "ASC")]), today()).expect("params");
        assert!(!p.desc);
        let p = parse(&req(&[("dir", "desc")]), today()).expect("params");
        assert!(!p.desc, "no sort key means the (cargue, invoice) order");
    }

    #[test]
    fn a_cursor_only_fits_the_order_it_was_issued_for() {
        let c = Cursor {
            key: "k".into(),
            cargue: "A".into(),
            invoice: "F1".into(),
            sort: "factura".into(),
            desc: true,
        };
        let token = c.encode();
        assert!(parse(&req(&[("sort", "factura"), ("cursor", &token)]), today()).is_ok());
        assert_eq!(
            err(&[("sort", "factura"), ("dir", "asc"), ("cursor", &token)]),
            "cursor no corresponde al orden pedido"
        );
        assert_eq!(
            err(&[("sort", "fecha_factura"), ("cursor", &token)]),
            "cursor no corresponde al orden pedido"
        );
        let empty = Cursor {
            cargue: String::new(),
            ..c.clone()
        };
        assert_eq!(
            err(&[("sort", "factura"), ("cursor", &empty.encode())]),
            "cursor invalido"
        );
    }

    #[test]
    fn the_cursor_json_keeps_the_go_keys_and_omits_empty_ones() {
        let c = Cursor {
            key: String::new(),
            cargue: "A".into(),
            invoice: "F".into(),
            sort: String::new(),
            desc: false,
        };
        let json = URL_SAFE_NO_PAD.decode(c.encode()).expect("base64");
        assert_eq!(
            String::from_utf8(json).expect("utf8"),
            r#"{"c":"A","f":"F"}"#
        );
    }
}
