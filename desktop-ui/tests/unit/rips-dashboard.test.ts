import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/components/rips/rips-dashboard";
import type { Kpis, Summary, TrackingRow } from "../../src/types/api";
import { CONSULTA, aggregateCalls, consultaCalls, fakeConsulta, pageCalls } from "../support/dashboard-consulta";
import { click, deepText, mountEl, q, qa, settle, text } from "../support/dashboard-dom";
import { listRow, portalView, trackingRow } from "../support/dashboard-fixtures";
import { routedApi } from "../support/dashboard-routed-api";

const holder = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("../../src/api", () => ({ api: new Proxy({}, { get: (_t, k) => holder.api[k as string] }) }));

const kpis: Kpis = {
  total: 42, no_radicado: 1, radicadas: 2, rechazadas: 3, radicacion_parcial: 4, nota_ajuste: 5,
  listas_para_radicar: 6, nota_credito: 7, en_proceso: 8, sin_autorizacion: 9,
};
// The chart still reads /api/resumen; its KPIs are no longer shown, so they differ on purpose.
const summary: Summary = {
  desde: "2026-10-01", hasta: "2026-10-31", kpis: { ...kpis, total: 7 },
  cargues: [{ cargue: "c1", nombre: "Convenio Uno", total: 3, gestionadas: 3, excluidas: 0, revision_manual: 0, radicadas: 1, valor_cargado: "10", valor_radicado: "5" }],
};
const rows = [listRow(trackingRow({ factura: "FVS1" })), listRow(trackingRow({ factura: "FVS2", cargue: "c2", alertas: ["falta_zip"] }))];
const facets = { estados: ["RECHAZADO", "TERMINADO"], alertas: { falta_zip: 2, nota_ajuste: 1 } };
const RESUMEN = "GET /api/resumen?desde=2026-10-01&hasta=2026-10-31";

async function mountDashboard(extra: Record<string, unknown> = {}, props: Record<string, unknown> = {}) {
  const routed = routedApi({ [CONSULTA]: fakeConsulta(rows, { kpis, facets }), [RESUMEN]: summary, ...extra });
  holder.api = routed.api as unknown as Record<string, unknown>;
  const el = await mountEl("rips-dashboard", props);
  await settle(el);
  return { ...routed, el };
}

const table = (el: Element) => q(el, "rips-table")!;
const invoices = (el: Element) => qa(table(el), ".invoice-link").map((b) => b.textContent!.trim());
const kpiCards = (el: Element) => qa(q(el, "rips-kpis")!, ".kpi");
const kpiCount = (el: Element, label: string) => {
  const card = kpiCards(el).find((k) => k.querySelector(".label")!.textContent!.trim() === label)!;
  return card.querySelector("[data-testid=kpi-count]")!.textContent!.trim();
};
const chartRows = (el: Element) => qa(q(q(el, "rips-chart")!, "rips-chart-bars")!, "g.row");
const summaryLine = (el: Element) => q(table(el), ".summary")!.textContent!.trim();

const filtersEl = (el: Element) => q(el, "rips-filters")!;
const field = (el: Element, label: string) => qa(filtersEl(el), "tx-form-field").find((f) => f.getAttribute("label") === label)!;
const control = (el: Element, label: string) => field(el, label).firstElementChild as HTMLElement;
const filterButton = (el: Element) => qa(filtersEl(el), "tx-button").find((b) => b.textContent!.trim() === "Filtrar")!;
const emit = (target: Element, name: string, detail: unknown) => target.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }));
const header = (el: Element, label: string) => qa(table(el), "th").find((th) => th.textContent!.trim().startsWith(label))!;

beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date(2026, 9, 15, 12));
});
afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
});

