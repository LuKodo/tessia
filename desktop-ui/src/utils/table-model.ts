// Pure helpers of the tracking table (the sort cycle and the summary line); the server sorts and filters.

export type SortDirection = 1 | -1;

export interface SortState {
  key: string | null;
  dir: SortDirection;
}

/** A new column starts descending; clicking the active column flips direction. */
export function nextSort(sort: SortState, key: string): SortState {
  return { key, dir: sort.key === key ? (-sort.dir as SortDirection) : -1 };
}

export interface SubLineInput {
  /** Invoices the server counted for the filters; undefined while the aggregates are loading. */
  total: number | undefined;
  /** Rows loaded in the table. */
  loaded: number;
  filtered: boolean;
}

const facturas = (n: number) => (n === 1 ? "1 factura" : `${n} facturas`);

export function subLine({ total, loaded, filtered }: SubLineInput): string {
  if (total === undefined) return `${facturas(loaded)} ${loaded === 1 ? "cargada" : "cargadas"}`;
  return facturas(total) + (filtered ? " (filtradas)" : "") + (loaded < total ? ` · mostrando ${loaded}` : "");
}
