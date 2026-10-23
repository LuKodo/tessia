//! Read queries behind the dashboard (`GET /api/cargues`, `/api/resumen`,
//! `/api/seguimiento/consulta`, `/api/facturas/{f}/seguimiento`).
//!
//! The dashboard reads the *current* cargue of every load (see
//! [`DatasetStore::current_cargue`]) over the stored summary columns of
//! `dataset_factura` (see [`super::summary`]), plus the native state tables
//! (`radicacion_estado`, `proceso_estado`) and two side tables (`portal_cargue`,
//! `factura_valor_xml`). Everything here is read-only and returns raw facts;
//! the rules that turn them into estados, pipelines and alerts live in
//! `api::read::derive`.
//!
//! The native state is keyed by the normalized invoice, not by cargue, so one
//! invoice stored in two cargues shares its state. Only prepared invoices read
//! native state (Go `enrichRows`).

use std::collections::HashMap;

use chrono::{DateTime, NaiveDate, Utc};
use diesel::prelude::*;
use diesel::sql_query;
use diesel::sql_types::{Array, BigInt, Bool, Date, Int4, Nullable, Numeric, Text, Timestamptz};
use rust_decimal::Decimal;

use super::{DatasetResult, DatasetStore};

/// The current upload of every load, with the load ids the dashboard accepts
/// as a cargue id (Go `ValidCargueID`: no `/`, `\` or `..`).
const CURRENT_LOAD: &str = "g.seq = COALESCE(
        (SELECT a.cargue_seq FROM dataset_actual a WHERE a.load_id = g.id),
        (SELECT MAX(n.seq) FROM cargues n WHERE n.id = g.id))
    AND g.id <> '' AND g.id <> '..'
    AND position('/' in g.id) = 0 AND position(chr(92) in g.id) = 0";

/// Phases that generate the four artifacts, in table order (ZIP XML/PDF, RIPS
/// JSON, TXT CUV, ZIP CRC).
pub const ARTIFACT_PHASES: [&str; 4] = ["thefactory", "rips", "cuv", "soporte"];

/// One current load as the cargues list shows it.
#[derive(Debug, Clone, QueryableByName)]
pub struct LoadRow {
    #[diesel(sql_type = BigInt)]
    pub load_seq: i64,
    #[diesel(sql_type = Text)]
    pub load_id: String,
    /// `load_name` of the dataset, or the upload name while unprepared.
    #[diesel(sql_type = Text)]
    pub name: String,
    #[diesel(sql_type = Nullable<Date>)]
    pub from_date: Option<NaiveDate>,
    #[diesel(sql_type = Nullable<Date>)]
    pub to_date: Option<NaiveDate>,
    /// Preparation time, or the upload time while unprepared.
    #[diesel(sql_type = Timestamptz)]
    pub modified_at: DateTime<Utc>,
    #[diesel(sql_type = Bool)]
    pub prepared: bool,
    /// Prepared invoices.
    #[diesel(sql_type = BigInt)]
    pub invoices: i64,
    #[diesel(sql_type = BigInt)]
    pub excluded: i64,
    /// Everything that is neither prepared nor excluded.
    #[diesel(sql_type = BigInt)]
    pub manual_review: i64,
}

/// One invoice of a current, prepared cargue with its stored summary.
#[derive(Debug, Clone, QueryableByName)]
pub struct DatasetRow {
    #[diesel(sql_type = BigInt)]
    pub load_seq: i64,
    /// `cargues.id`, the cargue id of the API.
    #[diesel(sql_type = Text)]
    pub load_id: String,
    #[diesel(sql_type = Text)]
    pub load_name: String,
    #[diesel(sql_type = Nullable<Date>)]
    pub load_from: Option<NaiveDate>,
    #[diesel(sql_type = Nullable<Date>)]
    pub load_to: Option<NaiveDate>,
    #[diesel(sql_type = Text)]
    pub factura: String,
    #[diesel(sql_type = Text)]
    pub factura_norm: String,
    /// `PREPARADA`, `EXCLUIDA` or `REVISION_MANUAL`.
    #[diesel(sql_type = Text)]
    pub estado: String,
    #[diesel(sql_type = Text)]
    pub codigo: String,
    #[diesel(sql_type = Text)]
    pub motivo: String,
    #[diesel(sql_type = Text)]
    pub id_mipres: String,
    #[diesel(sql_type = Text)]
    pub fecha: String,
    #[diesel(sql_type = Text)]
    pub ssc: String,
    #[diesel(sql_type = Text)]
    pub subplan: String,
    #[diesel(sql_type = Text)]
    pub autorizacion: String,
    #[diesel(sql_type = Nullable<Numeric>)]
    pub valor: Option<Decimal>,
}

