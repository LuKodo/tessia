import { describe, expect, it, vi } from "vitest";
import { loadMore, loadRange } from "../../src/components/rips/consulta-pager";
import type { PageRequest } from "../../src/components/rips/consulta-pager";
import type { TrackingRow } from "../../src/types/api";
import { trackingRow } from "../support/dashboard-fixtures";

// A server with `n` rows: a cursor is the offset of the next row ("c3").
function server(n: number) {
  const all: TrackingRow[] = Array.from({ length: n }, (_, i) => trackingRow({ factura: `F${i}` }));
  const requests: PageRequest[] = [];
  const fetchPage = vi.fn(async (req: PageRequest) => {
    requests.push(req);
    const from = req.cursor ? Number(req.cursor.slice(1)) : 0;
    const filas = all.slice(from, from + req.limit);
    return { filas, next_cursor: from + filas.length < n ? `c${from + filas.length}` : null };
  });
  return { fetchPage, requests, all };
}
const names = (rows: TrackingRow[]) => rows.map((r) => r.factura);

describe("consulta pager", () => {
  it("loadRange asks for the whole range in one request while it fits in a page of the server", async () => {
    const s = server(10);
    const page = await loadRange(s.fetchPage, 4);
    expect(s.requests).toEqual([{ limit: 4 }]);
    expect(names(page.rows)).toEqual(["F0", "F1", "F2", "F3"]);
    expect(page.next).toBe("c4");
  });

  it("loadRange ends with no cursor when the table has fewer rows than asked", async () => {
    const s = server(3);
    const page = await loadRange(s.fetchPage, 300);
    expect(names(page.rows)).toEqual(["F0", "F1", "F2"]);
    expect(page.next).toBeNull();
    expect(s.requests).toHaveLength(1);
  });

  it("loadRange never asks for more than 1000 rows at once: a longer range is chained by cursor", async () => {
    const s = server(2500);
    const page = await loadRange(s.fetchPage, 2200);
    expect(s.requests).toEqual([{ limit: 1000 }, { limit: 1000, cursor: "c1000" }, { limit: 200, cursor: "c2000" }]);
    expect(page.rows).toHaveLength(2200);
    expect(page.next).toBe("c2200");
  });

  it("loadRange stops when the server runs out of rows or answers an empty page", async () => {
    const s = server(1500);
    const page = await loadRange(s.fetchPage, 3000);
    expect(page.rows).toHaveLength(1500);
    expect(page.next).toBeNull();
    const stuck = vi.fn(async () => ({ filas: [], next_cursor: "c1" }));
    const empty = await loadRange(stuck, 50);
    expect(empty.rows).toEqual([]);
    expect(stuck).toHaveBeenCalledTimes(1);
  });

  it("loadMore appends the next page after the loaded rows using the cursor", async () => {
    const s = server(5);
    const first = await loadRange(s.fetchPage, 2);
    const second = await loadMore(s.fetchPage, first, 2);
    expect(s.requests.at(-1)).toEqual({ limit: 2, cursor: "c2" });
    expect(names(second.rows)).toEqual(["F0", "F1", "F2", "F3"]);
    expect(second.next).toBe("c4");
    const last = await loadMore(s.fetchPage, second, 2);
    expect(names(last.rows)).toEqual(["F0", "F1", "F2", "F3", "F4"]);
    expect(last.next).toBeNull();
  });

  it("loadMore without a cursor changes nothing and asks nothing", async () => {
    const s = server(1);
    const page = await loadRange(s.fetchPage, 5);
    const again = await loadMore(s.fetchPage, page, 5);
    expect(again).toBe(page);
    expect(s.fetchPage).toHaveBeenCalledTimes(1);
  });
});
