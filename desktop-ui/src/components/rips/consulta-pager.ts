import type { SeguimientoConsulta, TrackingRow } from "../../types/api";

/** Rows per page of the table (the server's default). */
export const PAGE_SIZE = 300;
/** The most rows the server answers in one request. */
export const MAX_LIMIT = 1000;

/** The rows loaded so far and the cursor of the page after them (null: that was the last one). */
export interface Page {
  rows: TrackingRow[];
  next: string | null;
}

export interface PageRequest {
  limit: number;
  cursor?: string;
}

/** One consulta request for rows (the filters, the sort and agregados=no are bound by the caller). */
export type FetchPage = (req: PageRequest) => Promise<Pick<SeguimientoConsulta, "filas" | "next_cursor">>;

/**
 * Loads the first `target` rows. One request while `target` fits in a server page; a longer range
 * (the user loaded several pages) is chained by cursor, so a refresh keeps everything they loaded.
 */
export async function loadRange(fetchPage: FetchPage, target: number): Promise<Page> {
  let page: Page = { rows: [], next: null };
  let first = true;
  while (first || (page.next && page.rows.length < target)) {
    const req: PageRequest = { limit: Math.min(Math.max(target - page.rows.length, 1), MAX_LIMIT) };
    if (page.next) req.cursor = page.next;
    const res = await fetchPage(req);
    page = { rows: first ? res.filas : page.rows.concat(res.filas), next: res.next_cursor };
    first = false;
    if (!res.filas.length) break;
  }
  return page;
}

/** Appends the page after `page` (`size` rows); `page` itself when it was the last one. */
export async function loadMore(fetchPage: FetchPage, page: Page, size: number): Promise<Page> {
  if (!page.next) return page;
  const res = await fetchPage({ limit: Math.min(size, MAX_LIMIT), cursor: page.next });
  return { rows: page.rows.concat(res.filas), next: res.next_cursor };
}