const ROW_COLUMNS: &str = "g.seq AS load_seq, g.id AS load_id, p.load_name, p.load_from, p.load_to,
    f.factura, f.factura_norm, f.estado, f.codigo, f.motivo, f.id_mipres,
    f.fecha_factura_txt AS fecha, f.ssc, f.subplan, f.autorizacion, f.valor_convenio AS valor";

/// The filing row of an invoice.
#[derive(Debug, Clone)]
pub struct RadRaw {
    pub step: i32,
    pub status: String,
    pub updated_at: DateTime<Utc>,
    /// `last_detail`, present only when the rules can read it: the status is
    /// `BLOCKED` (value block codes) or the text holds `22` (the `RFFE022`
    /// expiry code). Other details are not transferred.
    pub detail: Option<String>,
}

/// What the native tables say about one invoice.
#[derive(Debug, Clone, Default)]
pub struct NativeRaw {
    pub rad: Option<RadRaw>,
    /// `proceso_estado.estado` of [`ARTIFACT_PHASES`], in that order.
    pub artifacts: [Option<String>; 4],
    /// Any `proceso_estado` row of any phase exists.
    pub has_phase_rows: bool,
    /// Latest `updated_at` over all its `proceso_estado` rows.
    pub phase_updated: Option<DateTime<Utc>>,
}

