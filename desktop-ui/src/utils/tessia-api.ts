import type {
  CancelResult,
  Cargue,
  ConsultaParams,
  ConvenioUploadForm,
  ConvenioUploadResult,
  DownloadRequest,
  HallazgoUpdate,
  Hallazgo,
  HallazgosResponse,
  HealthResponse,
  InvoiceTracking,
  JobSnapshot,
  JobsHistory,
  ManualReviewReport,
  NotaTecnicaStatus,
  PortalView,
  PreparationParameters,
  PreparationParametersResponse,
  RadicadasQuery,
  RadicadasReport,
  RunRequest,
  SeguimientoConsulta,
  Summary,
  SummaryQuery,
} from "../types/api";
import type { ApiClient } from "./api-client";

const enc = encodeURIComponent;

/**
 * Path of the filed invoices report. The client's `Query` cannot repeat a key, and `cargue` repeats,
 * so the query string is built here and travels inside the path.
 */
function radicadasPath(q: RadicadasQuery, csv: boolean): string {
  const params = new URLSearchParams();
  for (const id of q.cargue ?? []) if (id) params.append("cargue", id);
  if (q.desde) params.set("desde", q.desde);
  if (q.hasta) params.set("hasta", q.hasta);
  if (q.q) params.set("q", q.q);
  if (q.limite && !csv) params.set("limite", String(q.limite));
  if (csv) params.set("formato", "csv");
  const qs = params.toString();
  return `/api/reportes/radicadas${qs ? `?${qs}` : ""}`;
}

/** One typed method per route of `tessia serve`. Path parameters are URL-encoded here. */
export function createTessiaApi(client: ApiClient) {
  return {
    health: () => client.getJSON<HealthResponse>("/api/health"),

    cargues: () => client.getJSON<Cargue[]>("/api/cargues"),
    informe: (id: string) => client.getJSON<ManualReviewReport>(`/api/cargues/${enc(id)}/informe`),
    /** Plain link for the CSV variant of the informe (not fetched through JSON). */
    informeCsvUrl: (id: string) => client.url(`/api/cargues/${enc(id)}/informe`, { formato: "csv" }),
    /** Filed (radicadas) invoices with their filing date and notes, filtered. */
    radicadas: (q: RadicadasQuery = {}) => client.getJSON<RadicadasReport>(radicadasPath(q, false)),
    /** Plain link for the full CSV of the same filters (no limit; not fetched through JSON). */
    radicadasCsvUrl: (q: RadicadasQuery = {}) => client.url(radicadasPath(q, true)),
    /** Full row of one invoice (convenio lines and state detail included); the list rows omit them. */
    seguimientoFactura: (factura: string, cargue?: string) =>
      client.getJSON<InvoiceTracking>(`/api/facturas/${enc(factura)}/seguimiento`, undefined, { cargue }),
    /**
     * One page and/or the aggregates of the "Seguimiento" table, filtered and sorted by the server.
     * The alerts travel as one comma list, so the client's single-valued `Query` is enough.
     */
    seguimientoConsulta: ({ alerta, agregados, ...rest }: ConsultaParams = {}) =>
      client.getJSON<SeguimientoConsulta>("/api/seguimiento/consulta", undefined, {
        ...rest,
        alerta: alerta?.join(","),
        agregados: agregados === false ? "no" : undefined,
      }),
    resumen: (q: SummaryQuery = {}) => client.getJSON<Summary>("/api/resumen", undefined, { desde: q.desde, hasta: q.hasta }),

    /** Uses requestJSON: a 200 may carry `error` next to usable data. */
    portal: (factura: string) => client.requestJSON<PortalView>(`/api/facturas/${enc(factura)}/portal`),
    refreshPortal: (factura: string) =>
      client.requestJSON<PortalView>(`/api/facturas/${enc(factura)}/portal/actualizar`, { method: "POST" }),

    hallazgos: () => client.getJSON<HallazgosResponse>("/api/hallazgos"),
    updateHallazgo: (codigo: string, body: HallazgoUpdate) =>
      client.sendJSON<Hallazgo>("PUT", `/api/hallazgos/${enc(codigo)}`, body),

    parameters: () => client.getJSON<PreparationParametersResponse>("/api/parameters"),
    saveParameters: (body: PreparationParameters) =>
      client.sendJSON<PreparationParametersResponse>("PUT", "/api/parameters", body),

    notaTecnica: () => client.getJSON<NotaTecnicaStatus>("/api/nota-tecnica"),

    uploadConvenio: ({ csv }: ConvenioUploadForm) => {
      const form = new FormData();
      form.append("csv", csv);
      return client.upload<ConvenioUploadResult>("/api/convenio/upload", form);
    },
    /** Note the capitalized request keys (Desde/Hasta). */
    download: (body: DownloadRequest) => client.sendJSON<JobSnapshot>("POST", "/api/download", body),
    /** Note the capitalized request keys (Cargue/From/To/Limit). */
    runJob: (body: RunRequest) => client.sendJSON<JobSnapshot>("POST", "/api/run", body),
    /** Lenient: a job snapshot carries `error` when it failed, which is data, not a failed request. */
    retryInvoice: (cargueId: string, factura: string) =>
      client.sendJSON<JobSnapshot>("POST", `/api/cargues/${enc(cargueId)}/reintentar`, { factura }, { lenient: true }),

    /**
     * Uses requestJSON: a finished job that failed or was cancelled has `error` set, and the
     * old UI's strict getJSON threw on it, hiding the error from the console and the alert.
     */
    jobsCurrent: () => client.requestJSON<JobSnapshot>("/api/jobs/current"),
    jobsHistory: () => client.getJSON<JobsHistory>("/api/jobs/history"),
    cancelJob: () => client.sendJSON<CancelResult>("POST", "/api/jobs/cancel"),
  };
}

export type TessiaApi = ReturnType<typeof createTessiaApi>;
