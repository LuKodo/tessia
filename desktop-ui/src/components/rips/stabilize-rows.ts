import type { TrackingRow } from "../../types/api";

const keyOf = (r: Pick<TrackingRow, "cargue" | "factura">) => (r.cargue || "") + "/" + r.factura;

/**
 * Reuses the previous object of every row whose content did not change. The tracking list is polled
 * and every answer is a fresh set of objects; handing Lit the same object for an unchanged row lets
 * the table skip it entirely. Returns `prev` itself when nothing changed (same rows, same order).
 */
export function stabilizeRows(prev: TrackingRow[] | null | undefined, next: TrackingRow[]): TrackingRow[] {
  if (!prev?.length) return next;
  const before = new Map<string, { row: TrackingRow; json: string }>();
  for (const row of prev) before.set(keyOf(row), { row, json: JSON.stringify(row) });
  let same = prev.length === next.length;
  const out = next.map((row, i) => {
    const old = before.get(keyOf(row));
    const reused = old && old.json === JSON.stringify(row) ? old.row : row;
    if (reused !== prev[i]) same = false;
    return reused;
  });
  return same ? prev : out;
}
