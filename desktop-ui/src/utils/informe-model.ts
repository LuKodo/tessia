// Pure logic for the manual review report: the Revisión manual page (RevisionTabla) renders
// what these helpers return.
//
// The server (GET /api/cargues/{id}/informe) sends
//   { conteos: { revision_manual, excluidas, fallas }, items: [{ factura, fase,
//     categoria: "revision_manual" | "excluida" | "falla", codigo, motivo, ... }] }
import type { ManualReviewReport, ReportCategory } from "../types/api";
import { PHASES } from "./phases";

/** Color names of the category badges. */
export type CategoryColor = "warning" | "neutral" | "error";

const CATEGORY: Record<ReportCategory, { label: string; color: CategoryColor }> = {
  revision_manual: { label: "Revisión manual", color: "warning" },
  excluida: { label: "Excluida", color: "neutral" },
  falla: { label: "Falla", color: "error" },
};

// Report phase names that differ from the pipeline phase keys in PHASES.
const PHASE_ALIASES: Record<string, string> = { fev_ajuste: "fev", fev_valor: "fev", conciliacion: "conciliar" };
const PHASE_LABELS: Record<string, string> = Object.fromEntries(PHASES);

function categoryEntry(categoria: string) {
  return (CATEGORY as Record<string, { label: string; color: CategoryColor } | undefined>)[categoria];
}

/**
 * Keeps the manual reviews by default; `includeOthers` also keeps the excluded invoices and
 * the failures. Order is preserved and the input is not mutated.
 */
export function filterItems<T extends { categoria?: string }>(
  items: readonly (T | null | undefined)[] | null | undefined,
  { includeOthers = false }: { includeOthers?: boolean } = {},
): T[] {
  if (!Array.isArray(items)) return [];
  const present = items.filter((i): i is T => !!i);
  return includeOthers ? present : present.filter((i) => i.categoria === "revision_manual");
}

export function categoryLabel(categoria: string | null | undefined): string {
  if (categoria === undefined || categoria === null) return "";
  return categoryEntry(categoria)?.label ?? String(categoria);
}

export function categoryColor(categoria: string | null | undefined): CategoryColor {
  return (categoria && categoryEntry(categoria)?.color) || "neutral";
}

export function phaseLabel(fase: string | null | undefined): string {
  if (fase === undefined || fase === null) return "";
  const key = String(fase).toLowerCase();
  return PHASE_LABELS[PHASE_ALIASES[key] ?? key] ?? String(fase);
}

function num(v: unknown): number {
  const n = Number(v);
  return Number.isFinite(n) && n > 0 ? Math.trunc(n) : 0;
}

/**
 * The one-line count shown above the table: manual reviews always, plus excluded invoices
 * and failures when the toggle includes them.
 */
export function reportSummaryText(conteos: Partial<ManualReviewReport["conteos"]> | null | undefined, includeOthers: boolean): string {
  const c = conteos || {};
  const parts = [`${num(c.revision_manual)} en revisión manual`];
  if (includeOthers) {
    const ex = num(c.excluidas);
    const fa = num(c.fallas);
    parts.push(`${ex} ${ex === 1 ? "excluida" : "excluidas"}`);
    parts.push(`${fa} ${fa === 1 ? "falla" : "fallas"}`);
  }
  return parts.join(" · ");
}
