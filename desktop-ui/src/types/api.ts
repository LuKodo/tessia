// Types mirroring the JSON contract of `tessia serve` (tessia/cmd/tessia/cmd_serve*.go
// and tessia/internal/dashboard). Derived from the json tags of the Go structs and the
// golden files in tessia/cmd/tessia/testdata/golden/dashboard.
//
// Contract quirks the client has to live with (see README, "API contract notes"):
//  - `writeJSON` answers every handler error as 500 `{ "error": "..." }`; other writers
//    use real status codes (400, 404, 405, 409, 413, 503) with the same body shape.
//  - Some handlers reject with a plain-text body (`http.Error`), so an error body is not
//    always JSON.
//  - `GET /api/facturas/{factura}/portal` can answer 200 with a readable `error` next to
//    usable data (see `PortalView`); use `requestJSON`, not `getJSON`.
//  - Go `time.Time` values arrive as RFC 3339 strings; a zero `time.Time` with
//    `omitempty` is NOT omitted (it serializes as "0001-01-01T00:00:00Z").
//  - Go nil slices/maps with `omitempty` are absent, without it they arrive as `null`.

/** Shape of any error body: `{ "error": "..." }`. */
export interface ApiErrorBody {
  error: string;
}

// --- Cargues ---------------------------------------------------------------

/** Rolled-up cargue status; `SIN_PREPARAR` is a CSV-only load, `SIN_DATOS` means no database. */
export type CargueStatus =
  | "COMPLETADO"
  | "CON_INCIDENCIAS"
  | "PROCESANDO"
  | "SIN_FACTURAS"
  | "SIN_DATOS"
  | "SIN_PREPARAR";

/** GET /api/cargues (latest 20, most recently modified first). */
export interface Cargue {
  id: string;
  nombre: string;
  fecha_inicio?: string;
  fecha_fin?: string;
  facturas: number;
  excluidas: number;
  revision_manual: number;
  modificado_en: string;
  estado: CargueStatus;
}

/** Outcome of `convenio preparar` for one invoice. */
export type PrepareOutcome = "preparada" | "excluida" | "revision_manual";

// --- Seguimiento / resumen ---------------------------------------------------

/** Dot state of an artifact or filing step. */
export type TrackingDot = "ok" | "error" | "pendiente" | "en_curso";

export type PipelineCategory =
  | "no_aplica"
  | "no_radicado"
  | "en_proceso"
  | "lista_para_radicar"
  | "radicacion_parcial"
  | "radicada"
  | "rechazada";

export type TrackingAlert =
  | "falta_zip"
  | "falta_json"
  | "falta_cuv"
  | "falta_crc"
  | "pendiente_radicar"
  | "nota_ajuste"
  | "nota_credito"
  | "requiere_autorizacion"
  | "vencida_22_dias"
  | "sin_autorizacion";

export interface AgreementLine {
  ssc: string;
  cum: string;
  valor_unitario: string;
  valor_total: string;
  autorizacion: string;
  id_mipres: string;
}

/** Four fixed slots: ZIP XML/PDF, RIPS JSON, TXT CUV, ZIP CRC. */
export type ArtifactDots = [TrackingDot, TrackingDot, TrackingDot, TrackingDot];
/** Mutual Ser filing steps 1-4. */
export type FilingDots = [TrackingDot, TrackingDot, TrackingDot, TrackingDot];

/** One row of the "Seguimiento de RIPS" table. */
export interface TrackingRow {
  factura: string;
  resultado: PrepareOutcome | string;
  cargue: string;
  cargue_nombre?: string;
  artefactos: ArtifactDots;
  radicacion: FilingDots;
  estado?: string;
  /**
   * Free text behind `estado`. The list routes cut it to 90 characters plus "…" (the payload is
   * slim); the whole text is in the invoice detail (`InvoiceTracking`, GET /api/facturas/{factura}/seguimiento).
   */
  estado_detalle?: string;
  /** YYYY-MM-DD */
  fecha_factura?: string;
  ssc?: string;
  subplan?: string;
  valor_xml?: string;
  valor_convenio?: string;
  autorizacion?: string;
  id_mipres?: string;
  actualizado_en?: string;
  requiere_nota_ajuste?: boolean;
  requiere_nota_credito?: boolean;
  vencida_rffe022?: boolean;
  pipeline: PipelineCategory;
  alertas?: TrackingAlert[];
  fecha_inicio_cargue?: string;
  fecha_fin_cargue?: string;
  portal_consultado: boolean;
  portal_estado?: string;
  portal_advertencias?: number;
  portal_errores?: number;
  portal_final?: boolean;
}

