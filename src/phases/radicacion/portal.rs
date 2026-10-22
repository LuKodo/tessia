//! Portal follow-up of the RIPS JSON upload (`radicacion run --portal-follow`;
//! Go `internal/portalcargue` + `mysqlstore/portal.go`).
//!
//! Step 3 is only confirmed as "uploaded"; Mutual Ser then validates the JSON
//! and reports a state (e.g. `VALIDADO_CON_ADVERTENCIA`) and a list of
//! findings. A background [`Follower`] reads both from the portal and stores
//! them in `portal_cargue` / `portal_cargue_detalle`. It never delays or fails
//! the radicación.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::{DateTime, NaiveDateTime, Utc};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use diesel::prelude::*;
use diesel::sql_types::{Bool, Int4, Nullable, Text, Timestamptz};

use super::api::ApiError;
use crate::db::{DbPool, DbResult};
use crate::domain::normalize_invoice;

diesel::table! {
    portal_cargue_detalle (id) {
        id -> Int8,
        num_factura -> Text,
        orden -> Int4,
        codigo -> Text,
        tipo -> Text,
        campo -> Text,
        detalle -> Nullable<Text>,
    }
}

diesel::table! {
    portal_hallazgo (codigo) {
        codigo -> Text,
        detalle -> Nullable<Text>,
        genera_glosa -> Bool,
        devolucion -> Bool,
        descripcion -> Nullable<Text>,
        created_at -> Timestamptz,
        updated_at -> Timestamptz,
    }
}

/// The portal row of one upload (`core/filtersAll` "contenido" item).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Record {
    pub invoice: String,
    pub status: String,
    pub remission: String,
    /// `fecha_cargue` as returned, e.g. `29-09-2026 17:01:26`.
    pub uploaded_at_text: String,
    /// The text parsed in America/Bogota (UTC-5, no DST).
    pub uploaded_at: Option<DateTime<Utc>>,
    pub user: String,
    pub services: String,
    pub modality: String,
    pub file_name: String,
    /// The portal `idArchivo` (feeds `findDetailsByFile`).
    pub file_id: String,
    /// The portal `idCargue`.
    pub upload_id: String,
    pub event_id: String,
    pub event_type: String,
    pub error_type: String,
    pub error_code: String,
    pub returned: bool,
    pub issue_date: String,
    pub receipt_name: String,
    /// Compact JSON of `documento_soporte` (`""` when absent).
    pub support_document: String,
    /// Compact JSON of the whole row.
    pub raw: String,
}

/// A validation finding of an upload.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Finding {
    pub code: String,
    pub kind: String,
    pub field: String,
    pub detail: String,
}

/// One entry of the catalog of portal findings (`portal_hallazgo`): the code
/// and detail come from the stored findings, the other three are edited by the
/// user.
#[derive(Debug, Clone, Default, PartialEq, Eq, QueryableByName)]
pub struct CatalogItem {
    #[diesel(sql_type = Text)]
    pub code: String,
    #[diesel(sql_type = Nullable<Text>)]
    detail: Option<String>,
    #[diesel(sql_type = Bool)]
    pub generates_glosa: bool,
    /// The payer returns the invoice because of this finding.
    #[diesel(sql_type = Bool)]
    pub returns: bool,
    #[diesel(sql_type = Nullable<Text>)]
    description: Option<String>,
}

impl CatalogItem {
    pub fn detail(&self) -> &str {
        self.detail.as_deref().unwrap_or_default()
    }

    pub fn description(&self) -> &str {
        self.description.as_deref().unwrap_or_default()
    }
}

/// What the portal reported at one query, ready to be stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub record: Record,
    pub findings: Vec<Finding>,
    pub warnings: i32,
    pub errors: i32,
    /// False when the portal has no row for the invoice yet.
    pub found: bool,
    /// The state will not change anymore and the findings were fetched.
    pub is_final: bool,
    pub queries: i32,
    pub queried_at: Option<DateTime<Utc>>,
}

/// Reads the portal.
pub trait PortalApi: Send + Sync {
    /// The row of the invoice's RIPS JSON upload.
    fn portal_record(&self, invoice: &str) -> Result<Option<Record>, ApiError>;
    /// The validation findings of an uploaded file.
    fn file_findings(&self, file_id: &str) -> Result<Vec<Finding>, ApiError>;
}

/// Upper-case fragments that mark a portal state as still being processed.
/// ASSUMPTION (to confirm with real data, same as Go): only
/// `VALIDADO_CON_ADVERTENCIA` is known, so any other state is final.
const IN_PROGRESS_MARKERS: [&str; 5] = ["PROCES", "PENDIENT", "COLA", "VALIDANDO", "CARGANDO"];

/// The portal state will not change anymore: not empty and with no
/// in-progress marker.
pub fn is_final(status: &str) -> bool {
    let s = status.trim().to_uppercase();
    !s.is_empty() && !IN_PROGRESS_MARKERS.iter().any(|m| s.contains(m))
}

/// `(warnings, errors)`: types containing ADVERT / ERROR or RECHAZ.
pub fn count_findings(findings: &[Finding]) -> (i32, i32) {
    let (mut warnings, mut errors) = (0, 0);
    for f in findings {
        let t = f.kind.to_uppercase();
        if t.contains("ADVERT") {
            warnings += 1;
        } else if t.contains("ERROR") || t.contains("RECHAZ") {
            errors += 1;
        }
    }
    (warnings, errors)
}

