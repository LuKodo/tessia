import type { InvoiceTracking } from "../types/api";

// Pure decision logic of the invoice detail modal (DetalleModal.vue).

/** A tracking row (with its detail fields when the invoice detail was loaded), possibly incomplete: the helpers read defensively, like the old code. */
export type DetailRow = Partial<InvoiceTracking> | null | undefined;

const STEP_LABELS = ["ZIP XML/PDF", "JSON", "TXT/CUV", "ZIP CRC"];
const RAD_LABELS = ["Radicación ZIP XML/PDF", "Radicación TXT/CUV", "Radicación JSON", "Radicación ZIP CRC"];
const TIMELINE_LABELS = ["Cargue ZIP XML/PDF a Mutualser", "Radicación TXT/CUV", "Radicación JSON", "Radicación ZIP CRC"];
const REJECTION_RE = /\b((?:RF-)?FE-[A-Z]+-\d+|RF-FE-\d+|RVG\d+)\b/;

const isDone = (status: string | undefined) => status === "ok";

/**
 * Filing step 1-4 is done. Steps 3-4 are pending while a nota de ajuste forces the reprocess
 * (same rule as the legacy modal).
 */
export function pasoOk(row: DetailRow, paso: number): boolean {
  if ((paso === 3 || paso === 4) && row?.requiere_nota_ajuste) return false;
  return isDone((row?.radicacion || [])[paso - 1]);
}

export function stepperSteps(row: DetailRow): Array<{ label: string; ok: boolean }> {
  const art = row?.artefactos || [];
  return [
    ...STEP_LABELS.map((label, i) => ({ label, ok: isDone(art[i]) })),
    ...RAD_LABELS.map((label, i) => ({ label, ok: pasoOk(row, i + 1) })),
  ];
}

export function rejectionCode(text: string | null | undefined): string | null {
  const m = String(text || "").match(REJECTION_RE);
  return m?.[1] ?? null;
}

/** Color names of the plazo badge. */
export type PlazoBadgeColor = "error" | "success" | "neutral";

export interface PlazoInfo {
  kind: "vencida" | "na" | "completa" | "pendiente";
  fecha: string;
  paso?: number;
  badge: { text: string; color: PlazoBadgeColor } | null;
}

/** The 22-business-day deadline is never computed here; only the RFFE022 flag marks an invoice as expired. */
export function plazoInfo(row: DetailRow): PlazoInfo {
  const fecha = row?.fecha_factura || "";
  if (row?.vencida_rffe022) return { kind: "vencida", fecha, badge: { text: "NOTA CRÉDITO · vencido (RFFE022)", color: "error" } };
  if (row?.resultado && row.resultado !== "preparada") return { kind: "na", fecha, badge: null };
  const pending = [1, 2, 3, 4].find((p) => !pasoOk(row, p));
  if (!pending) return { kind: "completa", fecha, badge: { text: "Radicación completa", color: "success" } };
  const code = rejectionCode(row?.estado_detalle);
  const badge: { text: string; color: PlazoBadgeColor } = code
    ? { text: `Paso ${pending} rechazado (${code})`, color: "error" }
    : { text: `Falta Paso ${pending}`, color: "neutral" };
  return { kind: "pendiente", fecha, paso: pending, badge };
}

const num = (v: string | undefined | null) => (v === undefined || v === null || v === "" ? NaN : Number(v));

/** XML total minus convenio total, or null when either is missing. */
export function valueDiff(row: DetailRow): { diff: number; ok: boolean } | null {
  const xml = num(row?.valor_xml);
  const conv = num(row?.valor_convenio);
  if (!Number.isFinite(xml) || !Number.isFinite(conv)) return null;
  const diff = xml - conv;
  return { diff, ok: Math.abs(diff) < 0.01 };
}

/** Filed (step 4 done) without an authorization number. */
export function glosaRisk(row: DetailRow): boolean {
  return pasoOk(row, 4) && !row?.autorizacion;
}

export function hasConvenio(row: DetailRow): boolean {
  return !!row?.ssc || (row?.lineas_convenio || []).length > 0;
}

export type TimelineState = "completado" | "reproceso" | "pendiente";

export function timelineItems(row: DetailRow): Array<{ label: string; state: TimelineState }> {
  return TIMELINE_LABELS.map((label, i) => {
    const paso = i + 1;
    let state: TimelineState = "pendiente";
    if (pasoOk(row, paso)) state = "completado";
    else if ((paso === 3 || paso === 4) && row?.requiere_nota_ajuste) state = "reproceso";
    return { label, state };
  });
}

/** One card per generated artifact (ok or missing). */
export function stageCards(row: DetailRow): Array<{ titulo: string; ok: boolean }> {
  const art = row?.artefactos || [];
  return STEP_LABELS.map((titulo, i) => ({ titulo, ok: isDone(art[i]) }));
}

export function notesOf(row: DetailRow): { ajuste: boolean; credito: boolean } {
  return { ajuste: !!row?.requiere_nota_ajuste, credito: !!(row?.requiere_nota_credito || row?.vencida_rffe022) };
}

const money = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
export function formatMoney(n: number): string {
  return "$ " + money.format(Number(n));
}