/// Which invoices a native read covers.
#[derive(Debug, Clone, Copy)]
pub enum NativeScope<'a> {
    /// The prepared invoices of every current cargue.
    AllCurrent,
    /// The prepared invoices of these cargues (`dataset_factura.cargue_seq`).
    Cargues(&'a [i64]),
    /// These normalized invoice names.
    Names(&'a [String]),
}

/// One `proceso_estado` row.
#[derive(Debug, Clone, QueryableByName)]
pub struct PhaseRaw {
    #[diesel(sql_type = Text)]
    pub num_factura: String,
    #[diesel(sql_type = Text)]
    pub fase: String,
    #[diesel(sql_type = Text)]
    pub estado: String,
    #[diesel(sql_type = Text)]
    pub motivo: String,
    #[diesel(sql_type = Timestamptz)]
    pub updated_at: DateTime<Utc>,
}

/// The free texts of an invoice's native rows.
#[derive(Debug, Clone, Default)]
pub struct DetailRaw {
    /// Whole `radicacion_estado.last_detail`.
    pub rad_detail: Option<String>,
    pub phases: Vec<PhaseRaw>,
}

/// The `portal_cargue` summary of an invoice.
#[derive(Debug, Clone, QueryableByName)]
pub struct PortalRaw {
    #[diesel(sql_type = Text)]
    pub num_factura: String,
    #[diesel(sql_type = Text)]
    pub estado: String,
    #[diesel(sql_type = Int4)]
    pub advertencias: i32,
    #[diesel(sql_type = Int4)]
    pub errores: i32,
    #[diesel(sql_type = Bool)]
    pub es_final: bool,
}

/// What the rows of one page need besides the dataset and the native facts.
#[derive(Debug, Clone, Default)]
pub struct PageExtras {
    pub details: HashMap<String, DetailRaw>,
    /// `factura_valor_xml.xml_valor` by normalized invoice.
    pub xml: HashMap<String, Decimal>,
    pub portal: HashMap<String, PortalRaw>,
}

/// An invoice looked up for the detail route.
#[derive(Debug, Clone)]
pub enum InvoiceHit {
    /// The cargue id was given and no current prepared cargue has it.
    NoCargue,
    NotFound,
    Found(Box<DatasetRow>),
}

#[derive(QueryableByName)]
struct RadQ {
    #[diesel(sql_type = Text)]
    invoice: String,
    #[diesel(sql_type = Int4)]
    step: i32,
    #[diesel(sql_type = Text)]
    status: String,
    #[diesel(sql_type = Timestamptz)]
    updated_at: DateTime<Utc>,
    #[diesel(sql_type = Nullable<Text>)]
    detail: Option<String>,
}

#[derive(QueryableByName)]
struct PhaseAggQ {
    #[diesel(sql_type = Text)]
    num_factura: String,
    #[diesel(sql_type = Timestamptz)]
    updated_at: DateTime<Utc>,
    #[diesel(sql_type = Nullable<Text>)]
    a1: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    a2: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    a3: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    a4: Option<String>,
}

#[derive(QueryableByName)]
struct RadDetailQ {
    #[diesel(sql_type = Text)]
    invoice: String,
    #[diesel(sql_type = Nullable<Text>)]
    last_detail: Option<String>,
}

#[derive(QueryableByName)]
struct XmlQ {
    #[diesel(sql_type = Text)]
    num_factura: String,
    #[diesel(sql_type = Numeric)]
    xml_valor: Decimal,
}

#[derive(QueryableByName)]
struct NameQ {
    #[diesel(sql_type = BigInt)]
    cargue_seq: i64,
    #[diesel(sql_type = Text)]
    factura_norm: String,
}

/// The `WHERE` fragment that selects the invoices of `scope` on `column`.
fn invoice_filter(scope: NativeScope<'_>, column: &str) -> String {
    match scope {
        NativeScope::AllCurrent => format!(
            "{column} IN (SELECT f.factura_norm FROM dataset_factura f
                 WHERE f.estado = 'PREPARADA'
                   AND f.cargue_seq IN (SELECT g.seq FROM cargues g WHERE {CURRENT_LOAD}))"
        ),
        NativeScope::Cargues(_) => format!(
            "{column} IN (SELECT f.factura_norm FROM dataset_factura f
                 WHERE f.estado = 'PREPARADA' AND f.cargue_seq = ANY($1))"
        ),
        NativeScope::Names(_) => format!("{column} = ANY($1)"),
    }
}