/// Parses a portal `fecha_cargue` (`dd-MM-yyyy HH:mm:ss`, America/Bogota).
pub fn parse_uploaded_at(text: &str) -> Option<DateTime<Utc>> {
    let local = NaiveDateTime::parse_from_str(text.trim(), "%d-%m-%Y %H:%M:%S").ok()?;
    Some((local + chrono::Duration::hours(5)).and_utc())
}

fn looks_like_rips_json(r: &Record) -> bool {
    r.file_name.trim().to_lowercase().ends_with(".json")
        || r.event_type.trim().eq_ignore_ascii_case("REGISTER_RIP")
}

/// Picks the portal row of `invoice` among `rows`: only rows of that invoice;
/// the ones that look like the RIPS JSON win, then the newest upload date
/// (unparsable dates lose), then the first in input order.
pub fn choose_record(rows: Vec<Record>, invoice: &str) -> Option<Record> {
    let mut best: Option<(Record, bool)> = None;
    for row in rows {
        if !row.invoice.trim().eq_ignore_ascii_case(invoice.trim()) {
            continue;
        }
        let is_json = looks_like_rips_json(&row);
        let better = match &best {
            None => true,
            Some((current, current_json)) => {
                (is_json && !current_json)
                    || (is_json == *current_json && row.uploaded_at > current.uploaded_at)
            }
        };
        if better {
            best = Some((row, is_json));
        }
    }
    best.map(|(row, _)| row)
}

/// One portal query for `invoice`. `prev` only feeds the query counter. A
/// missing portal row gives `found == false` and no error; an error fetching
/// the findings is returned and nothing must be saved.
pub fn fetch(
    api: &dyn PortalApi,
    invoice: &str,
    prev: Option<&Snapshot>,
    now: DateTime<Utc>,
) -> Result<Snapshot, ApiError> {
    let queries = prev.map_or(1, |p| p.queries + 1);
    let Some(record) = api.portal_record(invoice)? else {
        return Ok(Snapshot {
            queries,
            queried_at: Some(now),
            ..Snapshot::default()
        });
    };
    let findings = if record.file_id.is_empty() {
        Vec::new()
    } else {
        api.file_findings(&record.file_id)?
    };
    let (warnings, errors) = count_findings(&findings);
    Ok(Snapshot {
        is_final: is_final(&record.status),
        record,
        findings,
        warnings,
        errors,
        found: true,
        queries,
        queried_at: Some(now),
    })
}

#[derive(QueryableByName)]
struct HeaderRow {
    #[diesel(sql_type = Text)]
    estado: String,
    #[diesel(sql_type = Text)]
    remision: String,
    #[diesel(sql_type = Nullable<Timestamptz>)]
    fecha_cargue: Option<DateTime<Utc>>,
    #[diesel(sql_type = Text)]
    fecha_cargue_texto: String,
    #[diesel(sql_type = Text)]
    usuario: String,
    #[diesel(sql_type = Nullable<Text>)]
    servicios: Option<String>,
    #[diesel(sql_type = Text)]
    modalidad: String,
    #[diesel(sql_type = Text)]
    nombre_archivo: String,
    #[diesel(sql_type = Text)]
    id_archivo: String,
    #[diesel(sql_type = Text)]
    id_cargue: String,
    #[diesel(sql_type = Text)]
    id_evento: String,
    #[diesel(sql_type = Text)]
    tipo_evento: String,
    #[diesel(sql_type = Text)]
    tipo_error: String,
    #[diesel(sql_type = Text)]
    codigo_error: String,
    #[diesel(sql_type = Bool)]
    devuelto: bool,
    #[diesel(sql_type = Text)]
    fecha_expedicion: String,
    #[diesel(sql_type = Text)]
    receipt_name: String,
    #[diesel(sql_type = Nullable<Text>)]
    documento_soporte: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    raw_json: Option<String>,
    #[diesel(sql_type = Int4)]
    total_advertencias: i32,
    #[diesel(sql_type = Int4)]
    total_errores: i32,
    #[diesel(sql_type = Bool)]
    es_final: bool,
    #[diesel(sql_type = Int4)]
    consultas: i32,
    #[diesel(sql_type = Nullable<Timestamptz>)]
    consultado_en: Option<DateTime<Utc>>,
}

const HEADER_UPSERT: &str = "INSERT INTO portal_cargue \
 (num_factura, estado, remision, fecha_cargue, fecha_cargue_texto, usuario, servicios, modalidad, \
  nombre_archivo, id_archivo, id_cargue, id_evento, tipo_evento, tipo_error, codigo_error, devuelto, \
  fecha_expedicion, receipt_name, documento_soporte, raw_json, total_advertencias, total_errores, \
  total_hallazgos, es_final, consultas, consultado_en) \
 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22,$23,$24,$25,$26) \
 ON CONFLICT (num_factura) DO UPDATE SET estado=EXCLUDED.estado, remision=EXCLUDED.remision, \
  fecha_cargue=EXCLUDED.fecha_cargue, fecha_cargue_texto=EXCLUDED.fecha_cargue_texto, \
  usuario=EXCLUDED.usuario, servicios=EXCLUDED.servicios, modalidad=EXCLUDED.modalidad, \
  nombre_archivo=EXCLUDED.nombre_archivo, id_archivo=EXCLUDED.id_archivo, id_cargue=EXCLUDED.id_cargue, \
  id_evento=EXCLUDED.id_evento, tipo_evento=EXCLUDED.tipo_evento, tipo_error=EXCLUDED.tipo_error, \
  codigo_error=EXCLUDED.codigo_error, devuelto=EXCLUDED.devuelto, \
  fecha_expedicion=EXCLUDED.fecha_expedicion, receipt_name=EXCLUDED.receipt_name, \
  documento_soporte=EXCLUDED.documento_soporte, raw_json=EXCLUDED.raw_json, \
  total_advertencias=EXCLUDED.total_advertencias, total_errores=EXCLUDED.total_errores, \
  total_hallazgos=EXCLUDED.total_hallazgos, es_final=EXCLUDED.es_final, consultas=EXCLUDED.consultas, \
  consultado_en=EXCLUDED.consultado_en, updated_at=now()";