describe("<rips-dashboard>", () => {
  it("asks the server for the first page and for the aggregates at once and shows only what it answers", async () => {
    const { el, calls, callsTo } = await mountDashboard();
    expect(pageCalls(calls).map((p) => Object.fromEntries(p))).toEqual([{ agregados: "no", limit: "300" }]);
    expect(aggregateCalls(calls).map((p) => Object.fromEntries(p))).toEqual([{ limit: "0" }]);
    expect(callsTo("GET", "/api/seguimiento")).toHaveLength(0); // the whole-table route is not used any more
    expect(callsTo("GET", "/api/resumen?desde=2026-10-01&hasta=2026-10-31")).toHaveLength(1);
    expect(chartRows(el)[0]!.textContent).toContain("Convenio Uno");
    expect(invoices(el)).toEqual(["FVS1", "FVS2"]);
    expect(summaryLine(el)).toBe("2 facturas");
  });

  it("sends the page and the aggregates requests without waiting for each other", async () => {
    let release!: () => void;
    const gate = new Promise<void>((r) => (release = r));
    const answer = fakeConsulta(rows, { kpis, facets });
    const { calls, el } = await mountDashboard({
      [CONSULTA]: async (call: { path: string }) => {
        await gate;
        return answer(call as never);
      },
    });
    expect(consultaCalls(calls)).toHaveLength(2); // both are in flight before either answered
    release();
    await settle(el);
  });

  it("lays the chart, cards, filters and table out in one column", async () => {
    const { el } = await mountDashboard();
    expect(["rips-chart", "rips-kpis", "rips-filters", "rips-table"].map((t) => !!q(el, t))).toEqual([true, true, true, true]);
    expect(text(q(el, "rips-filters")!)).toContain("Filtros de búsqueda");
    expect(q(el, ".column")).not.toBeNull();
  });

  it("polls the rows and the aggregates every 10 s and the summary every 30 s", async () => {
    const { calls, callsTo } = await mountDashboard();
    await vi.advanceTimersByTimeAsync(10000);
    expect(pageCalls(calls)).toHaveLength(2);
    expect(aggregateCalls(calls)).toHaveLength(2);
    expect(callsTo("GET", "/api/resumen?desde=2026-10-01&hasta=2026-10-31")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(20000);
    expect(callsTo("GET", "/api/resumen?desde=2026-10-01&hasta=2026-10-31")).toHaveLength(2);
  });

  it("shows the summary error when /api/resumen fails", async () => {
    const { el } = await mountDashboard({ [RESUMEN]: new Response(JSON.stringify({ error: "sin base" }), { status: 500 }) });
    expect(q(q(el, "rips-chart")!, ".error")!.textContent).toContain("No se pudo cargar el resumen: sin base");
  });

  it("takes the KPI cards from the consulta answer, not from /api/resumen", async () => {
    const { el } = await mountDashboard();
    expect(kpiCount(el, "Total facturas")).toBe("42");
    expect(kpiCount(el, "Radicadas")).toBe("2");
    expect(kpiCount(el, "Sin autorización")).toBe("9");
  });

  it("shows the error alert when the rows cannot be loaded, and the empty state with no rows", async () => {
    const { el } = await mountDashboard({ [CONSULTA]: new Response(JSON.stringify({ error: "sin base" }), { status: 503 }) });
    expect(q(table(el), ".load-error")!.getAttribute("description")).toBe("No se pudieron cargar las facturas: sin base");
    expect(text(table(el))).toContain("Sin facturas cargadas");
  });

  it("keeps the rows it has when a later poll fails", async () => {
    let fail = false;
    const answer = fakeConsulta(rows, { kpis, facets });
    const { el } = await mountDashboard({ [CONSULTA]: (call: never) => (fail ? new Response(JSON.stringify({ error: "caido" }), { status: 500 }) : answer(call)) });
    fail = true;
    await vi.advanceTimersByTimeAsync(10000);
    await settle(el);
    expect(invoices(el)).toEqual(["FVS1", "FVS2"]);
    expect(q(table(el), ".load-error")!.getAttribute("description")).toContain("caido");
  });

  it("offers the estados and the alert counts of the facets to the filters", async () => {
    const { el } = await mountDashboard();
    const filters = filtersEl(el) as HTMLElement & { estadoOptions: string[]; alertCounts: Record<string, number> };
    expect(filters.estadoOptions).toEqual(["RECHAZADO", "TERMINADO"]);
    expect(filters.alertCounts).toEqual({ falta_zip: 2, nota_ajuste: 1 });
  });

  it("typing a filter does nothing until Filtrar; then the server is asked with it, for the rows and the aggregates", async () => {
    const { el, calls } = await mountDashboard();
    emit(control(el, "Número de factura"), "tx-input", { value: "  FVS2 " });
    emit(control(el, "Fecha desde"), "tx-input", { value: "2026-10-01" });
    await settle(el);
    expect(consultaCalls(calls)).toHaveLength(2);
    click(filterButton(el));
    await settle(el);
    const page = pageCalls(calls).at(-1)!;
    expect(Object.fromEntries(page)).toEqual({ factura: "fvs2", desde: "2026-10-01", agregados: "no", limit: "300" });
    expect(Object.fromEntries(aggregateCalls(calls).at(-1)!)).toEqual({ factura: "fvs2", desde: "2026-10-01", limit: "0" });
  });

  it("filtering by estado asks the server with estado=... and shows what it answers", async () => {
    const { el, calls } = await mountDashboard({
      [CONSULTA]: fakeConsulta((p) => (p.get("estado") === "TERMINADO" ? [rows[1]!] : rows), { kpis, facets }),
    });
    emit(control(el, "Estado"), "tx-change", { value: "TERMINADO" });
    click(filterButton(el));
    await settle(el);
    expect(pageCalls(calls).at(-1)!.get("estado")).toBe("TERMINADO");
    expect(aggregateCalls(calls).at(-1)!.get("estado")).toBe("TERMINADO");
    expect(invoices(el)).toEqual(["FVS2"]);
    expect(summaryLine(el)).toBe("1 factura (filtradas)");
  });

  it("alert filters travel as one comma list", async () => {
    const { el, calls } = await mountDashboard();
    const boxes = qa(filtersEl(el), "tx-checkbox");
    const box = (label: string) => boxes.find((b) => b.textContent!.includes(label))!;
    emit(box("Falta ZIP XML/PDF"), "tx-change", {});
    emit(box("Requiere nota ajuste"), "tx-change", {});
    await settle(el);
    expect(pageCalls(calls).at(-1)!.get("alerta")).toBe("falta_zip,nota_ajuste");
  });

  it("a KPI card click filters by its pipeline on the server", async () => {
    const { el, calls } = await mountDashboard({
      [CONSULTA]: fakeConsulta((p) => (p.get("pipeline") === "radicada" ? [rows[0]!] : rows), { kpis, facets }),
    });
    click(kpiCards(el).find((k) => k.querySelector(".label")!.textContent!.trim() === "Radicadas")!);
    await settle(el);
    expect(pageCalls(calls).at(-1)!.get("pipeline")).toBe("radicada");
    expect(aggregateCalls(calls).at(-1)!.get("pipeline")).toBe("radicada");
    expect(invoices(el)).toEqual(["FVS1"]);
  });

  it("clicking a chart bar sets the cargue filter and queries the server again", async () => {
    const { el, calls } = await mountDashboard({
      [CONSULTA]: fakeConsulta((p) => (p.get("cargue") === "c1" ? [rows[0]!] : rows), { kpis: { ...kpis, total: 3 }, facets }),
    });
    expect(invoices(el)).toEqual(["FVS1", "FVS2"]);
    click(chartRows(el)[0]!);
    await settle(el);
    expect(pageCalls(calls).at(-1)!.get("cargue")).toBe("c1");
    expect(aggregateCalls(calls).at(-1)!.get("cargue")).toBe("c1");
    expect(invoices(el)).toEqual(["FVS1"]);
    expect(kpiCount(el, "Total facturas")).toBe("3");
  });

  it("clicking a sortable header asks the server for that sort (descending first) and starts on page one", async () => {
    const { el, calls } = await mountDashboard();
    click(header(el, "Fecha factura"));
    await settle(el);
    expect(Object.fromEntries(pageCalls(calls).at(-1)!)).toEqual({ sort: "fecha_factura", dir: "desc", agregados: "no", limit: "300" });
    expect(Object.fromEntries(aggregateCalls(calls).at(-1)!)).toEqual({ sort: "fecha_factura", dir: "desc", limit: "0" });
    expect(header(el, "Fecha factura").getAttribute("aria-sort")).toBe("descending");
    click(header(el, "Fecha factura"));
    await settle(el);
    expect(pageCalls(calls).at(-1)!.get("dir")).toBe("asc");
    click(header(el, "Actualizado"));
    await settle(el);
    expect(pageCalls(calls).at(-1)!.get("sort")).toBe("actualizado_en");
  });

  describe("paging", () => {
    const many = Array.from({ length: 5 }, (_, i) => listRow(trackingRow({ factura: `F${i}` })));
    const mountPaged = (extra: Record<string, unknown> = {}) => mountDashboard({ [CONSULTA]: fakeConsulta(many, { kpis, facets }), ...extra }, { pageSize: 2 });
    const more = (el: Element) => q(table(el), ".more tx-button");

    it("shows the total and the loaded count and offers Cargar más while rows are left", async () => {
      const { el } = await mountPaged();
      expect(invoices(el)).toEqual(["F0", "F1"]);
      expect(summaryLine(el)).toBe("5 facturas · mostrando 2");
      expect(more(el)).not.toBeNull();
    });

    it("Cargar más appends the next page with the cursor and keeps what was loaded", async () => {
      const { el, calls } = await mountPaged();
      const before = qa(table(el), "tbody tr").map((tr) => tr);
      click(more(el));
      await settle(el);
      expect(pageCalls(calls).at(-1)!.get("cursor")).toBe("c2");
      expect(pageCalls(calls).at(-1)!.get("limit")).toBe("2");
      expect(invoices(el)).toEqual(["F0", "F1", "F2", "F3"]);
      expect(qa(table(el), "tbody tr").slice(0, 2)).toEqual(before); // the rows already there are not rebuilt
      expect(summaryLine(el)).toBe("5 facturas · mostrando 4");
      click(more(el));
      await settle(el);
      expect(invoices(el)).toEqual(["F0", "F1", "F2", "F3", "F4"]);
      expect(summaryLine(el)).toBe("5 facturas");
      expect(more(el)).toBeNull();
    });

    it("the next poll refetches the loaded range in one request, so the user keeps their place", async () => {
      const { el, calls } = await mountPaged();
      click(more(el));
      await settle(el);
      await vi.advanceTimersByTimeAsync(10000);
      await settle(el);
      const last = pageCalls(calls).at(-1)!;
      expect(last.get("limit")).toBe("4");
      expect(last.has("cursor")).toBe(false);
      expect(invoices(el)).toEqual(["F0", "F1", "F2", "F3"]);
      click(more(el)); // the cursor of the refetched range still leads to the rest
      await settle(el);
      expect(pageCalls(calls).at(-1)!.get("cursor")).toBe("c4");
      expect(invoices(el)).toEqual(["F0", "F1", "F2", "F3", "F4"]);
    });

    it("changing the filters or the sort goes back to the first page", async () => {
      const { el, calls } = await mountPaged();
      click(more(el));
      await settle(el);
      expect(invoices(el)).toHaveLength(4);
      click(header(el, "Fecha factura"));
      await settle(el);
      const afterSort = pageCalls(calls).at(-1)!;
      expect(afterSort.get("limit")).toBe("2");
      expect(afterSort.has("cursor")).toBe(false);
      expect(invoices(el)).toEqual(["F0", "F1"]);
      click(more(el));
      await settle(el);
      emit(control(el, "SSC"), "tx-input", { value: "5" });
      click(filterButton(el));
      await settle(el);
      expect(pageCalls(calls).at(-1)!.get("limit")).toBe("2");
      expect(pageCalls(calls).at(-1)!.has("cursor")).toBe(false);
      expect(invoices(el)).toEqual(["F0", "F1"]);
    });

    it("ignores a second click on Cargar más while the page is loading", async () => {
      const answer = fakeConsulta(many, { kpis, facets });
      let release!: () => void;
      let hold = false;
      const gate = new Promise<void>((r) => (release = r));
      const { el, calls } = await mountPaged({
        [CONSULTA]: async (call: never) => {
          if (hold) await gate;
          return answer(call);
        },
      });
      hold = true;
      click(more(el));
      await settle(el);
      click(more(el));
      await settle(el);
      expect(pageCalls(calls).filter((p) => p.has("cursor"))).toHaveLength(1);
      release();
      await settle(el);
      expect(invoices(el)).toHaveLength(4);
    });
  });

  it("opens the detail modal and the portal modal from the table, re-polling the rows after a portal load", async () => {
    const { el, callsTo, calls } = await mountDashboard({
      "GET /api/facturas/FVS1/portal": portalView({ factura: "FVS1" }),
      "GET /api/facturas/FVS1/seguimiento?cargue=c1": trackingRow({ factura: "FVS1", lineas_convenio: [{ ssc: "1", cum: "CUM-1", valor_unitario: "", valor_total: "", autorizacion: "", id_mipres: "" }] }),
    });
    const detalle = q(el, "detalle-modal") as HTMLElement & { row: unknown };
    expect(detalle.row).toBeNull();
    expect(callsTo("GET", "/api/facturas/FVS1/seguimiento?cargue=c1")).toHaveLength(0); // the list never carried the detail
    click(q(table(el), "tx-button[title='Ver detalle de FVS1']"));
    await settle(el);
    expect((detalle.row as { factura: string }).factura).toBe("FVS1");
    expect(callsTo("GET", "/api/facturas/FVS1/seguimiento?cargue=c1")).toHaveLength(1);
    expect(deepText(detalle)).toContain("CUM-1");
    detalle.dispatchEvent(new CustomEvent("close"));
    await settle(el);
    expect(detalle.row).toBeNull();

    click(q(table(el), ".invoice-link"));
    await settle(el);
    expect((q(el, "portal-modal") as HTMLElement & { open: boolean }).open).toBe(true);
    expect(callsTo("GET", "/api/facturas/FVS1/portal")).toHaveLength(1);
    expect(pageCalls(calls)).toHaveLength(2); // the portal load re-polled the rows, not the aggregates
    expect(aggregateCalls(calls)).toHaveLength(1);
    q(el, "portal-modal")!.dispatchEvent(new CustomEvent("close"));
    await settle(el);
    expect((q(el, "portal-modal") as HTMLElement & { open: boolean }).open).toBe(false);
  });

  it("keeps the open detail live: the next poll refreshes the row it shows", async () => {
    let answer: TrackingRow[] = [listRow(trackingRow({ factura: "FVS1", estado: "EN_PROCESO" }))];
    const { el } = await mountDashboard({ [CONSULTA]: (call: never) => fakeConsulta(answer)(call) });
    click(q(table(el), "tx-button[title='Ver detalle de FVS1']"));
    await settle(el);
    answer = [listRow(trackingRow({ factura: "FVS1", estado: "TERMINADO" }))];
    await vi.advanceTimersByTimeAsync(10000);
    await settle(el);
    expect(((q(el, "detalle-modal") as HTMLElement & { row: { estado: string } }).row).estado).toBe("TERMINADO");
  });

  it("closes the detail when its row disappears from the poll", async () => {
    let answer: TrackingRow[] = [listRow(trackingRow({ factura: "FVS1" }))];
    const { el } = await mountDashboard({ [CONSULTA]: (call: never) => fakeConsulta(answer)(call) });
    click(q(table(el), "tx-button[title='Ver detalle de FVS1']"));
    await settle(el);
    answer = [];
    await vi.advanceTimersByTimeAsync(10000);
    await settle(el);
    expect((q(el, "detalle-modal") as HTMLElement & { row: unknown }).row).toBeNull();
  });
});
