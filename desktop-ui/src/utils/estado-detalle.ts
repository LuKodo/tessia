/**
 * A detail text is "positive" when it reports success (radicado/confirmado) and does not
 * carry an explicit negative confirmation (confirmado=false).
 */
export function isPositiveDetail(detail: string | null | undefined): boolean {
  if (!detail) return false;
  if (/confirmado\s*=\s*false/i.test(detail)) return false;
  return /radicad|confirmad/i.test(detail);
}