const HEADER_SELECT: &str = "SELECT estado, remision, fecha_cargue, fecha_cargue_texto, usuario, \
 servicios, modalidad, nombre_archivo, id_archivo, id_cargue, id_evento, tipo_evento, tipo_error, \
 codigo_error, devuelto, fecha_expedicion, receipt_name, documento_soporte, raw_json, \
 total_advertencias, total_errores, es_final, consultas, consultado_en \
 FROM portal_cargue WHERE num_factura = $1";

/// Rows of one multi-row INSERT of findings.
const FINDINGS_BATCH: usize = 200;

/// Longest finding code kept in the catalog (characters).
const MAX_CODE_CHARS: usize = 100;

const CATALOG_COLUMNS: &str = "codigo AS code, detalle AS detail, genera_glosa AS generates_glosa, \
 devolucion AS returns, descripcion AS description";

fn non_empty(text: &str) -> Option<&str> {
    (!text.is_empty()).then_some(text)
}

/// `portal_cargue` (one row per invoice) and `portal_cargue_detalle` (its findings).
#[derive(Clone)]
pub struct PortalStore {
    pool: DbPool,
}

impl PortalStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Upserts the header and replaces the findings in one transaction. A
    /// snapshot with `found == false` is not written ("not found yet" must
    /// never overwrite a good record).
    pub fn save(&self, snapshot: &Snapshot) -> DbResult<()> {
        if !snapshot.found {
            return Ok(());
        }
        let invoice = normalize_invoice(&snapshot.record.invoice);
        if invoice.is_empty() {
            return Err(crate::db::DbError::other("registro del portal sin factura"));
        }
        self.pool.transaction("save portal record", |conn| {
            let r = &snapshot.record;
            diesel::sql_query(HEADER_UPSERT)
                .bind::<Text, _>(&invoice)
                .bind::<Text, _>(&r.status)
                .bind::<Text, _>(&r.remission)
                .bind::<Nullable<Timestamptz>, _>(r.uploaded_at)
                .bind::<Text, _>(&r.uploaded_at_text)
                .bind::<Text, _>(&r.user)
                .bind::<Nullable<Text>, _>(non_empty(&r.services))
                .bind::<Text, _>(&r.modality)
                .bind::<Text, _>(&r.file_name)
                .bind::<Text, _>(&r.file_id)
                .bind::<Text, _>(&r.upload_id)
                .bind::<Text, _>(&r.event_id)
                .bind::<Text, _>(&r.event_type)
                .bind::<Text, _>(&r.error_type)
                .bind::<Text, _>(&r.error_code)
                .bind::<Bool, _>(r.returned)
                .bind::<Text, _>(&r.issue_date)
                .bind::<Text, _>(&r.receipt_name)
                .bind::<Nullable<Text>, _>(non_empty(&r.support_document))
                .bind::<Nullable<Text>, _>(non_empty(&r.raw))
                .bind::<Int4, _>(snapshot.warnings)
                .bind::<Int4, _>(snapshot.errors)
                .bind::<Int4, _>(snapshot.findings.len() as i32)
                .bind::<Bool, _>(snapshot.is_final)
                .bind::<Int4, _>(snapshot.queries)
                .bind::<Nullable<Timestamptz>, _>(snapshot.queried_at)
                .execute(conn)?;
            use portal_cargue_detalle as d;
            diesel::delete(d::table.filter(d::num_factura.eq(&invoice))).execute(conn)?;
            for (batch, part) in snapshot.findings.chunks(FINDINGS_BATCH).enumerate() {
                let rows: Vec<_> = part
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        (
                            d::num_factura.eq(&invoice),
                            d::orden.eq((batch * FINDINGS_BATCH + i) as i32),
                            d::codigo.eq(&f.code),
                            d::tipo.eq(&f.kind),
                            d::campo.eq(&f.field),
                            d::detalle.eq(Some(&f.detail)),
                        )
                    })
                    .collect();
                diesel::insert_into(d::table).values(&rows).execute(conn)?;
            }
            register_catalog(conn, &snapshot.findings)
        })
    }

    /// The catalog of portal findings ordered by code.
    pub fn catalog(&self) -> DbResult<Vec<CatalogItem>> {
        self.pool.run("list findings catalog", |conn| {
            Ok(diesel::sql_query(format!(
                "SELECT {CATALOG_COLUMNS} FROM portal_hallazgo ORDER BY codigo COLLATE \"C\""
            ))
            .load::<CatalogItem>(conn)?)
        })
    }

    /// Sets `genera_glosa` and `descripcion` (stored trimmed, empty as NULL) of
    /// an existing code; `devolucion == None` keeps the stored value, so a
    /// client that does not know the field never resets it. Returns the stored
    /// entry, or `None` when the code is not in the catalog (entries are only
    /// created by the portal queries).
    pub fn update_catalog(
        &self,
        code: &str,
        generates_glosa: bool,
        returns: Option<bool>,
        description: &str,
    ) -> DbResult<Option<CatalogItem>> {
        let description = non_empty(description.trim());
        self.pool.run("update findings catalog", |conn| {
            Ok(diesel::sql_query(format!(
                "UPDATE portal_hallazgo SET genera_glosa = $1, descripcion = $2, \
                 devolucion = COALESCE($3, devolucion), updated_at = now() \
                 WHERE codigo = $4 RETURNING {CATALOG_COLUMNS}"
            ))
            .bind::<Bool, _>(generates_glosa)
            .bind::<Nullable<Text>, _>(description)
            .bind::<Nullable<Bool>, _>(returns)
            .bind::<Text, _>(code)
            .get_result::<CatalogItem>(conn)
            .optional()?)
        })
    }

    /// The stored snapshot of `invoice`, findings included.
    pub fn get(&self, invoice: &str) -> DbResult<Option<Snapshot>> {
        let invoice = normalize_invoice(invoice);
        self.pool.run("get portal record", |conn| {
            let header: Option<HeaderRow> = diesel::sql_query(HEADER_SELECT)
                .bind::<Text, _>(&invoice)
                .get_result(conn)
                .optional()?;
            let Some(h) = header else { return Ok(None) };
            use portal_cargue_detalle as d;
            let findings: Vec<(String, String, String, Option<String>)> = d::table
                .filter(d::num_factura.eq(&invoice))
                .order((d::orden.asc(), d::id.asc()))
                .select((d::codigo, d::tipo, d::campo, d::detalle))
                .load(conn)?;
            Ok(Some(Snapshot {
                record: Record {
                    invoice: invoice.clone(),
                    status: h.estado,
                    remission: h.remision,
                    uploaded_at_text: h.fecha_cargue_texto,
                    uploaded_at: h.fecha_cargue,
                    user: h.usuario,
                    services: h.servicios.unwrap_or_default(),
                    modality: h.modalidad,
                    file_name: h.nombre_archivo,
                    file_id: h.id_archivo,
                    upload_id: h.id_cargue,
                    event_id: h.id_evento,
                    event_type: h.tipo_evento,
                    error_type: h.tipo_error,
                    error_code: h.codigo_error,
                    returned: h.devuelto,
                    issue_date: h.fecha_expedicion,
                    receipt_name: h.receipt_name,
                    support_document: h.documento_soporte.unwrap_or_default(),
                    raw: h.raw_json.unwrap_or_default(),
                },
                findings: findings
                    .into_iter()
                    .map(|(code, kind, field, detail)| Finding {
                        code,
                        kind,
                        field,
                        detail: detail.unwrap_or_default(),
                    })
                    .collect(),
                warnings: h.total_advertencias,
                errors: h.total_errores,
                found: true,
                is_final: h.es_final,
                queries: h.consultas,
                queried_at: h.consultado_en,
            }))
        })
    }
}

