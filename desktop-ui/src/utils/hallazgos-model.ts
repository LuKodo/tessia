// Pure helpers for the "Hallazgos" section (no Vue, no fetch).
// Port of webui-src/src/hallazgosModel.js.

import type { HallazgoUpdate } from "../types/api";

// Must stay equal to the backend limit on the description (code points).
export const MAX_DESCRIPTION = 2000;

/** The editable fields of a hallazgo; any of them may be missing (older backends omit `devolucion`). */
export interface HallazgoFields {
  genera_glosa?: boolean | null;
  devolucion?: boolean | null;
  descripcion?: string | null;
}

interface Searchable {
  codigo?: string | null;
  detalle?: string | null;
}

// Lower-cased, accent-free text used for searching.
function fold(value: unknown): string {
  return String(value ?? "").normalize("NFD").replace(/\p{Diacritic}/gu, "").toLowerCase();
}

/**
 * Items whose code or detail contain the query (case and accent insensitive).
 * A blank query returns every item; null items are tolerated and dropped.
 */
export function filterHallazgos<T extends Searchable>(
  items: readonly (T | null | undefined)[] | null | undefined,
  query: string | null | undefined,
): T[] {
  const list = (Array.isArray(items) ? items : []).filter((item): item is T => Boolean(item));
  const needle = fold(query).trim();
  if (!needle) return list;
  return list.filter((item) => fold(item.codigo).includes(needle) || fold(item.detalle).includes(needle));
}

/** Trims and turns CRLF / lone CR into LF, so the textarea value and the stored value compare equal. */
export function normalizeDescription(text: unknown): string {
  return String(text ?? "").replace(/\r\n?/g, "\n").trim();
}

/**
 * True only when a checkbox or the normalized description differ.
 * A missing `devolucion` (older backend) counts as false.
 */
export function needsSave(item: HallazgoFields | null | undefined, draft: HallazgoFields | null | undefined): boolean {
  const current = item || {};
  const next = draft || {};
  if (Boolean(current.genera_glosa) !== Boolean(next.genera_glosa)) return true;
  if (Boolean(current.devolucion) !== Boolean(next.devolucion)) return true;
  return normalizeDescription(current.descripcion) !== normalizeDescription(next.descripcion);
}

/** Body of the PUT: always the three editable fields, description normalized. */
export function buildPayload(draft: HallazgoFields | null | undefined): Required<HallazgoUpdate> {
  const source = draft || {};
  return {
    genera_glosa: Boolean(source.genera_glosa),
    devolucion: Boolean(source.devolucion),
    descripcion: normalizeDescription(source.descripcion),
  };
}

/** Spanish message when the description is too long (code points), else null. */
export function descriptionError(text: unknown): string | null {
  if ([...normalizeDescription(text)].length > MAX_DESCRIPTION) {
    return `La descripción no puede superar ${MAX_DESCRIPTION} caracteres.`;
  }
  return null;
}

/** "N de M hallazgos" (singular when the total is one). */
export function summaryText(shown: number, total: number): string {
  return `${shown} de ${total} ${total === 1 ? "hallazgo" : "hallazgos"}`;
}
