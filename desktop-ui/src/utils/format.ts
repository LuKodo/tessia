/** Date-time in es-CO with a 24 hour clock; "" for a missing or invalid date. */
export function fmtTime(s?: string | null): string {
  if (!s) return "";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return "";
  return d.toLocaleString("es-CO", { hour12: false });
}