/** GET /api/facturas/{factura}/seguimiento: the table row plus the detail the list rows leave out. */
export interface InvoiceTracking extends TrackingRow {
  /** Whole text, not cut. */
  estado_detalle?: string;
  lineas_convenio?: AgreementLine[];
}

/** Sort keys of GET /api/seguimiento/consulta (no key: cargue, then invoice, ascending). */
export type ConsultaSort = "fecha_factura" | "actualizado_en" | "factura";

/** Query of GET /api/seguimiento/consulta; every field is optional and omitted when empty. */
export interface ConsultaParams {
  /** Case-insensitive substring filters. `cargue` matches the cargue name and id. */
  factura?: string;
  cargue?: string;
  ssc?: string;
  autorizacion?: string;
  mipres?: string;
  subplan?: string;
  /** Exact. */
  estado?: string;
  pipeline?: string;
  /** The rows that raise at least one of them; sent as one comma list. */
  alerta?: string[];
  /** Invoice date range, YYYY-MM-DD; rows without a date drop out when either is set. */
  desde?: string;
  hasta?: string;
  sort?: ConsultaSort;
  dir?: "asc" | "desc";
  /** Page size, 0..1000 (server default 300); 0 asks for the aggregates alone. */
  limit?: number;
  /** Opaque; only valid with the sort and direction it was issued for. */
  cursor?: string;
  /** false: leave total, kpis and facets out of the answer. */
  agregados?: boolean;
  /** Date bound of the KPI cards (server default: the current month; lifted while `cargue` is set). */
  kpi_desde?: string;
  kpi_hasta?: string;
}

/** The filter options the server derives from the whole table (no filter applied). */
export interface ConsultaFacets {
  estados: string[];
  /** Rows per alert code. */
  alertas: Record<string, number>;
}

/** GET /api/seguimiento/consulta. `total`, `kpis` and `facets` are absent with `agregados=no`. */
export interface SeguimientoConsulta {
  total?: number;
  filas: TrackingRow[];
  facets?: ConsultaFacets;
  kpis?: Kpis;
  /** Cursor of the next page; null on the last one. */
  next_cursor: string | null;
}

export interface Kpis {
  total: number;
  no_radicado: number;
  radicadas: number;
  rechazadas: number;
  radicacion_parcial: number;
  nota_ajuste: number;
  listas_para_radicar: number;
  nota_credito: number;
  en_proceso: number;
  sin_autorizacion: number;
}

export interface CarguePoint {
  cargue: string;
  nombre?: string;
  fecha_inicio?: string;
  total: number;
  gestionadas: number;
  excluidas: number;
  revision_manual: number;
  radicadas: number;
  /** Plain decimal string. */
  valor_cargado: string;
  /** Plain decimal string. */
  valor_radicado: string;
}

/** GET /api/resumen?desde=YYYY-MM-DD&hasta=YYYY-MM-DD (default: last 30 days; max 366 days, else 400). */
export interface Summary {
  desde: string;
  hasta: string;
  kpis: Kpis;
  cargues: CarguePoint[];
}

export interface SummaryQuery {
  desde?: string;
  hasta?: string;
}

// --- Informe de revisión manual ---------------------------------------------

export type ReportCategory = "revision_manual" | "falla" | "excluida";

export interface ReportItem {
  factura: string;
  fase: string;
  categoria: ReportCategory;
  /** Leading code of a "CODE: text" reason, else "". */
  codigo: string;
  motivo: string;
  /** RFC 3339 UTC; "" for entries only known from convenio.json. */
  actualizado_en: string;
}

/** GET /api/cargues/{id}/informe (`?formato=csv` downloads a CSV instead). */
export interface ManualReviewReport {
  cargue: string;
  generado_en: string;
  /** True when persisted states could not be read (no database). */
  incompleto: boolean;
  conteos: { revision_manual: number; excluidas: number; fallas: number };
  items: ReportItem[];
}

// --- Reportes -----------------------------------------------------------------

