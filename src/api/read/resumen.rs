//! `GET /api/resumen`: the KPI cards of a date range plus the per-cargue chart
//! series (Go `dashboard.Summarize` over `SummarizeStored`). The answer goes
//! through [`derive::cached`](super::derive::cached).

use std::collections::HashMap;

use chrono::{Duration, Local, NaiveDate};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::api::read::derive::{
    self, Kpis, cache_key, cached, derive as derive_row, parse_day, parse_state, plain_amount,
};
use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::domain::dataset::InvoiceState;
use crate::stores::dataset::DatasetStore;
use crate::stores::dataset::dashboard::NativeScope;

/// The longest range a request may ask for, in days.
const MAX_DAYS: i64 = 366;

/// One bar of the "Avance de radicación por cargue" chart: every invoice of
/// one cargue, whatever their dates or outcomes (`CarguePoint`).
#[derive(Debug, Serialize)]
struct CarguePoint {
    cargue: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    nombre: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    fecha_inicio: String,
    total: usize,
    gestionadas: usize,
    excluidas: usize,
    revision_manual: usize,
    radicadas: usize,
    valor_cargado: String,
    valor_radicado: String,
}

#[derive(Debug, Serialize)]
struct Summary {
    desde: String,
    hasta: String,
    kpis: Kpis,
    cargues: Vec<CarguePoint>,
}

struct Accumulator {
    point: CarguePoint,
    loaded: Decimal,
    filed: Decimal,
}

pub fn get(state: &ApiState, req: &Request) -> ApiResult {
    let (from, to) = range(req, Local::now().date_naive())?;
    cached(state, &cache_key("resumen", req), || {
        compute(state, from, to)
    })
}

/// The requested range, validated like Go's `summaryHandler`: the default is
/// the last 30 days ending today; `hasta` is read before `desde`.
fn range(req: &Request, today: NaiveDate) -> Result<(NaiveDate, NaiveDate), ApiError> {
    let day = |name: &str| -> Result<Option<NaiveDate>, ApiError> {
        match req.query_one(name).filter(|v| !v.is_empty()) {
            None => Ok(None),
            Some(v) => parse_day(v.trim())
                .map(Some)
                .ok_or_else(|| ApiError::bad_request(format!("--{name} must be YYYY-MM-DD"))),
        }
    };
    let to = day("hasta")?.unwrap_or(today);
    let from = day("desde")?.unwrap_or(default_from(today));
    if to < from {
        return Err(ApiError::bad_request("hasta es anterior a desde"));
    }
    if (to - from).num_days() >= MAX_DAYS {
        return Err(ApiError::bad_request("el rango no puede superar 366 días"));
    }
    Ok((from, to))
}

/// 29 days before `today`: the default `desde` when `hasta` is not given. When
/// only `hasta` is given Go still defaults `desde` to 29 days before *today*.
fn default_from(today: NaiveDate) -> NaiveDate {
    today - Duration::days(29)
}

