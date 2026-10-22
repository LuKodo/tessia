import type { Kpis, PipelineCategory, TrackingAlert, TrackingDot, TrackingRow } from "../types/api";
import type { RipsIconKey } from "./rips-icons";

export type KpiKey = keyof Kpis;

/** Hues of the KPI cards: Tailwind palette names (`neutral` is the themed neutral, olive). */
export type KpiColor = "cyan" | "neutral" | "green" | "red" | "blue" | "pink" | "orange" | "purple" | "violet";

/** [key, label, sub, icon key (see rips-icons.ts), hue] */
export type KpiDefinition = readonly [key: KpiKey, label: string, sub: string, icon: RipsIconKey, color: KpiColor];

export const KPIS: readonly KpiDefinition[] = [
  ["total", "Total facturas", "", "list", "cyan"],
  ["no_radicado", "No radicado", "nada generado", "circleMinus", "neutral"],
  ["radicadas", "Radicadas", "", "circleCheck", "green"],
  ["rechazadas", "Rechazadas", "", "circleX", "red"],
  ["radicacion_parcial", "Radicación parcial", "", "hourglass", "blue"],
  ["nota_ajuste", "Requieren nota ajuste", "", "fileAlert", "pink"],
  ["listas_para_radicar", "Listas para radicar", "4/4 completas", "send", "orange"],
  ["nota_credito", "Requieren nota crédito", "", "receipt", "purple"],
  ["en_proceso", "En proceso", "generación parcial", "hourglass", "violet"],
  ["sin_autorizacion", "Sin autorización", "riesgo de glosa", "shieldX", "red"],
];

/**
 * Classes of each KPI hue. They are literal strings because Tailwind only emits the classes it finds
 * written out in the source: `card` tints the card, `icon` colors the glyph, `bar` the share bar and
 * `ring` the outline of the active card (through `currentColor`).
 */
export const KPI_TONE: Record<KpiColor, { card: string; icon: string; bar: string; ring: string }> = {
  cyan: { card: "bg-cyan-50 dark:bg-cyan-950/40", icon: "text-cyan-600 dark:text-cyan-400", bar: "bg-cyan-500", ring: "text-cyan-500" },
  neutral: { card: "bg-neutral-100 dark:bg-neutral-800/60", icon: "text-neutral-600 dark:text-neutral-400", bar: "bg-neutral-500", ring: "text-neutral-500" },
  green: { card: "bg-green-50 dark:bg-green-950/40", icon: "text-green-600 dark:text-green-400", bar: "bg-green-500", ring: "text-green-500" },
  red: { card: "bg-red-50 dark:bg-red-950/40", icon: "text-red-600 dark:text-red-400", bar: "bg-red-500", ring: "text-red-500" },
  blue: { card: "bg-blue-50 dark:bg-blue-950/40", icon: "text-blue-600 dark:text-blue-400", bar: "bg-blue-500", ring: "text-blue-500" },
  pink: { card: "bg-pink-50 dark:bg-pink-950/40", icon: "text-pink-600 dark:text-pink-400", bar: "bg-pink-500", ring: "text-pink-500" },
  orange: { card: "bg-orange-50 dark:bg-orange-950/40", icon: "text-orange-600 dark:text-orange-400", bar: "bg-orange-500", ring: "text-orange-500" },
  purple: { card: "bg-purple-50 dark:bg-purple-950/40", icon: "text-purple-600 dark:text-purple-400", bar: "bg-purple-500", ring: "text-purple-500" },
  violet: { card: "bg-violet-50 dark:bg-violet-950/40", icon: "text-violet-600 dark:text-violet-400", bar: "bg-violet-500", ring: "text-violet-500" },
};

export type KpiFilterTarget =
  | { type: "pipeline"; value: PipelineCategory }
  | { type: "alert"; value: TrackingAlert };

/**
 * What a clickable KPI filters: a Pipeline value, or an alert code (same codes the
 * "Alertas y pendientes" checkboxes use). "total" has no entry: it stays non-clickable.
 */
export const KPI_FILTER: Partial<Record<KpiKey, KpiFilterTarget>> = {
  no_radicado: { type: "pipeline", value: "no_radicado" },
  radicadas: { type: "pipeline", value: "radicada" },
  rechazadas: { type: "pipeline", value: "rechazada" },
  radicacion_parcial: { type: "pipeline", value: "radicacion_parcial" },
  listas_para_radicar: { type: "pipeline", value: "lista_para_radicar" },
  en_proceso: { type: "pipeline", value: "en_proceso" },
  nota_ajuste: { type: "alert", value: "nota_ajuste" },
  nota_credito: { type: "alert", value: "nota_credito" },
  sin_autorizacion: { type: "alert", value: "sin_autorizacion" },
};