/// Adds the distinct, non-empty finding codes to the catalog with
/// `ON CONFLICT DO NOTHING`, so an existing code keeps its detail, `genera_glosa`
/// and `descripcion` (the first detail ever seen wins). Runs in the caller's
/// transaction. Codes are inserted in ascending order, so two concurrent saves
/// with overlapping codes take their key locks in the same order and never
/// deadlock.
fn register_catalog(conn: &mut PgConnection, findings: &[Finding]) -> DbResult<()> {
    let mut first_detail: std::collections::BTreeMap<String, &str> = Default::default();
    for f in findings {
        let code: String = f.code.trim().chars().take(MAX_CODE_CHARS).collect();
        if !code.is_empty() {
            first_detail.entry(code).or_insert(f.detail.as_str());
        }
    }
    let entries: Vec<(&String, &&str)> = first_detail.iter().collect();
    use portal_hallazgo as h;
    for part in entries.chunks(FINDINGS_BATCH) {
        let rows: Vec<_> = part
            .iter()
            .map(|(code, detail)| (h::codigo.eq(code.as_str()), h::detalle.eq(Some(**detail))))
            .collect();
        diesel::insert_into(h::table)
            .values(&rows)
            .on_conflict_do_nothing()
            .execute(conn)?;
    }
    Ok(())
}

/// Wait before each portal query of one invoice: 10 s, 15 s and 30 s, then
/// 60 s repeated (12 rounds, about 10 minutes at most).
pub fn default_schedule() -> Vec<Duration> {
    let mut schedule = vec![
        Duration::from_secs(10),
        Duration::from_secs(15),
        Duration::from_secs(30),
    ];
    schedule.extend(std::iter::repeat_n(Duration::from_secs(60), 9));
    schedule
}

/// How long a finished radicación waits for the follower.
pub const DRAIN: Duration = Duration::from_secs(60);

const WORKERS: usize = 3;
const QUEUE_SIZE: usize = 1024;

struct FollowerShared {
    api: Arc<dyn PortalApi>,
    store: PortalStore,
    schedule: Vec<Duration>,
    cancel: Receiver<()>,
    cancelled: AtomicBool,
}

