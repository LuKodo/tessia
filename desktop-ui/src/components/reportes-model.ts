import type { RadicadasQuery, RadicadasReport } from "../types/api";

/** The filters of the "Reportes" page. Empty values mean "no filter". */
export interface ReportFilters {
  /** Cargue ids; none means every cargue. */
  cargues: string[];
  /** First and last filing day, YYYY-MM-DD, inclusive. */
  desde: string;
  hasta: string;
  /** Part of the invoice number. */
  q: string;
}

export const NO_FILTERS: ReportFilters = { cargues: [], desde: "", hasta: "", q: "" };

export function hasFilters(f: ReportFilters): boolean {
  return f.cargues.length > 0 || f.desde !== "" || f.hasta !== "" || f.q.trim() !== "";
}

/** The API query of the filters (the search is trimmed; empty values are dropped by the client). */
export function toQuery(f: ReportFilters): RadicadasQuery {
  return { cargue: f.cargues, desde: f.desde, hasta: f.hasta, q: f.q.trim() };
}

/** Why the date range cannot be queried ("" when it can): the server answers 400 for it. */
export function rangeError(f: ReportFilters): string {
  return f.desde && f.hasta && f.hasta < f.desde ? "La fecha «desde» no puede ser posterior a «hasta»." : "";
}

const count = new Intl.NumberFormat("es-CO");

/** One line above the table: how many filed invoices match, and how many of them are shown. */
export function summaryText(report: RadicadasReport): string {
  const total = report.total;
  if (total === 0) return "Sin facturas radicadas con estos filtros";
  const noun = total === 1 ? "factura radicada" : "facturas radicadas";
  if (report.truncado) {
    return `Mostrando ${count.format(report.filas.length)} de ${count.format(total)} ${noun}. Descarga el CSV para obtenerlas todas.`;
  }
  return `${count.format(total)} ${noun}`;
}
