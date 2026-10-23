import type { PortalFinding, PortalView, TrackingRow } from "../types/api";

// Pure logic for the Mutual Ser portal follow-up (table icon + modal). No Vue, no DOM:
// RipsTable / PortalModal / PortalContenido render what these helpers return.
//
// A "summary" is either portalRowSummary(tracking row) (GET /api/seguimiento) or the modal
// payload (GET /api/facturas/{factura}/portal); both carry
//   { consultado, estado, advertencias, errores, final }
// and any of them may be missing (= defaults).

export interface PortalSummary {
  consultado?: boolean;
  estado?: string;
  advertencias?: number;
  errores?: number;
  final?: boolean;
}

export type PortalIconKey = "pendiente" | "procesando" | "advertencia" | "error" | "ok";

/** Color names of the portal status icon. */
export type PortalColor = "neutral" | "error" | "warning" | "info" | "success";

export interface PortalIconInfo {
  key: PortalIconKey;
  color: PortalColor;
  label: string;
}

const up = (v: unknown) => String(v ?? "").toUpperCase();
const hasAny = (v: unknown, parts: string[]) => parts.some((p) => up(v).includes(p));
const isErrorType = (v: unknown) => hasAny(v, ["ERROR", "RECHAZ"]);
const isWarningType = (v: unknown) => hasAny(v, ["ADVERT"]);

/** The portal summary of a tracking row, whose fields come prefixed (portal_consultado, ...) and may be absent. */
export function portalRowSummary(row: Partial<TrackingRow> | null | undefined): Required<PortalSummary> {
  const r = row || {};
  return {
    consultado: !!r.portal_consultado,
    estado: r.portal_estado || "",
    advertencias: Number(r.portal_advertencias) || 0,
    errores: Number(r.portal_errores) || 0,
    final: !!r.portal_final,
  };
}

/** Status icon of an invoice. Rules, in order: not consulted, errors, warnings, still processing, clean. */
export function portalIcon(summary: PortalSummary | null | undefined): PortalIconInfo {
  const s = summary || {};
  if (!s.consultado) return { key: "pendiente", color: "neutral", label: "Aún no consultado en el portal" };
  if (Number(s.errores) > 0 || isErrorType(s.estado)) {
    return { key: "error", color: "error", label: "Con errores en el portal" };
  }
  if (Number(s.advertencias) > 0 || isWarningType(s.estado)) {
    return { key: "advertencia", color: "warning", label: "Con advertencias" };
  }
  if (!s.final) return { key: "procesando", color: "info", label: "Procesando en el portal" };
  return { key: "ok", color: "success", label: "Validada sin advertencias" };
}

// Literal iconoir classes: the icon bundle only includes names written out under app/.
const ICON_CLASSES: Record<PortalIconKey, string> = {
  pendiente: "i-iconoir-clock",
  procesando: "i-iconoir-clock-rotate-right",
  advertencia: "i-iconoir-warning-triangle",
  error: "i-iconoir-warning-circle",
  ok: "i-iconoir-check-circle",
};

/** The iconoir class of a portalIcon key (pending when unknown). */
export function portalIconClass(key: string | undefined): string {
  return ICON_CLASSES[key as PortalIconKey] ?? ICON_CLASSES.pendiente;
}

const TONE_CLASSES: Record<PortalColor, string> = {
  neutral: "text-muted",
  error: "text-error",
  warning: "text-warning",
  info: "text-info",
  success: "text-success",
};

/** Text color class of a portal color (muted when unknown). */
export function portalToneClass(color: string | undefined): string {
  return TONE_CLASSES[color as PortalColor] ?? TONE_CLASSES.neutral;
}

export interface FindingGroup {
  tipo: string;
  label: string;
  color: PortalColor;
  items: Array<Partial<PortalFinding>>;
}

/**
 * Groups the portal findings by type: errors first, then warnings, then every other type in
 * order of first appearance. Order inside a group is the order received. Null/undefined/
 * non-object entries are ignored.
 */
export function groupPortalFindings(
  hallazgos: ReadonlyArray<Partial<PortalFinding> | null | undefined> | null | undefined,
): FindingGroup[] {
  const errors: FindingGroup = { tipo: "ERROR", label: "Errores", color: "error", items: [] };
  const warnings: FindingGroup = { tipo: "ADVERTENCIA", label: "Advertencias", color: "warning", items: [] };
  const others = new Map<string, FindingGroup>();
  for (const h of Array.isArray(hallazgos) ? hallazgos : []) {
    if (!h || typeof h !== "object") continue;
    if (isErrorType(h.tipo)) errors.items.push(h);
    else if (isWarningType(h.tipo)) warnings.items.push(h);
    else {
      const tipo = up(h.tipo).trim();
      let group = others.get(tipo);
      if (!group) {
        group = { tipo, label: tipo || "Otros", color: "neutral", items: [] };
        others.set(tipo, group);
      }
      group.items.push(h);
    }
  }
  return [errors, warnings, ...others.values()].filter((g) => g.items.length);
}

/** Color of the badge of a portal state. */
export function portalStatusColor(estado: string | null | undefined): PortalColor {
  if (isErrorType(estado)) return "error";
  if (isWarningType(estado)) return "warning";
  if (hasAny(estado, ["PROCES", "PEND"])) return "info";
  if (hasAny(estado, ["VALIDADO"])) return "success";
  return "neutral";
}

const present = (v: unknown) => v !== undefined && v !== null && String(v).trim() !== "";

/** The ordered [label, value] pairs of the "datos del portal" grid, skipping empty values. */
export function portalInfoRows(payload: Partial<PortalView> | null | undefined): Array<[string, string]> {
  const p = payload || {};
  const doc = p.documento_soporte;
  const docText =
    doc && present(doc.nombre_archivo)
      ? present(doc.estado)
        ? `${doc.nombre_archivo} (${doc.estado})`
        : String(doc.nombre_archivo)
      : "";
  const pairs: Array<[string, unknown]> = [
    ["Remisión", p.remision],
    ["Fecha de cargue", present(p.fecha_cargue_texto) ? p.fecha_cargue_texto : p.fecha_cargue],
    ["Usuario", p.usuario],
    ["Servicios", p.servicios],
    ["Modalidad", p.modalidad],
    ["Archivo", p.nombre_archivo],
    ["Id cargue", p.id_cargue],
    ["Id archivo", p.id_archivo],
    ["Id evento", p.id_evento],
    ["Tipo de evento", p.tipo_evento],
    ["Fecha de expedición", p.fecha_expedicion],
    ["Recibo", p.receipt_name],
    ["Devuelto", p.devuelto === true ? "Sí" : ""],
    ["Documento soporte", docText],
  ];
  return pairs.filter(([, v]) => present(v)).map(([k, v]) => [k, String(v)]);
}