/// Asks the portal for the state and findings of the RIPS JSON of the invoices
/// it is given, in the background, until the state is final or the schedule
/// ends, and stores every result it finds. [`Follower::enqueue`] never blocks.
pub struct Follower {
    queue: Mutex<Option<Sender<String>>>,
    tracked: Arc<Mutex<HashSet<String>>>,
    cancel: Mutex<Option<Sender<()>>>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl Follower {
    /// Starts the 3 workers.
    pub fn start(api: Arc<dyn PortalApi>, store: PortalStore, schedule: Vec<Duration>) -> Follower {
        let (queue_tx, queue_rx) = bounded::<String>(QUEUE_SIZE);
        let (cancel_tx, cancel_rx) = bounded::<()>(0);
        let tracked = Arc::new(Mutex::new(HashSet::new()));
        let shared = Arc::new(FollowerShared {
            api,
            store,
            schedule: if schedule.is_empty() {
                default_schedule()
            } else {
                schedule
            },
            cancel: cancel_rx,
            cancelled: AtomicBool::new(false),
        });
        let workers = (0..WORKERS)
            .map(|_| {
                let (shared, queue, tracked) = (shared.clone(), queue_rx.clone(), tracked.clone());
                std::thread::spawn(move || {
                    for invoice in queue {
                        if !shared.cancelled.load(Ordering::SeqCst) {
                            follow_safely(&shared, &invoice);
                        }
                        tracked
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .remove(&invoice);
                    }
                })
            })
            .collect();
        Follower {
            queue: Mutex::new(Some(queue_tx)),
            tracked,
            cancel: Mutex::new(Some(cancel_tx)),
            workers: Mutex::new(workers),
        }
    }

    /// Asks for the invoice to be followed. Empty invoices, invoices already
    /// queued or being followed and invoices that do not fit the bounded queue
    /// are ignored.
    pub fn enqueue(&self, invoice: &str) {
        let invoice = normalize_invoice(invoice);
        if invoice.is_empty() {
            return;
        }
        let queue = self.queue.lock().unwrap_or_else(|p| p.into_inner());
        let Some(queue) = queue.as_ref() else { return };
        let mut tracked = self.tracked.lock().unwrap_or_else(|p| p.into_inner());
        if !tracked.insert(invoice.clone()) {
            return;
        }
        match queue.try_send(invoice.clone()) {
            Ok(()) => {}
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                tracked.remove(&invoice);
                tracing::debug!(invoice = %invoice, "portal follow queue full: invoice dropped");
            }
        }
    }

    /// Stops accepting work, waits up to `drain` for the queued and in-flight
    /// invoices, then cancels what is left. Idempotent.
    pub fn close(&self, drain: Duration) {
        drop(self.queue.lock().unwrap_or_else(|p| p.into_inner()).take());
        let workers: Vec<_> =
            std::mem::take(&mut *self.workers.lock().unwrap_or_else(|p| p.into_inner()));
        if workers.is_empty() {
            return;
        }
        let (done_tx, done_rx) = bounded::<()>(1);
        std::thread::spawn(move || {
            for worker in workers {
                let _ = worker.join();
            }
            let _ = done_tx.send(());
        });
        if done_rx.recv_timeout(drain).is_err() {
            drop(self.cancel.lock().unwrap_or_else(|p| p.into_inner()).take());
            let _ = done_rx.recv_timeout(Duration::from_secs(5));
        }
    }
}

impl Drop for Follower {
    fn drop(&mut self) {
        self.close(Duration::ZERO);
    }
}

/// Isolates a panic to the invoice being followed.
fn follow_safely(shared: &FollowerShared, invoice: &str) {
    let run = std::panic::AssertUnwindSafe(|| follow(shared, invoice));
    if std::panic::catch_unwind(run).is_err() {
        tracing::error!(invoice, "portal follow panicked");
    }
}

