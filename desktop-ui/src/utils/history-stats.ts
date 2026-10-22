// Pure logic for the per-run invoice stats shown in the process history and the process
// console: the components render whatever statChips returns.
//
// The server (serve_jobs.go, jobStats) only sends "estadisticas" for jobs that ran the
// proceso pipeline, so older entries and "convenio descargar" jobs have none and show no chips.

const KEYS = [
  "limite",
  "evaluadas",
  "esperadas",
  "procesadas",
  "reemplazadas",
  "gestionadas",
  "terminadas",
  "con_errores",
  "excluidas",
  "revision_manual",
] as const;

/** Color names (badge). */
export type StatChipColor = "neutral" | "info" | "primary" | "success" | "warning" | "error";

export interface StatChip {
  key: string;
  label: string;
  value: number;
  color: StatChipColor;
  hint: string;
}

function count(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.trunc(n) : 0;
}

/**
 * Returns the ordered chips for an entry's estadisticas, or [] when it has none. Límite,
 * Evaluadas, Excluidas and Revisión manual are optional: they only appear above zero (older
 * entries have no Límite/Evaluadas, and a run without --limit has no Límite). Procesadas
 * (above Esperadas) and Reemplazadas (above zero) only appear when invoices that stopped
 * before radicación were replaced. Con errores is neutral at zero so a clean run does not look
 * like a failure. The input is `unknown` on purpose: old entries and junk values are tolerated.
 */
export function statChips(estadisticas: unknown): StatChip[] {
  if (!estadisticas || typeof estadisticas !== "object") return [];
  const stats = estadisticas as Record<string, unknown>;
  if (!KEYS.some((k) => k in stats)) return [];
  const v = (k: (typeof KEYS)[number]) => count(stats[k]);

  const chips: StatChip[] = [];
  if (v("limite") > 0) {
    chips.push({
      key: "limite",
      label: "Límite",
      value: v("limite"),
      color: "neutral",
      hint: "Facturas pendientes que se pidió preparar (--limit)",
    });
  }
  if (v("evaluadas") > 0) {
    chips.push({
      key: "evaluadas",
      label: "Evaluadas",
      value: v("evaluadas"),
      color: "neutral",
      hint: "Facturas pendientes que se revisaron contra las reglas de convenio hasta completar el límite",
    });
  }
  chips.push({
    key: "esperadas",
    label: "Esperadas",
    value: v("esperadas"),
    color: "neutral",
    hint: "Facturas que entraron al proceso.",
  });
  if (v("procesadas") > v("esperadas")) {
    chips.push({
      key: "procesadas",
      label: "Procesadas",
      value: v("procesadas"),
      color: "neutral",
      hint: "Facturas que entraron al proceso, incluyendo las tomadas para reemplazar a las que se detuvieron",
    });
  }
  if (v("reemplazadas") > 0) {
    chips.push({
      key: "reemplazadas",
      label: "Reemplazadas",
      value: v("reemplazadas"),
      color: "info",
      hint: "Facturas adicionales que se tomaron porque otras se detuvieron antes de la radicación",
    });
  }
  chips.push(
    {
      key: "gestionadas",
      label: "Gestionadas",
      value: v("gestionadas"),
      color: "primary",
      hint: "Facturas con resultado final: terminadas, con errores, excluidas o en revisión manual. Puede ser menor que las esperadas si el proceso se canceló o se detuvo por un error.",
    },
    {
      key: "terminadas",
      label: "Terminadas",
      value: v("terminadas"),
      color: "success",
      hint: "Facturas que superaron todas las fases del proceso y, si la radicación estaba activa, se radicaron en un lote exitoso.",
    },
    {
      key: "con_errores",
      label: "Con errores",
      value: v("con_errores"),
      color: v("con_errores") > 0 ? "error" : "neutral",
      hint: "Facturas cuya fase falló y se descartaron. Un lote de radicación fallido cuenta como un solo error.",
    },
  );
  if (v("excluidas") > 0) {
    chips.push({
      key: "excluidas",
      label: "Excluidas",
      value: v("excluidas"),
      color: "neutral",
      hint: "Facturas descartadas por una regla de negocio, tanto en la preparación del convenio (monto, subplan, nota técnica, MIPRES…) como en fases posteriores; no se procesan.",
    });
  }
  if (v("revision_manual") > 0) {
    chips.push({
      key: "revision_manual",
      label: "Revisión manual",
      value: v("revision_manual"),
      color: "warning",
      hint: "Facturas que salieron del proceso y requieren revisión de una persona, tanto en la preparación del convenio como en fases posteriores. Revísalas en el informe de revisión manual del Dashboard.",
    });
  }
  return chips;
}
