const money = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });

/** "$ 1.234,5" for a decimal string, the text as is when it is not a number, null when empty. */
export function amountText(v: string | null | undefined): string | null {
  if (!v) return null;
  const n = Number(v);
  return "$ " + (Number.isFinite(n) ? money.format(n) : v);
}

/** dd/MM/yyyy HH:mm in local time; null when empty, "" when the date is invalid. */
export function whenText(s: string | null | undefined): string | null {
  if (!s) return null;
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return "";
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getDate())}/${p(d.getMonth() + 1)}/${d.getFullYear()} ${p(d.getHours())}:${p(d.getMinutes())}`;
}
