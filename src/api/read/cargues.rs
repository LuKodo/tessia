//! `GET /api/cargues`: the latest 20 current cargues, most recently modified
//! first (Go `dashboard.ListCargues`). The answer goes through
//! [`derive::cached`](super::derive::cached).

use std::collections::HashMap;

use serde::Serialize;

use crate::api::read::derive::{
    self, ESTADO_CARGUE, cache_key, cached, estado_of, is_incident, rfc3339, status_from_counts,
    status_without_invoices,
};
use crate::api::{
    ApiState,
    errors::{ApiError, ApiResult},
    request::{Request, Response},
};
use crate::stores::dataset::dashboard::{LoadRow, NativeScope};
use crate::stores::dataset::{DatasetResult, DatasetStore};

/// How many cargues the list shows.
const LIST_LIMIT: i64 = 20;

/// One cargue of the list (`ui/src/types/api.ts` `Cargue`).
#[derive(Debug, Serialize)]
struct Cargue {
    id: String,
    nombre: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    fecha_inicio: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    fecha_fin: String,
    facturas: i64,
    excluidas: i64,
    revision_manual: i64,
    modificado_en: String,
    estado: &'static str,
}

pub fn list(state: &ApiState, req: &Request) -> ApiResult {
    cached(state, &cache_key("cargues", req), || compute(state))
}

fn compute(state: &ApiState) -> ApiResult {
    let store = DatasetStore::new(state.pool.clone());
    let answer = build(&store).map_err(|e| {
        tracing::error!(error = %e, "dashboard: list cargues");
        ApiError::internal("no se pudieron leer los cargues")
    })?;
    Ok(Response::json(&answer))
}

fn build(store: &DatasetStore) -> DatasetResult<Vec<Cargue>> {
    let loads = store.dashboard_loads(Some(LIST_LIMIT))?;
    let statuses = prepared_statuses(store, &loads)?;
    Ok(loads
        .into_iter()
        .map(|l| {
            let estado = if !l.prepared {
                "SIN_PREPARAR"
            } else {
                statuses
                    .get(&l.load_seq)
                    .copied()
                    .unwrap_or_else(|| status_without_invoices(l.excluded, l.manual_review))
            };
            Cargue {
                id: l.load_id,
                nombre: l.name,
                fecha_inicio: l.from_date.map(|d| d.to_string()).unwrap_or_default(),
                fecha_fin: l.to_date.map(|d| d.to_string()).unwrap_or_default(),
                facturas: l.invoices,
                excluidas: l.excluded,
                revision_manual: l.manual_review,
                modificado_en: rfc3339(l.modified_at),
                estado,
            }
        })
        .collect())
}

/// The status of every prepared load that holds prepared invoices, rolled up
/// from the derived estado of those invoices (Go `cargueStatus`).
fn prepared_statuses(
    store: &DatasetStore,
    loads: &[LoadRow],
) -> DatasetResult<HashMap<i64, &'static str>> {
    let seqs: Vec<i64> = loads
        .iter()
        .filter(|l| l.prepared && l.invoices > 0)
        .map(|l| l.load_seq)
        .collect();
    if seqs.is_empty() {
        return Ok(HashMap::new());
    }
    let names = store.dashboard_prepared_names(&seqs)?;
    let native = store.dashboard_native(NativeScope::Cargues(&seqs))?;
    // (total, terminated, incidents) per cargue.
    let mut counts: HashMap<i64, (usize, usize, usize)> = HashMap::new();
    for (seq, name) in &names {
        let estado = native.get(name).map_or(ESTADO_CARGUE, estado_of);
        let c = counts.entry(*seq).or_default();
        c.0 += 1;
        c.1 += usize::from(estado == derive::ESTADO_TERMINADO);
        c.2 += usize::from(is_incident(estado));
    }
    Ok(loads
        .iter()
        .filter(|l| l.prepared && l.invoices > 0)
        .map(|l| {
            let status = match counts.get(&l.load_seq) {
                Some(&(total, done, incidents)) => status_from_counts(total, done, incidents),
                None => status_without_invoices(l.excluded, l.manual_review),
            };
            (l.load_seq, status)
        })
        .collect())
}
