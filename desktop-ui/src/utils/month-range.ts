// Month helper for the dashboard cards: months are "YYYY-MM" strings and ranges are the
// { desde, hasta } days the /api/resumen endpoint expects.
const pad = (n: number) => String(n).padStart(2, "0");

export interface DayRange {
  desde: string;
  hasta: string;
}

export function monthOf(date: Date): string {
  return date.getFullYear() + "-" + pad(date.getMonth() + 1);
}

export function monthRange(ym: string): DayRange {
  const [y = 0, m = 0] = ym.split("-").map(Number);
  const last = new Date(y, m, 0).getDate();
  return { desde: ym + "-01", hasta: ym + "-" + pad(last) };
}
