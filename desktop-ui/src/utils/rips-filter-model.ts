import type { ConsultaParams, ConsultaSort } from "../types/api";
import { KPI_FILTER, TEXT_FILTERS } from "./rips-constants";
import type { KpiKey, TextFilterKey } from "./rips-constants";
import type { SortState } from "./table-model";

// Pure filter logic behind useRipsFilters: the "draft" form state, the "applied" snapshot
// derived from it, and the query the server is asked with. The server does the row matching.

/** What is typed/selected/checked right now (not yet applied to the table). */
export interface FilterDraft {
  text: Record<TextFilterKey, string>;
  estado: string;
  pipeline: string;
  desde: string;
  hasta: string;
  alerts: Set<string>;
}

/** The committed snapshot the server is asked to filter with. */
export interface AppliedFilters {
  text: Partial<Record<TextFilterKey, string>>;
  estado: string;
  pipeline: string;
  desde: string;
  hasta: string;
  alerts: string[];
}

export function emptyDraft(): FilterDraft {
  return {
    text: Object.fromEntries(TEXT_FILTERS.map(([k]) => [k, ""])) as Record<TextFilterKey, string>,
    estado: "",
    pipeline: "",
    desde: "",
    hasta: "",
    alerts: new Set(),
  };
}

export function computeApplied(d: FilterDraft): AppliedFilters {
  const text: Partial<Record<TextFilterKey, string>> = {};
  for (const [k, v] of Object.entries(d.text) as Array<[TextFilterKey, string]>) {
    const t = v.trim().toLowerCase();
    if (t) text[k] = t;
  }
  return { text, estado: d.estado, pipeline: d.pipeline, desde: d.desde, hasta: d.hasta, alerts: [...d.alerts] };
}

export function hasCargueFilter(applied: AppliedFilters): boolean {
  return Boolean(applied.text.cargue);
}

export function countActive(applied: AppliedFilters): number {
  return (
    Object.keys(applied.text).length +
    (applied.estado ? 1 : 0) +
    (applied.pipeline ? 1 : 0) +
    (applied.desde ? 1 : 0) +
    (applied.hasta ? 1 : 0) +
    applied.alerts.length
  );
}

/**
 * The filter and sort part of a GET /api/seguimiento/consulta query: the applied filters and the
 * table sort, with only what is set (the server applies the "no_aplica stays hidden" rule itself).
 */
export function toConsultaParams(applied: AppliedFilters, sort: SortState): ConsultaParams {
  const params: ConsultaParams = { ...applied.text };
  if (applied.estado) params.estado = applied.estado;
  if (applied.pipeline) params.pipeline = applied.pipeline;
  if (applied.desde) params.desde = applied.desde;
  if (applied.hasta) params.hasta = applied.hasta;
  if (applied.alerts.length) params.alerta = [...applied.alerts];
  if (sort.key) {
    params.sort = sort.key as ConsultaSort;
    params.dir = sort.dir === 1 ? "asc" : "desc";
  }
  return params;
}

export function togglePipelineDraft(draft: FilterDraft, value: string): FilterDraft {
  return { ...draft, pipeline: draft.pipeline === value ? "" : value };
}

export function toggleAlertDraft(draft: FilterDraft, value: string): FilterDraft {
  const alerts = new Set(draft.alerts);
  if (alerts.has(value)) alerts.delete(value);
  else alerts.add(value);
  return { ...draft, alerts };
}

export function isKPIActive(draft: FilterDraft, key: KpiKey | string): boolean {
  const f = KPI_FILTER[key as KpiKey];
  if (!f) return false;
  return f.type === "pipeline" ? draft.pipeline === f.value : draft.alerts.has(f.value);
}

/**
 * A text filter (e.g. the "Lote / Cargue" box) set from elsewhere, such as a chart bar:
 * the same value again clears it (case/space-insensitive).
 */
export function isTextActive(draft: FilterDraft, key: TextFilterKey, value: string | null | undefined): boolean {
  const want = String(value || "").trim().toLowerCase();
  return want !== "" && String(draft.text[key] || "").trim().toLowerCase() === want;
}

export function toggleTextDraft(draft: FilterDraft, key: TextFilterKey, value: string): FilterDraft {
  const next = isTextActive(draft, key, value) ? "" : value;
  return { ...draft, text: { ...draft.text, [key]: next } };
}