fn compute(state: &ApiState, from: NaiveDate, to: NaiveDate) -> ApiResult {
    let store = DatasetStore::new(state.pool.clone());
    let fail = |e: &dyn std::fmt::Display| {
        tracing::error!(error = %e, "dashboard: summary");
        ApiError::internal("no se pudo calcular el resumen")
    };
    let rows = store.dashboard_rows().map_err(|e| fail(&e))?;
    let native = store
        .dashboard_native(NativeScope::AllCurrent)
        .map_err(|e| fail(&e))?;
    let (from_day, to_day) = (from.to_string(), to.to_string());
    let mut kpis = Kpis::default();
    let mut points: HashMap<String, Accumulator> = HashMap::new();
    for row in &rows {
        let state = parse_state(&row.estado);
        let d = derive_row(state, &row.autorizacion, native.get(&row.factura_norm));
        let acc = points
            .entry(row.load_id.clone())
            .or_insert_with(|| Accumulator {
                point: CarguePoint {
                    cargue: row.load_id.clone(),
                    nombre: row.load_name.clone(),
                    fecha_inicio: row.load_from.map(|v| v.to_string()).unwrap_or_default(),
                    total: 0,
                    gestionadas: 0,
                    excluidas: 0,
                    revision_manual: 0,
                    radicadas: 0,
                    valor_cargado: String::new(),
                    valor_radicado: String::new(),
                },
                loaded: Decimal::ZERO,
                filed: Decimal::ZERO,
            });
        acc.point.total += 1;
        match state {
            InvoiceState::Prepared => {
                let value = row.valor.unwrap_or(Decimal::ZERO);
                acc.point.gestionadas += 1;
                acc.loaded += value;
                if d.pipeline == derive::PIPELINE_RADICADA {
                    acc.point.radicadas += 1;
                    acc.filed += value;
                }
                if row.fecha >= from_day && row.fecha <= to_day {
                    kpis.add(&d);
                }
            }
            InvoiceState::Excluded => acc.point.excluidas += 1,
            InvoiceState::ManualReview => acc.point.revision_manual += 1,
        }
    }
    let mut cargues: Vec<CarguePoint> = points
        .into_values()
        .map(|mut a| {
            a.point.valor_cargado = plain_amount(a.loaded);
            a.point.valor_radicado = plain_amount(a.filed);
            a.point
        })
        .collect();
    sort_points(&mut cargues);
    Ok(Response::json(&Summary {
        desde: from_day,
        hasta: to_day,
        kpis,
        cargues,
    }))
}

/// Most recent first: by cargue start date, undated ones last, ties broken by
/// cargue id descending so the order is deterministic.
fn sort_points(points: &mut [CarguePoint]) {
    points.sort_by(|a, b| {
        match (a.fecha_inicio.is_empty(), b.fecha_inicio.is_empty()) {
            (true, false) => return std::cmp::Ordering::Greater,
            (false, true) => return std::cmp::Ordering::Less,
            _ => {}
        }
        b.fecha_inicio
            .cmp(&a.fecha_inicio)
            .then_with(|| b.cargue.cmp(&a.cargue))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(id: &str, start: &str) -> CarguePoint {
        CarguePoint {
            cargue: id.into(),
            nombre: String::new(),
            fecha_inicio: start.into(),
            total: 0,
            gestionadas: 0,
            excluidas: 0,
            revision_manual: 0,
            radicadas: 0,
            valor_cargado: String::new(),
            valor_radicado: String::new(),
        }
    }

    #[test]
    fn the_chart_is_newest_first_with_undated_last_and_ties_by_id_descending() {
        let mut pts = vec![
            point("A", "2026-09-01"),
            point("U", ""),
            point("B", "2026-09-26"),
            point("C", "2026-09-01"),
            point("V", ""),
        ];
        sort_points(&mut pts);
        let ids: Vec<&str> = pts.iter().map(|p| p.cargue.as_str()).collect();
        assert_eq!(ids, ["B", "C", "A", "V", "U"]);
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 9, 30).expect("date")
    }

    fn req(query: &[(&str, &str)]) -> Request {
        let mut r = Request::new("GET", "/api/resumen");
        for (k, v) in query {
            r = r.with_query(k, v);
        }
        r
    }

    #[test]
    fn the_default_range_is_the_last_thirty_days() {
        let (from, to) = range(&req(&[]), today()).expect("range");
        assert_eq!(
            (from.to_string(), to.to_string()),
            ("2026-09-01".into(), "2026-09-30".into())
        );
    }

    #[test]
    fn the_range_is_validated_like_go() {
        let bad = |q: &[(&str, &str)]| range(&req(q), today()).expect_err("error").message;
        assert_eq!(bad(&[("hasta", "2026-9-30")]), "--hasta must be YYYY-MM-DD");
        assert_eq!(bad(&[("desde", "x")]), "--desde must be YYYY-MM-DD");
        assert_eq!(
            bad(&[("desde", "2026-09-10"), ("hasta", "2026-09-09")]),
            "hasta es anterior a desde"
        );
        assert_eq!(
            bad(&[("desde", "2025-09-29"), ("hasta", "2026-09-30")]),
            "el rango no puede superar 366 días"
        );
        // 365 days apart is accepted; 366 is not.
        assert!(
            range(
                &req(&[("desde", "2025-09-30"), ("hasta", "2026-09-30")]),
                today()
            )
            .is_ok()
        );
    }
}