/// Runs `sql` binding `$1` as the scope asks (no bind for `AllCurrent`).
fn load_scoped<T>(conn: &mut PgConnection, sql: &str, scope: NativeScope<'_>) -> QueryResult<Vec<T>>
where
    T: diesel::deserialize::QueryableByName<diesel::pg::Pg> + 'static,
{
    match scope {
        NativeScope::AllCurrent => sql_query(sql).load(conn),
        NativeScope::Cargues(seqs) => sql_query(sql)
            .bind::<Array<BigInt>, _>(seqs.to_vec())
            .load(conn),
        NativeScope::Names(names) => sql_query(sql)
            .bind::<Array<Text>, _>(names.to_vec())
            .load(conn),
    }
}

impl DatasetStore {
    /// The current upload of every load, newest first (`modified_at`
    /// descending, then id), at most `limit` of them. Loads without a prepared
    /// dataset are included (`prepared == false`, counts zero).
    pub fn dashboard_loads(&self, limit: Option<i64>) -> DatasetResult<Vec<LoadRow>> {
        let sql = format!(
            "SELECT g.seq AS load_seq, g.id AS load_id, COALESCE(p.load_name, g.name) AS name,
                    CASE WHEN p.cargue_seq IS NULL THEN g.date_from ELSE p.load_from END AS from_date,
                    CASE WHEN p.cargue_seq IS NULL THEN g.date_to ELSE p.load_to END AS to_date,
                    COALESCE(p.prepared_at, g.created_at) AS modified_at,
                    (p.cargue_seq IS NOT NULL) AS prepared,
                    COUNT(f.factura) FILTER (WHERE f.estado = 'PREPARADA') AS invoices,
                    COUNT(f.factura) FILTER (WHERE f.estado = 'EXCLUIDA') AS excluded,
                    COUNT(f.factura) FILTER (WHERE f.estado NOT IN ('PREPARADA', 'EXCLUIDA')) AS manual_review
             FROM cargues g
             LEFT JOIN dataset_preparacion p ON p.cargue_seq = g.seq
             LEFT JOIN dataset_factura f ON f.cargue_seq = g.seq
             WHERE {CURRENT_LOAD}
             GROUP BY g.seq, g.id, g.name, g.date_from, g.date_to, g.created_at,
                      p.cargue_seq, p.load_name, p.load_from, p.load_to, p.prepared_at
             ORDER BY modified_at DESC, g.id
             LIMIT $1"
        );
        Ok(self.pool.run("dashboard loads", |conn| {
            Ok(sql_query(&sql)
                .bind::<Nullable<BigInt>, _>(limit)
                .load::<LoadRow>(conn)?)
        })?)
    }

    /// Every invoice of every current prepared cargue (any state) with its
    /// stored summary. Order is unspecified.
    pub fn dashboard_rows(&self) -> DatasetResult<Vec<DatasetRow>> {
        let sql = format!(
            "SELECT {ROW_COLUMNS}
             FROM cargues g
             JOIN dataset_preparacion p ON p.cargue_seq = g.seq
             JOIN dataset_factura f ON f.cargue_seq = g.seq
             WHERE {CURRENT_LOAD}"
        );
        Ok(self.pool.run("dashboard rows", |conn| {
            Ok(sql_query(&sql).load::<DatasetRow>(conn)?)
        })?)
    }

    /// `(cargue_seq, normalized name)` of the prepared invoices of `seqs`.
    pub fn dashboard_prepared_names(&self, seqs: &[i64]) -> DatasetResult<Vec<(i64, String)>> {
        Ok(self.pool.run("dashboard prepared names", |conn| {
            let rows = sql_query(
                "SELECT cargue_seq, factura_norm FROM dataset_factura
                 WHERE estado = 'PREPARADA' AND cargue_seq = ANY($1)",
            )
            .bind::<Array<BigInt>, _>(seqs.to_vec())
            .load::<NameQ>(conn)?;
            Ok(rows
                .into_iter()
                .map(|r| (r.cargue_seq, r.factura_norm))
                .collect())
        })?)
    }

    /// The native facts of the invoices in `scope`, by normalized invoice. An
    /// invoice with no row in either table is absent.
    pub fn dashboard_native(
        &self,
        scope: NativeScope<'_>,
    ) -> DatasetResult<HashMap<String, NativeRaw>> {
        let rad_sql = format!(
            "SELECT r.invoice, r.step::int4 AS step, r.status, r.updated_at,
                    CASE WHEN r.status = 'BLOCKED' OR position('22' in r.last_detail) > 0
                         THEN r.last_detail END AS detail
             FROM radicacion_estado r WHERE {}",
            invoice_filter(scope, "r.invoice")
        );
        let phase_sql = format!(
            "SELECT p.num_factura, max(p.updated_at) AS updated_at,
                    max(p.estado) FILTER (WHERE p.fase = 'thefactory') AS a1,
                    max(p.estado) FILTER (WHERE p.fase = 'rips') AS a2,
                    max(p.estado) FILTER (WHERE p.fase = 'cuv') AS a3,
                    max(p.estado) FILTER (WHERE p.fase = 'soporte') AS a4
             FROM proceso_estado p WHERE {} GROUP BY p.num_factura",
            invoice_filter(scope, "p.num_factura")
        );
        let (rad, phases) = self.pool.run("dashboard native state", |conn| {
            let rad: Vec<RadQ> = load_scoped(conn, &rad_sql, scope)?;
            let phases: Vec<PhaseAggQ> = load_scoped(conn, &phase_sql, scope)?;
            Ok((rad, phases))
        })?;
        let mut out: HashMap<String, NativeRaw> = HashMap::with_capacity(rad.len() + phases.len());
        for r in rad {
            out.entry(r.invoice).or_default().rad = Some(RadRaw {
                step: r.step,
                status: r.status,
                updated_at: r.updated_at,
                detail: r.detail,
            });
        }
        for p in phases {
            let n = out.entry(p.num_factura).or_default();
            n.has_phase_rows = true;
            n.phase_updated = Some(p.updated_at);
            n.artifacts = [p.a1, p.a2, p.a3, p.a4];
        }
        Ok(out)
    }

    /// The texts, XML totals and portal summaries of the (normalized) invoices
    /// of one page.
    pub fn dashboard_extras(&self, names: &[String]) -> DatasetResult<PageExtras> {
        if names.is_empty() {
            return Ok(PageExtras::default());
        }
        let names = names.to_vec();
        Ok(self.pool.run("dashboard page extras", |conn| {
            let bind = |sql: &str| sql_query(sql.to_owned()).bind::<Array<Text>, _>(names.clone());
            let rad: Vec<RadDetailQ> =
                bind("SELECT invoice, last_detail FROM radicacion_estado WHERE invoice = ANY($1)")
                    .load(conn)?;
            let phases: Vec<PhaseRaw> = bind(
                "SELECT num_factura, fase, estado, motivo, updated_at
                 FROM proceso_estado WHERE num_factura = ANY($1)",
            )
            .load(conn)?;
            let xml: Vec<XmlQ> = bind(
                "SELECT num_factura, xml_valor FROM factura_valor_xml
                 WHERE num_factura = ANY($1) AND xml_valor IS NOT NULL",
            )
            .load(conn)?;
            let portal: Vec<PortalRaw> = bind(
                "SELECT num_factura, estado, total_advertencias AS advertencias,
                        total_errores AS errores, es_final
                 FROM portal_cargue WHERE num_factura = ANY($1)",
            )
            .load(conn)?;
            let mut extras = PageExtras::default();
            for r in rad {
                extras.details.entry(r.invoice).or_default().rad_detail = r.last_detail;
            }
            for p in phases {
                extras
                    .details
                    .entry(p.num_factura.clone())
                    .or_default()
                    .phases
                    .push(p);
            }
            extras.xml = xml
                .into_iter()
                .map(|x| (x.num_factura, x.xml_valor))
                .collect();
            extras.portal = portal
                .into_iter()
                .map(|p| (p.num_factura.clone(), p))
                .collect();
            Ok(extras)
        })?)
    }

    /// Looks up the normalized invoice `norm` in the current prepared cargues:
    /// in the load `load_id` when given, else in the newest one (by
    /// preparation time) that holds it (Go `TrackingInvoice`).
    pub fn dashboard_invoice(
        &self,
        norm: &str,
        load_id: Option<&str>,
    ) -> DatasetResult<InvoiceHit> {
        let sql = format!(
            "SELECT {ROW_COLUMNS}
             FROM cargues g
             JOIN dataset_preparacion p ON p.cargue_seq = g.seq
             JOIN dataset_factura f ON f.cargue_seq = g.seq
             WHERE {CURRENT_LOAD} AND f.factura_norm = $1
               AND ($2::text IS NULL OR g.id = $2)
             ORDER BY p.prepared_at DESC, g.seq DESC
             LIMIT 1"
        );
        let exists = format!(
            "SELECT g.seq AS load_seq FROM cargues g
             JOIN dataset_preparacion p ON p.cargue_seq = g.seq
             WHERE {CURRENT_LOAD} AND g.id = $1"
        );
        Ok(self.pool.run("dashboard invoice", |conn| {
            let found: Option<DatasetRow> = sql_query(&sql)
                .bind::<Text, _>(norm)
                .bind::<Nullable<Text>, _>(load_id)
                .get_result(conn)
                .optional()?;
            if let Some(row) = found {
                return Ok(InvoiceHit::Found(Box::new(row)));
            }
            let Some(load) = load_id else {
                return Ok(InvoiceHit::NotFound);
            };
            let seqs: Vec<LoadSeq> = sql_query(&exists).bind::<Text, _>(load).load(conn)?;
            Ok(if seqs.is_empty() {
                InvoiceHit::NoCargue
            } else {
                InvoiceHit::NotFound
            })
        })?)
    }
}

#[derive(QueryableByName)]
struct LoadSeq {
    #[diesel(sql_type = BigInt)]
    #[allow(dead_code)]
    load_seq: i64,
}
