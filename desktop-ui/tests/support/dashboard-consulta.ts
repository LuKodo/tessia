import type { ConsultaFacets, Kpis, SeguimientoConsulta, TrackingRow } from "../../src/types/api";
import type { RecordedCall } from "./dashboard-routed-api";

export const CONSULTA = "GET /api/seguimiento/consulta";

export const zeroKpis: Kpis = {
  total: 0, no_radicado: 0, radicadas: 0, rechazadas: 0, radicacion_parcial: 0, nota_ajuste: 0,
  listas_para_radicar: 0, nota_credito: 0, en_proceso: 0, sin_autorizacion: 0,
};

export interface FakeConsultaOptions {
  kpis?: Kpis;
  facets?: ConsultaFacets;
}

/**
 * The route GET /api/seguimiento/consulta over `rows` (or over the rows a function picks from the
 * request's params, which stands for the server filtering). It only does what the server does around
 * the rows: pages them (`limit`, a cursor is "c" + the offset of the next row) and leaves total, kpis
 * and facets out with agregados=no. limit=0 answers the aggregates alone.
 */
export function fakeConsulta(rows: TrackingRow[] | ((params: URLSearchParams) => TrackingRow[]), options: FakeConsultaOptions = {}) {
  return (call: RecordedCall): SeguimientoConsulta => {
    const params = new URLSearchParams(call.path.split("?")[1] ?? "");
    const all = typeof rows === "function" ? rows(params) : rows;
    const limit = params.has("limit") ? Number(params.get("limit")) : 300;
    const from = params.get("cursor") ? Number(params.get("cursor")!.slice(1)) : 0;
    const filas = all.slice(from, from + limit);
    const out: SeguimientoConsulta = { filas, next_cursor: limit > 0 && from + filas.length < all.length ? `c${from + filas.length}` : null };
    if (params.get("agregados") !== "no") {
      out.total = all.length;
      out.kpis = options.kpis ?? { ...zeroKpis, total: all.length };
      out.facets = options.facets ?? { estados: [], alertas: {} };
    }
    return out;
  };
}

/** The consulta requests recorded so far, as their query params. */
export function consultaCalls(calls: RecordedCall[]): URLSearchParams[] {
  return calls.filter((c) => c.method === "GET" && c.path.startsWith("/api/seguimiento/consulta")).map((c) => new URLSearchParams(c.path.split("?")[1] ?? ""));
}

/** Only the requests for rows (they ask for agregados=no) ... */
export const pageCalls = (calls: RecordedCall[]) => consultaCalls(calls).filter((p) => p.get("agregados") === "no");
/** ... and only the requests for the aggregates (limit=0). */
export const aggregateCalls = (calls: RecordedCall[]) => consultaCalls(calls).filter((p) => p.get("limit") === "0");