/** One invoice with its four filing steps confirmed (GET /api/reportes/radicadas). */
export interface RadicadaRow {
  cargue: string;
  cargue_nombre: string;
  factura: string;
  /** RFC 3339 UTC: when step 4 was confirmed (radicacion_estado.updated_at); "" when not recorded. */
  fecha_radicacion: string;
  estado: string;
  fecha_factura: string;
  ssc: string;
  autorizacion: string;
  /** Convenio total as a plain decimal. */
  valor: string;
  /** Notes recorded for the invoice, oldest first, joined with " | ". */
  observaciones: string;
}

/** GET /api/reportes/radicadas (`?formato=csv` downloads every matching row as a CSV instead). */
export interface RadicadasReport {
  generado_en: string;
  /** Invoices matching the filters, before `limite`. */
  total: number;
  /** True when `filas` holds fewer rows than `total`. */
  truncado: boolean;
  filas: RadicadaRow[];
}

/** Query of the filed invoices report; empty values are left out of the URL. */
export interface RadicadasQuery {
  /** Cargues to include (repeated `cargue`); none means every cargue. */
  cargue?: string[];
  /** First and last filing day, YYYY-MM-DD, inclusive. */
  desde?: string;
  hasta?: string;
  /** Part of the invoice number. */
  q?: string;
  /** Rows of the JSON preview (default 500, at most 5000); the CSV ignores it. */
  limite?: number;
}

// --- Health ------------------------------------------------------------------

/** One row of GET /api/health (TCP reachability only, not authenticated health). */
export interface ServiceHealth {
  nombre: string;
  disponible: boolean;
  detalle?: string;
  latencia_ms: number;
}

/** GET /api/health */
export type HealthResponse = ServiceHealth[];

// --- Nota técnica ------------------------------------------------------------

/**
 * GET /api/nota-tecnica: read-only status of the Nota Técnica published in Redis. Always answers
 * 200; `{ "exists": false }` is the whole answer when Redis is unreachable or has no `nt:actual`.
 */
export interface NotaTecnicaStatus {
  exists: boolean;
  /** First 12 hex characters of `version`. */
  file_name?: string;
  /** Current version: 64 hex characters (SHA-256). */
  version?: string;
  /** Number of rows of the catalog (`meta.filas`); the API forwards it as a string. */
  filas?: string | number;
}

// --- Parameters --------------------------------------------------------------

/** Request body of PUT /api/parameters (unknown fields are rejected with 400). */
export interface PreparationParameters {
  max_invoice_amount: string;
  automatic_subplans: string[];
  wave_size: number;
  derechos_unavailable_percent: number;
  derechos_min_sample: number;
  cuv_token_failure_percent: number;
  cuv_min_sample: number;
}

/** GET/PUT /api/parameters response; `configured` is false while the defaults are shown. 503 without a database. */
export interface PreparationParametersResponse extends PreparationParameters {
  configured: boolean;
}

// --- Hallazgos ---------------------------------------------------------------

export interface Hallazgo {
  codigo: string;
  detalle: string;
  genera_glosa: boolean;
  /** The payer returns the invoice because of this finding. */
  devolucion: boolean;
  descripcion: string;
}

/** GET /api/hallazgos (503 without a database). */
export interface HallazgosResponse {
  hallazgos: Hallazgo[];
  total: number;
}

/**
 * PUT /api/hallazgos/{codigo} body. `genera_glosa` is required; `devolucion` is optional
 * (omitted or null keeps the stored value); `descripcion` max 2000 characters.
 */
export interface HallazgoUpdate {
  genera_glosa: boolean;
  devolucion?: boolean | null;
  descripcion: string;
}

// --- Portal Mutual Ser -------------------------------------------------------

/**
 * `documento_soporte` of a portal result: the support document object as Mutual Ser sent it
 * (`core/filtersAll`), stored verbatim. Its scalars are free-form (the Go client decodes them
 * with `flexString`, so a number or bool can arrive where text is expected); only `id`,
 * `nombre_archivo` and `estado` are known and read, any other key passes through.
 */
export interface PortalSupportDocument {
  id?: string | number;
  nombre_archivo?: string | number;
  estado?: string | number;
  [key: string]: unknown;
}

export interface PortalFinding {
  codigo: string;
  tipo: string;
  campo: string;
  detalle: string;
}

/**
 * GET /api/facturas/{factura}/portal and POST .../portal/actualizar.
 * Quirk: a portal failure is a 200 carrying the stored data plus a non-empty `error`
 * (and possibly `mensaje`); only a store failure is a 500. Every string field is always
 * present ("" when unknown). `documento_soporte` is a JSON object or null.
 */