export const PIPELINES: ReadonlyArray<readonly [PipelineCategory, string]> = [
  ["no_radicado", "No radicado"],
  ["en_proceso", "En proceso"],
  ["lista_para_radicar", "Lista para radicar"],
  ["radicacion_parcial", "Radicación parcial"],
  ["radicada", "Radicada"],
  ["rechazada", "Rechazada"],
  ["no_aplica", "No aplica (excluida / revisión)"],
];

/** [code, label, danger?] */
export type AlertDefinition = readonly [code: TrackingAlert, label: string, danger?: boolean];

export const ALERTS: readonly AlertDefinition[] = [
  ["falta_zip", "Falta ZIP XML/PDF"],
  ["falta_json", "Falta JSON"],
  ["falta_cuv", "Falta TXT/CUV"],
  ["falta_crc", "Falta ZIP CRC"],
  ["pendiente_radicar", "Pendiente por radicar"],
  ["nota_ajuste", "Requiere nota ajuste"],
  ["nota_credito", "Requiere nota crédito"],
  ["requiere_autorizacion", "Requiere autorización"],
  ["vencida_22_dias", "Más de 22 días hábiles", true],
  ["sin_autorizacion", "Sin autorización", true],
];

export type TextFilterKey = "factura" | "cargue" | "ssc" | "autorizacion" | "mipres" | "subplan";

export type TextFilterDefinition = readonly [
  key: TextFilterKey,
  label: string,
  placeholder: string,
  icon: RipsIconKey,
];

export const TEXT_FILTERS: readonly TextFilterDefinition[] = [
  ["factura", "Número de factura", "Ej. FVS1428252", "fileText"],
  ["cargue", "Lote / Cargue", "Ej. Convenio_...", "stack"],
  ["ssc", "SSC", "Ej. 5618214", "hash"],
  ["autorizacion", "Autorización", "", "shieldCheck"],
  ["mipres", "ID MIPRES", "", "id"],
  ["subplan", "Subplan", "", "tag"],
];

/** Artifact column headers, in the order of `TrackingRow.artefactos`. */
export const ARTIFACTS: ReadonlyArray<readonly [key: string, label: string]> = [
  ["zip", "ZIP XML/PDF"],
  ["json", "JSON RIPS"],
  ["cuv", "TXT CUV"],
  ["crc", "ZIP CRC"],
];

export const PIPELINE_LABEL: Record<string, string> = {
  ...Object.fromEntries(PIPELINES),
  no_aplica: "No aplica",
};

/** Color names of the artifact / radicación dots. */
export type DotColor = "success" | "error" | "warning" | "neutral";

export const DOT_COLOR: Record<TrackingDot, DotColor> = { ok: "success", error: "error", en_curso: "warning", pendiente: "neutral" };

/** Background class of each dot color, spelled out because Tailwind only emits literal class strings. */
export const DOT_TONE: Record<DotColor, string> = {
  success: "bg-success",
  error: "bg-error",
  warning: "bg-warning",
  neutral: "bg-neutral-400",
};

/** Background class of a dot or step by status (pending tone for an unknown one). */
export function dotTone(status: string): string {
  return DOT_TONE[(DOT_COLOR as Record<string, DotColor>)[status] ?? "neutral"];
}

export const DOT_LABEL: Record<TrackingDot, string> = {
  ok: "correcto",
  error: "falta / error",
  pendiente: "pendiente",
  en_curso: "en curso",
};

/** Display estado of a row (upper case): the log estado, "SIN INICIAR" for a fresh prepared row, else the outcome. */
export function estadoOf(r: Pick<TrackingRow, "estado" | "resultado">): string {
  return (r.estado || (r.resultado === "preparada" ? "SIN INICIAR" : r.resultado || "")).toUpperCase();
}

/** Color of the estado badge. */
export function estadoClass(e: string | null | undefined): "success" | "error" | "warning" | "primary" {
  const v = (e || "").toUpperCase();
  if (v === "TERMINADO") return "success";
  if (["RECHAZADO", "VENCIDO", "NOTA_AJUSTE", "EXCLUIDA"].includes(v)) return "error";
  if (v === "REVISION_MANUAL") return "warning";
  return "primary";
}