/// Polls the portal for one invoice following the schedule. Errors are logged
/// without payloads (the portal answer carries the uploader's e-mail).
fn follow(shared: &FollowerShared, invoice: &str) {
    let mut prev = match shared.store.get(invoice) {
        Ok(Some(stored)) if stored.is_final => return,
        Ok(stored) => stored,
        Err(err) => {
            tracing::warn!(invoice, error = %err, "portal follow: reading stored result failed");
            None
        }
    };
    for (round, wait) in shared.schedule.iter().enumerate() {
        // Disconnected = cancelled; a timeout is the wait elapsing.
        if !matches!(
            shared.cancel.recv_timeout(*wait),
            Err(crossbeam_channel::RecvTimeoutError::Timeout)
        ) {
            shared.cancelled.store(true, Ordering::SeqCst);
            return;
        }
        let snapshot = match fetch(shared.api.as_ref(), invoice, prev.as_ref(), Utc::now()) {
            Ok(snapshot) => snapshot,
            Err(err) => {
                tracing::warn!(invoice, round = round + 1, error = %err, "portal follow: query failed");
                continue;
            }
        };
        if !snapshot.found {
            tracing::debug!(
                invoice,
                round = round + 1,
                "portal follow: not on the portal yet"
            );
            continue;
        }
        prev = Some(snapshot.clone());
        if let Err(err) = shared.store.save(&snapshot) {
            tracing::warn!(invoice, error = %err, "portal follow: saving result failed");
            continue;
        }
        if snapshot.is_final {
            tracing::info!(
                invoice,
                status = %snapshot.record.status,
                warnings = snapshot.warnings,
                errors = snapshot.errors,
                "portal follow: final state stored"
            );
            return;
        }
    }
    tracing::debug!(
        invoice,
        "portal follow: schedule ended before a final state"
    );
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::db::test_support::TestDb;

    fn record(invoice: &str, status: &str) -> Record {
        Record {
            invoice: invoice.into(),
            status: status.into(),
            file_name: format!("{invoice}.json"),
            file_id: format!("f-{invoice}"),
            uploaded_at_text: "29-09-2026 17:01:26".into(),
            uploaded_at: parse_uploaded_at("29-09-2026 17:01:26"),
            user: "ops@example.org".into(),
            services: "[1]".into(),
            raw: format!(r#"{{"numero_factura":"{invoice}"}}"#),
            ..Record::default()
        }
    }

    fn finding(code: &str, kind: &str) -> Finding {
        Finding {
            code: code.into(),
            kind: kind.into(),
            field: "campo".into(),
            detail: format!("detalle {code}"),
        }
    }

    #[test]
    fn final_states_follow_the_marker_rule() {
        for (status, want) in [
            ("VALIDADO_CON_ADVERTENCIA", true),
            ("VALIDADO", true),
            ("RECHAZADO", true),
            ("cualquier cosa", true),
            ("", false),
            ("  ", false),
            ("EN PROCESO", false),
            ("pendiente", false),
            ("EN COLA", false),
            ("VALIDANDO", false),
            ("Cargando archivo", false),
        ] {
            assert_eq!(is_final(status), want, "{status:?}");
        }
    }

    #[test]
    fn findings_are_counted_by_type() {
        let list = [
            finding("1", "ADVERTENCIA"),
            finding("2", "advertencia"),
            finding("3", "ERROR"),
            finding("4", "Rechazo"),
            finding("5", "INFO"),
            finding("6", ""),
        ];
        assert_eq!(count_findings(&list), (2, 2));
        let with_rechaz = [finding("7", "RECHAZADO")];
        assert_eq!(count_findings(&with_rechaz), (0, 1));
    }

    #[test]
    fn upload_dates_are_bogota_time() {
        let at = parse_uploaded_at(" 29-09-2026 17:01:26 ").expect("date");
        assert_eq!(at.to_rfc3339(), "2026-09-29T22:01:26+00:00");
        assert_eq!(parse_uploaded_at(""), None);
        assert_eq!(parse_uploaded_at("2026-09-29"), None);
    }

    #[test]
    fn the_rips_json_row_wins_then_the_newest_upload() {
        let mut crc = record("FEV1", "A");
        crc.file_name = "CRC_FEV1.zip".into();
        crc.uploaded_at = parse_uploaded_at("30-09-2026 10:00:00");
        let mut old_json = record("fev1", "B");
        old_json.uploaded_at = parse_uploaded_at("28-09-2026 10:00:00");
        let new_json = record("FEV1", "C");
        let mut event = record("FEV1", "D");
        event.file_name = String::new();
        event.event_type = "register_rip".into();
        event.uploaded_at = None;
        let other = record("FEV2", "E");
        let chosen = choose_record(vec![crc.clone(), old_json, new_json, event, other], "FEV1");
        assert_eq!(chosen.map(|r| r.status), Some("C".to_owned()));
        assert_eq!(
            choose_record(vec![crc], "FEV1").map(|r| r.status),
            Some("A".to_owned())
        );
        assert_eq!(choose_record(vec![], "FEV1"), None);
    }

    /// Answers a script of portal rows, one per query.
    struct ScriptedApi {
        rows: Mutex<VecDeque<Result<Option<Record>, ApiError>>>,
        findings: Vec<Finding>,
        queries: AtomicUsize,
    }

    impl ScriptedApi {
        fn new(
            rows: Vec<Result<Option<Record>, ApiError>>,
            findings: Vec<Finding>,
        ) -> Arc<ScriptedApi> {
            Arc::new(ScriptedApi {
                rows: Mutex::new(rows.into()),
                findings,
                queries: AtomicUsize::new(0),
            })
        }
    }

    impl PortalApi for ScriptedApi {
        fn portal_record(&self, _invoice: &str) -> Result<Option<Record>, ApiError> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            let mut rows = self.rows.lock().expect("rows");
            match rows.len() {
                0 => Ok(None),
                1 => match rows.front() {
                    Some(Ok(row)) => Ok(row.clone()),
                    _ => rows.pop_front().unwrap_or(Ok(None)),
                },
                _ => rows.pop_front().unwrap_or(Ok(None)),
            }
        }

        fn file_findings(&self, _file_id: &str) -> Result<Vec<Finding>, ApiError> {
            Ok(self.findings.clone())
        }
    }

    fn quick() -> Vec<Duration> {
        vec![Duration::from_millis(5); 6]
    }

    #[test]
    fn store_round_trips_the_header_and_replaces_the_findings() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        assert_eq!(store.get("FEV1").expect("get"), None);
        let findings = vec![finding("A1", "ADVERTENCIA"), finding("E1", "ERROR")];
        let snap = fetch(
            ScriptedApi::new(
                vec![Ok(Some(record("fev-1", "VALIDADO_CON_ADVERTENCIA")))],
                findings.clone(),
            )
            .as_ref(),
            "FEV1",
            None,
            Utc::now(),
        )
        .expect("fetch");
        assert!(snap.found && snap.is_final);
        assert_eq!((snap.warnings, snap.errors, snap.queries), (1, 1, 1));
        let mut stored = snap.clone();
        stored.record.invoice = "FEV1".into();
        store.save(&stored).expect("save");
        let loaded = store.get("fev-1").expect("get").expect("row");
        assert_eq!(loaded.record, stored.record);
        assert_eq!(loaded.findings, findings);
        assert_eq!(
            (
                loaded.warnings,
                loaded.errors,
                loaded.queries,
                loaded.is_final
            ),
            (1, 1, 1, true)
        );
        assert!(loaded.queried_at.is_some());

        let mut again = stored.clone();
        again.findings = vec![finding("Z", "ERROR")];
        again.queries = 2;
        store.save(&again).expect("save again");
        let loaded = store.get("FEV1").expect("get").expect("row");
        assert_eq!(loaded.findings.len(), 1);
        assert_eq!(loaded.queries, 2);

        store
            .save(&Snapshot {
                found: false,
                ..again
            })
            .expect("not found is a no-op");
        assert_eq!(store.get("FEV1").expect("get").expect("row").queries, 2);
    }

    #[test]
    fn many_findings_are_stored_in_order() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        let mut snap = Snapshot {
            record: record("FEV1", "VALIDADO"),
            found: true,
            queries: 1,
            ..Snapshot::default()
        };
        snap.findings = (0..450)
            .map(|n| finding(&format!("C{n}"), "ERROR"))
            .collect();
        store.save(&snap).expect("save");
        let loaded = store.get("FEV1").expect("get").expect("row");
        assert_eq!(loaded.findings.len(), 450);
        assert_eq!(loaded.findings[0].code, "C0");
        assert_eq!(loaded.findings[449].code, "C449");
    }

    #[test]
    fn the_follower_polls_until_the_state_is_final_and_stores_it() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        let api = ScriptedApi::new(
            vec![
                Ok(None),
                Err(ApiError::Other("503".into())),
                Ok(Some(record("FEV1", "EN PROCESO"))),
                Ok(Some(record("FEV1", "VALIDADO_CON_ADVERTENCIA"))),
                Ok(Some(record("FEV1", "NO DEBE CONSULTARSE"))),
            ],
            vec![finding("A1", "ADVERTENCIA")],
        );
        let follower = Follower::start(api.clone(), store.clone(), quick());
        follower.enqueue("fev-1");
        follower.enqueue("FEV1");
        follower.close(Duration::from_secs(10));
        let stored = store.get("FEV1").expect("get").expect("stored");
        assert_eq!(stored.record.status, "VALIDADO_CON_ADVERTENCIA");
        assert!(stored.is_final);
        assert_eq!(
            stored.queries, 2,
            "the two rounds without a row do not count as stored queries"
        );
        assert_eq!(stored.warnings, 1);
        assert_eq!(
            api.queries.load(Ordering::SeqCst),
            4,
            "stops at the first final state"
        );
    }

    #[test]
    fn a_stored_final_result_is_never_queried_again() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        let done = Snapshot {
            record: record("FEV1", "VALIDADO"),
            found: true,
            is_final: true,
            queries: 3,
            ..Snapshot::default()
        };
        store.save(&done).expect("save");
        let api = ScriptedApi::new(vec![Ok(Some(record("FEV1", "OTRO")))], vec![]);
        let follower = Follower::start(api.clone(), store.clone(), quick());
        follower.enqueue("FEV1");
        follower.close(Duration::from_secs(10));
        assert_eq!(api.queries.load(Ordering::SeqCst), 0);
        assert_eq!(
            store.get("FEV1").expect("get").expect("row").record.status,
            "VALIDADO"
        );
    }

    #[test]
    fn a_never_final_state_ends_with_the_schedule_and_close_cancels_after_the_drain() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        let api = ScriptedApi::new(vec![Ok(Some(record("FEV1", "EN PROCESO")))], vec![]);
        let follower = Follower::start(
            api.clone(),
            store.clone(),
            vec![Duration::from_millis(5); 4],
        );
        follower.enqueue("FEV1");
        follower.close(Duration::from_secs(10));
        let stored = store.get("FEV1").expect("get").expect("row");
        assert!(!stored.is_final);
        assert_eq!(stored.queries, 4, "every round of the schedule was stored");

        let slow = Follower::start(
            ScriptedApi::new(vec![Ok(Some(record("FEV2", "EN PROCESO")))], vec![]),
            store.clone(),
            vec![Duration::from_secs(600)],
        );
        slow.enqueue("FEV2");
        let started = std::time::Instant::now();
        slow.close(Duration::from_millis(100));
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "the drain cap cancels the wait"
        );
        assert_eq!(store.get("FEV2").expect("get"), None);
        slow.enqueue("FEV3");
        slow.close(Duration::ZERO);
    }

    #[test]
    fn the_default_schedule_has_twelve_rounds() {
        let schedule = default_schedule();
        assert_eq!(schedule.len(), 12);
        assert_eq!(&schedule[..4], [10, 15, 30, 60].map(Duration::from_secs));
        assert_eq!(schedule.iter().sum::<Duration>(), Duration::from_secs(595));
    }

    fn found(invoice: &str, findings: Vec<Finding>) -> Snapshot {
        Snapshot {
            record: record(invoice, "VALIDADO_CON_ADVERTENCIA"),
            findings,
            found: true,
            queries: 1,
            ..Snapshot::default()
        }
    }

    fn detailed(code: &str, detail: &str) -> Finding {
        Finding {
            detail: detail.into(),
            ..finding(code, "ADVERTENCIA")
        }
    }

    #[test]
    fn save_registers_each_code_once_and_the_first_detail_wins() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        store
            .save(&found(
                "FEV1",
                vec![
                    detailed(" RVC012 ", "primero"),
                    detailed("RVC012", "segundo"),
                    detailed("", "sin codigo"),
                    detailed("AAA001", "otro"),
                ],
            ))
            .expect("save");
        store
            .save(&found("FEV2", vec![detailed("RVC012", "tercero")]))
            .expect("save again");
        let catalog = store.catalog().expect("catalog");
        let seen: Vec<(&str, &str)> = catalog
            .iter()
            .map(|c| (c.code.as_str(), c.detail()))
            .collect();
        assert_eq!(seen, [("AAA001", "otro"), ("RVC012", "primero")]);
        assert!(catalog.iter().all(|c| !c.generates_glosa && !c.returns));
        assert!(catalog.iter().all(|c| c.description().is_empty()));
    }

    #[test]
    fn saving_again_keeps_what_the_user_edited() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        store
            .save(&found("FEV1", vec![detailed("RVC012", "primero")]))
            .expect("save");
        store
            .update_catalog("RVC012", true, Some(true), "  nota  ")
            .expect("update")
            .expect("exists");
        store
            .save(&found("FEV1", vec![detailed("RVC012", "cambiado")]))
            .expect("save again");
        let item = &store.catalog().expect("catalog")[0];
        assert_eq!(
            (
                item.detail(),
                item.generates_glosa,
                item.returns,
                item.description()
            ),
            ("primero", true, true, "nota")
        );
    }

    #[test]
    fn a_save_without_findings_or_codes_adds_nothing() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        store.save(&found("FEV1", Vec::new())).expect("save");
        store
            .save(&found("FEV2", vec![detailed("  ", "x")]))
            .expect("save");
        assert!(store.catalog().expect("catalog").is_empty());
    }

    #[test]
    fn long_codes_are_cut_to_one_hundred_characters() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        store
            .save(&found("FEV1", vec![detailed(&"A".repeat(150), "x")]))
            .expect("save");
        assert_eq!(store.catalog().expect("catalog")[0].code, "A".repeat(100));
    }

    #[test]
    fn update_keeps_devolucion_when_absent_and_stores_empty_text_as_null() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        store
            .save(&found("FEV1", vec![detailed("RVC012", "d")]))
            .expect("save");
        let first = store
            .update_catalog("RVC012", true, Some(true), "texto")
            .expect("update")
            .expect("exists");
        assert!(first.generates_glosa && first.returns);
        let second = store
            .update_catalog("RVC012", false, None, "   ")
            .expect("update")
            .expect("exists");
        assert!(!second.generates_glosa && second.returns, "devolucion kept");
        assert_eq!(second.description, None, "blank text is NULL");
        let third = store
            .update_catalog("RVC012", false, Some(false), "")
            .expect("update")
            .expect("exists");
        assert!(!third.returns);
        assert_eq!(
            store
                .update_catalog("NOPE", true, None, "")
                .expect("update"),
            None
        );
    }

    #[test]
    fn the_catalog_is_ordered_by_code() {
        let db = TestDb::new();
        let store = PortalStore::new(db.pool());
        store
            .save(&found(
                "FEV1",
                vec![
                    detailed("B2", "x"),
                    detailed("A10", "x"),
                    detailed("A2", "x"),
                ],
            ))
            .expect("save");
        let codes: Vec<String> = store
            .catalog()
            .expect("catalog")
            .into_iter()
            .map(|c| c.code)
            .collect();
        assert_eq!(codes, ["A10", "A2", "B2"]);
    }

    #[test]
    fn the_migration_seeds_the_catalog_from_the_stored_findings() {
        let db = TestDb::new();
        let up = include_str!("../../../migrations/2026-10-24-010000_portal_hallazgo/up.sql");
        let seed = &up[up
            .find("INSERT INTO portal_hallazgo")
            .expect("seed statement")..];
        let mut conn = db.conn();
        diesel::sql_query(
            "INSERT INTO portal_cargue_detalle (num_factura, orden, codigo, detalle) VALUES \
             ('FEV1', 0, 'RVC012', 'zeta'), ('FEV2', 0, ' RVC012 ', 'alfa'), \
             ('FEV2', 1, 'ABC1', NULL), ('FEV2', 2, '', 'sin codigo'), ('FEV2', 3, '   ', 'blank')",
        )
        .execute(&mut conn)
        .expect("findings");
        diesel::sql_query(seed).execute(&mut conn).expect("seed");
        diesel::sql_query(seed)
            .execute(&mut conn)
            .expect("seed is idempotent");
        let catalog = PortalStore::new(db.pool()).catalog().expect("catalog");
        let seen: Vec<(&str, &str)> = catalog
            .iter()
            .map(|c| (c.code.as_str(), c.detail()))
            .collect();
        assert_eq!(seen, [("ABC1", ""), ("RVC012", "alfa")]);
    }
}