export interface PortalView {
  factura: string;
  consultado: boolean;
  final: boolean;
  estado: string;
  remision: string;
  fecha_cargue: string;
  fecha_cargue_texto: string;
  usuario: string;
  servicios: string;
  modalidad: string;
  nombre_archivo: string;
  id_archivo: string;
  id_cargue: string;
  id_evento: string;
  tipo_evento: string;
  tipo_error: string;
  codigo_error: string;
  devuelto: boolean;
  fecha_expedicion: string;
  receipt_name: string;
  /** Null when the invoice has no support document (empty, or not a JSON object). */
  documento_soporte: PortalSupportDocument | null;
  advertencias: number;
  errores: number;
  hallazgos: PortalFinding[];
  consultas: number;
  consultado_en: string;
  origen: string;
  mensaje: string;
  error: string;
}

// --- Jobs --------------------------------------------------------------------

export interface JobStats {
  esperadas: number;
  procesadas: number;
  reemplazadas: number;
  gestionadas: number;
  terminadas: number;
  con_errores: number;
  excluidas: number;
  revision_manual: number;
  evaluadas: number;
  limite: number;
}

/** Live tally of one pipeline phase while a job runs. */
export interface JobPhaseProgress {
  fase: string;
  etiqueta: string;
  en_curso: number;
  terminadas: number;
  revision_manual: number;
  excluidas: number;
  con_errores: number;
  /** Busy wall time in ms; absent when the phase never ran. */
  tiempo_ms?: number;
}

export type JobConvenioState = "preparando" | "ok" | "error";

/** Step 0 (convenio preparar) state of a job. */
export interface JobConvenio {
  estado: JobConvenioState;
  evaluadas: number;
  total: number;
  preparadas: number;
  excluidas: number;
  revision_manual: number;
  mensaje?: string;
}

/**
 * GET /api/jobs/current, and the answer of POST /api/run, /api/download and
 * /api/cargues/{id}/reintentar. With no job yet in this process the server answers
 * `{ "id": "", "tipo": "", ..., "instancia_servidor": "<id>" }`: all fields at their zero
 * values (`iniciado_en` is "0001-01-01T00:00:00Z") except `instancia_servidor`.
 */
export interface JobSnapshot {
  id: string;
  tipo: string;
  corriendo: boolean;
  cancelable: boolean;
  iniciado_en: string;
  /** Not omitted while running: a Go zero time arrives as "0001-01-01T00:00:00Z". */
  terminado_en?: string;
  error?: string;
  salida: string;
  estadisticas?: JobStats;
  fases?: JobPhaseProgress[];
  radicadas?: number;
  convenio?: JobConvenio;
  /** Identifies the `tessia serve` process: a change means the server restarted. */
  instancia_servidor?: string;
}

/** One finished job of GET /api/jobs/history (most recently finished first). */
export interface JobHistoryEntry {
  id: string;
  tipo: string;
  iniciado_en: string;
  terminado_en: string;
  error?: string;
  /** Short output tail (about 2 KiB). */
  salida_final?: string;
  estadisticas?: JobStats;
}

/** GET /api/jobs/history. Quirk: a Go nil slice, so it is `null` (not `[]`) before any job finished. */
export type JobsHistory = JobHistoryEntry[] | null;

/** POST /api/jobs/cancel */
export interface CancelResult {
  ok: true;
}

/**
 * POST /api/run body. Quirk: the Go struct has no json tags, so the keys are capitalized.
 * `From`/`To` are phase names ("" means from the start / to the end); `Limit` 0 = no limit.
 */
export interface RunRequest {
  Cargue: string;
  From: string;
  To: string;
  Limit: number;
  /** Prepare the convenio again (current cap) even when the cargue is already prepared. Sent only when true. */
  Reprepare?: boolean;
}

/** POST /api/download body. Quirk: capitalized keys, dates as YYYY-MM-DD. 409 when the folder exists. */
export interface DownloadRequest {
  Desde: string;
  Hasta: string;
}

/** POST /api/cargues/{id}/reintentar body (lowercase key, unknown fields rejected). 409 unless the invoice has an eligible VALOR_DIFERENTE manual review. */
export interface RetryRequest {
  factura: string;
}

// --- Convenio upload ---------------------------------------------------------

/** Form fields of POST /api/convenio/upload (multipart: csv). */
export interface ConvenioUploadForm {
  csv: Blob;
}

export interface ConvenioUploadResult {
  ok: true;
  cargue: string;
  file_name: string;
}
