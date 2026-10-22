import type { CarguePoint } from "../types/api";

// Pure model behind the "Avance de radicación por cargue" chart: one horizontal bar per
// cargue (radicadas / gestionadas). No Vue, no DOM. The backend (/api/resumen `cargues`)
// already sends the cargues most recent first.
export const CHART_DEFAULT_LIMIT = 12;

const money = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 0 });
const FLAT_LABEL = "Cargue sin carpeta"; // the "." folder: files directly under the automatic dir

/** % radicado = radicadas over gestionadas (radicadas are a subset; excluded invoices cannot be filed). */
export const carguePct = (c: CarguePoint): number => (c.gestionadas ? Math.round((100 * c.radicadas) / c.gestionadas) : 0);

// The backend sends `gestionadas` inclusive of `radicadas` (700 of 1000 with 100 radicadas).
// The chart is additive, so the "Gestionadas" segment is the ones still NOT radicadas.
const pendingOf = (c: CarguePoint): number => Math.max(0, (c.gestionadas || 0) - (c.radicadas || 0));

export const MIN_SEGMENT_W = 3; // px: a non-empty segment never disappears
const round2 = (n: number): number => Math.round(n * 100) / 100;

export interface ChartSegment {
  key: "radicadas" | "gestionadas" | "revision_manual" | "excluidas";
  label: string;
  cls: string;
  count: number;
  w: number;
  x: number;
}

/**
 * Splits a track of width W into contiguous segments, in order: Radicadas | Gestionadas (sin
 * radicar) | Revisión manual | Excluidas. Empty segments are omitted. The bar always fills the
 * track: counts are scaled to their own sum, tiny segments get MIN_SEGMENT_W taken from the
 * widest one, and rounding residue goes to the widest one so widths add up to W exactly.
 */
export function segmentsOf(c: CarguePoint, W: number): ChartSegment[] {
  const defs = [
    { key: "radicadas", label: "Radicadas", cls: "seg-filed", count: c.radicadas || 0 },
    { key: "gestionadas", label: "Gestionadas", cls: "seg-managed", count: pendingOf(c) },
    { key: "revision_manual", label: "Revisión manual", cls: "seg-manual", count: c.revision_manual || 0 },
    { key: "excluidas", label: "Excluidas", cls: "seg-excluded", count: c.excluidas || 0 },
  ] as const;
  const used = defs.filter((d) => d.count > 0);
  const total = used.reduce((n, d) => n + d.count, 0);
  if (!total) return [];
  const segs: ChartSegment[] = used.map((d) => ({ ...d, w: Math.max(MIN_SEGMENT_W, round2((W * d.count) / total)), x: 0 }));
  const widest = segs.reduce((m, s) => (s.w > m.w ? s : m), segs[0]!);
  widest.w = round2(widest.w + (W - segs.reduce((n, s) => n + s.w, 0)));
  let x = 0;
  for (const s of segs) {
    s.x = round2(x);
    x += s.w;
  }
  return segs;
}

/** Numbers line of a row; additive and mirrors the bar: Radicadas + Gestionadas (sin radicar) + Revisión manual (if > 0) + Excluidas = Total. */
export function statsOf(c: CarguePoint): string[] {
  const out = [`Radicadas ${c.radicadas || 0}`, `Gestionadas ${pendingOf(c)}`];
  if (c.revision_manual > 0) out.push(`Revisión manual ${c.revision_manual}`);
  out.push(`Excluidas ${c.excluidas || 0}`, `Total ${c.total || 0}`);
  return out;
}

const CHAR_W = 6.6; // px per character at 12px

/** Joins the stats with " · " into as few lines as fit maxWidth. */
export function packStats(stats: string[], maxWidth: number): string[] {
  const lines: string[] = [];
  let cur = "";
  for (const s of stats) {
    const next = cur ? `${cur} · ${s}` : s;
    if (cur && next.length * CHAR_W > maxWidth) {
      lines.push(cur);
      cur = s;
    } else {
      cur = next;
    }
  }
  if (cur) lines.push(cur);
  return lines;
}

export function cargueLabel(c: CarguePoint): string {
  if (c.nombre) return c.nombre;
  return c.cargue === "." ? FLAT_LABEL : c.cargue;
}

export function truncateLabel(text: unknown, max: number): string {
  const s = String(text ?? "");
  if (s.length <= max) return s;
  return s.slice(0, Math.max(0, max - 1)) + "…";
}

export function visibleCargues(cargues: CarguePoint[] | null | undefined, showAll: boolean, limit = CHART_DEFAULT_LIMIT) {
  const all = cargues || [];
  const items = showAll ? all : all.slice(0, limit);
  return { items, hidden: all.length - items.length, canToggle: all.length > limit };
}

/** Text the "Lote / Cargue" filter gets when a bar is clicked: the folder id, or the file name for the flat "." folder. */
export function cargueFilterValue(c: CarguePoint): string {
  return c.cargue === "." ? c.nombre || "" : c.cargue;
}

const ddmmyyyy = (s: string | undefined): string =>
  /^\d{4}-\d{2}-\d{2}/.test(s || "") ? `${s!.slice(8, 10)}/${s!.slice(5, 7)}/${s!.slice(0, 4)}` : "";
const moneyOf = (v: string | undefined): string => "$" + money.format(Number(v) || 0);

export function tooltipOf(c: CarguePoint) {
  return {
    title: cargueLabel(c),
    cargue: c.cargue,
    inicio: ddmmyyyy(c.fecha_inicio),
    total: c.total,
    gestionadas: c.gestionadas, // inclusive: includes the radicadas
    gestionadasSinRadicar: pendingOf(c),
    excluidas: c.excluidas,
    revisionManual: c.revision_manual || 0,
    radicadas: c.radicadas,
    pct: carguePct(c),
    valorCargado: moneyOf(c.valor_cargado),
    valorRadicado: moneyOf(c.valor_radicado),
  };
}

export interface ChartRow {
  cargue: CarguePoint;
  y: number;
  h: number;
  labelY: number;
  barY: number;
  barH: number;
  segments: ChartSegment[];
  lines: string[];
  linesY: number;
  lineH: number;
  label: string;
  full: string;
  pct: number;
  pctText: string;
}

/**
 * SVG layout for width W. Each cargue is a row: the label (and the % radicado, right-aligned)
 * on top, then ONE stacked bar spanning the whole width, then the numbers (wrapped to fit).
 */
export function buildBars(items: CarguePoint[], W: number) {
  const pad = 8, barH = 14, labelH = 20, gap = 8, lineH = 15, bottom = 8;
  const barX = pad, barW = Math.max(120, W - 2 * pad);
  const labelW = Math.round(Math.min(280, Math.max(120, W * 0.5)));
  const maxChars = Math.max(6, Math.floor(labelW / CHAR_W));
  let y = pad;
  const rows = items.map((c): ChartRow => {
    const pct = carguePct(c);
    const lines = packStats(statsOf(c), barW);
    const barY = y + labelH;
    const h = labelH + barH + gap + lines.length * lineH + bottom;
    const row: ChartRow = {
      cargue: c, y, h, labelY: y + labelH - 6, barY, barH,
      segments: segmentsOf(c, barW).map((s) => ({ ...s, x: barX + s.x })),
      lines, linesY: barY + barH + gap + 11, lineH,
      label: truncateLabel(cargueLabel(c), maxChars), full: cargueLabel(c),
      pct, pctText: `${pct}% radicado`,
    };
    y += h;
    return row;
  });
  return { H: pad * 2 + rows.reduce((n, r) => n + r.h, 0), W, pad, barX, barW, rows };
}
