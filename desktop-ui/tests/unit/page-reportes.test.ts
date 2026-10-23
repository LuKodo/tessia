import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/pages/page-reportes";
import type { PageReportes } from "../../src/pages/page-reportes";
import type { RadicadasReport } from "../../src/types/api";
import { cargue, radicada, reporte } from "../support/reportes-fixtures";
import { deep, deepAll, toggle, typeInto, settle } from "../support/revision-dom";
import { routedApi } from "../support/revision-routed-api";
import { createApiClient } from "../../src/utils/api-client";
import { createTessiaApi } from "../../src/utils/tessia-api";

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("../../src/api", () => ({
  get api() {
    return holder.api;
  },
}));

const REPORT = "/api/reportes/radicadas";
const failure = (message: string, status = 500) => new Response(JSON.stringify({ error: message }), { status });

const all = reporte([
  radicada({ factura: "EVSB1", cargue: "B", cargue_nombre: "Convenio B", fecha_radicacion: "2026-09-12T18:30:00Z", observaciones: "RIPS regenerado | radicada" }),
  radicada({ factura: "FVSA2", fecha_radicacion: "2026-09-12T12:30:00Z" }),
  radicada({ factura: "FVSA1", fecha_radicacion: "2026-09-10T12:30:00Z", observaciones: "" }),
]);

async function mountPage(routes: Parameters<typeof routedApi>[0] = {}) {
  const routed = routedApi({
    "GET /api/cargues": [cargue("A"), cargue("B")],
    [`GET ${REPORT}`]: all,
    ...routes,
  });
  holder.api = routed.api;
  const el = document.createElement("tx-page-reportes") as PageReportes;
  el.searchDelayMs = 0;
  document.body.append(el);
  await settle(el);
  return { ...routed, el };
}

type Mounted = Awaited<ReturnType<typeof mountPage>>;
const facturas = (m: Mounted) => deepAll(m.el, "tr.reportes-row").map((r) => r.querySelectorAll("td")[1]?.textContent?.trim());
const text = (m: Mounted) => m.el.shadowRoot!.textContent!.replace(/\s+/g, " ");
const reportCalls = (m: Mounted) => m.calls.filter((c) => c.path.startsWith(REPORT)).map((c) => c.path);
const csvHref = (m: Mounted) => deep(m.el, "[data-testid=reportes-csv]").getAttribute("href");
const checkboxes = (m: Mounted) => deepAll(m.el, "tx-checkbox-group tx-checkbox");

describe("<tx-page-reportes>", () => {
  beforeEach(() => {
    window.history.replaceState(null, "", "/reportes");
  });
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("sets the title and loads every filed invoice with the list of cargues", async () => {
    const m = await mountPage();
    expect(document.title).toBe("Reportes — Tessia");
    expect(deep(m.el, "h2").textContent).toBe("Reportes");
    expect(m.callsTo("GET", "/api/cargues")).toHaveLength(1);
    expect(reportCalls(m)).toEqual([REPORT]);
    expect(facturas(m)).toEqual(["EVSB1", "FVSA2", "FVSA1"]);
    expect(text(m)).toContain("3 facturas radicadas");
    expect(checkboxes(m)).toHaveLength(2);
  });

  it("links the CSV of the current filters on the API base", async () => {
    const m = await mountPage();
    expect(csvHref(m)).toBe("http://api.test/api/reportes/radicadas?formato=csv");
    toggle(checkboxes(m)[0]!);
    await settle(m.el);
    expect(csvHref(m)).toBe("http://api.test/api/reportes/radicadas?cargue=A&formato=csv");
  });

  it("queries again with the checked cargues, without a limit in the CSV", async () => {
    const m = await mountPage();
    toggle(checkboxes(m)[0]!);
    await settle(m.el);
    toggle(checkboxes(m)[1]!);
    await settle(m.el);
    expect(reportCalls(m)).toEqual([REPORT, `${REPORT}?cargue=A`, `${REPORT}?cargue=A&cargue=B`]);
  });

  it("queries again with the date range", async () => {
    const m = await mountPage();
    typeInto(deep(m.el, "[data-testid=reportes-desde]"), "2026-09-11");
    await settle(m.el);
    typeInto(deep(m.el, "[data-testid=reportes-hasta]"), "2026-09-12");
    await settle(m.el);
    expect(reportCalls(m).slice(1)).toEqual([`${REPORT}?desde=2026-09-11`, `${REPORT}?desde=2026-09-11&hasta=2026-09-12`]);
    expect(csvHref(m)).toBe("http://api.test/api/reportes/radicadas?desde=2026-09-11&hasta=2026-09-12&formato=csv");
  });

  it("searches by invoice once the typing pauses", async () => {
    const m = await mountPage();
    typeInto(deep(m.el, "[data-testid=reportes-search]"), "fvs");
    await settle(m.el);
    expect(reportCalls(m)).toEqual([REPORT, `${REPORT}?q=fvs`]);
  });

  it("does not query for a range that ends before it starts", async () => {
    const m = await mountPage();
    typeInto(deep(m.el, "[data-testid=reportes-desde]"), "2026-09-12");
    await settle(m.el);
    typeInto(deep(m.el, "[data-testid=reportes-hasta]"), "2026-09-01");
    await settle(m.el);
    expect(reportCalls(m)).toEqual([REPORT, `${REPORT}?desde=2026-09-12`]);
    expect(deep(m.el, "[data-testid=range-error]").textContent).toContain("no puede ser posterior");
    expect(deep(m.el, "[data-testid=reportes-csv]").hasAttribute("href")).toBe(false);
  });

  it("clears every filter and loads the whole report again", async () => {
    const m = await mountPage();
    toggle(checkboxes(m)[0]!);
    await settle(m.el);
    typeInto(deep(m.el, "[data-testid=reportes-desde]"), "2026-09-11");
    await settle(m.el);
    deep(m.el, "[data-testid=reportes-clear]").click();
    await settle(m.el);
    expect(reportCalls(m).at(-1)).toBe(REPORT);
    expect(csvHref(m)).toBe("http://api.test/api/reportes/radicadas?formato=csv");
    expect(deepAll(m.el, "[data-testid=reportes-clear]")).toHaveLength(0);
  });

  it("says how many rows the truncated preview shows", async () => {
    const m = await mountPage({ [`GET ${REPORT}`]: { ...all, total: 1234, truncado: true } });
    expect(text(m)).toContain("Mostrando 3 de 1.234 facturas radicadas. Descarga el CSV para obtenerlas todas.");
  });

  it("shows the empty state", async () => {
    const m = await mountPage({ [`GET ${REPORT}`]: reporte([]) });
    expect(text(m)).toContain("Sin facturas radicadas con estos filtros");
    expect(deepAll(m.el, "tr.reportes-row")).toHaveLength(0);
  });

  it("shows the progress bar while the first answer is pending", async () => {
    const never = vi.fn(() => new Promise<Response>(() => {})); // routedApi would answer a promise as "{}"
    holder.api = createTessiaApi(createApiClient({ baseUrl: "http://api.test", fetch: never as unknown as typeof fetch }));
    const el = document.createElement("tx-page-reportes") as PageReportes;
    document.body.append(el);
    await el.updateComplete;
    expect(deepAll(el, "tx-progress")).toHaveLength(1);
    expect(deepAll(el, "tx-reportes-tabla")).toHaveLength(0);
  });

  it("shows the load error and retries from its button", async () => {
    let calls = 0;
    const m = await mountPage({ [`GET ${REPORT}`]: () => (++calls === 1 ? failure("sin conexión", 503) : all) });
    const alert = deep(m.el, "[data-testid=reportes-error]");
    expect(alert.getAttribute("heading")).toBe("No se pudo cargar el reporte");
    expect(alert.getAttribute("description")).toContain("sin conexión");
    deep(m.el, "[data-testid=reportes-error] tx-button").click();
    await settle(m.el);
    expect(deepAll(m.el, "[data-testid=reportes-error]")).toHaveLength(0);
    expect(facturas(m)).toHaveLength(3);
  });

  it("still reports when the list of cargues fails", async () => {
    const m = await mountPage({ "GET /api/cargues": () => failure("sin cargues", 500) });
    expect(facturas(m)).toHaveLength(3);
    expect(deep(m.el, "[data-testid=cargues-error]").getAttribute("heading")).toBe("No se pudo cargar la lista de cargues");
    expect(checkboxes(m)).toHaveLength(0);
  });

  it("keeps the answer of the latest query when an older one arrives late", async () => {
    // routedApi answers synchronously, so this one drives the fetch by hand.
    const pending: Array<(r: RadicadasReport) => void> = [];
    const json = (body: unknown) => new Response(JSON.stringify(body), { status: 200 });
    const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
      const url = new URL(String(input));
      if (url.pathname === "/api/cargues") return json([cargue("A"), cargue("B")]);
      if (!url.search) return json(all);
      return new Promise<Response>((resolve) => pending.push((r) => resolve(json(r))));
    });
    holder.api = createTessiaApi(createApiClient({ baseUrl: "http://api.test", fetch: fetchMock as unknown as typeof fetch }));
    const el = document.createElement("tx-page-reportes") as PageReportes;
    el.searchDelayMs = 0;
    document.body.append(el);
    await settle(el);
    const m = { el } as Mounted;
    toggle(checkboxes(m)[0]!); // ?cargue=A, slow
    await settle(el);
    toggle(checkboxes(m)[1]!); // ?cargue=A&cargue=B, slow
    await settle(el);
    pending[1]!(reporte([radicada({ factura: "NEW" })]));
    await settle(el);
    pending[0]!(reporte([radicada({ factura: "OLD" })]));
    await settle(el);
    expect(facturas(m)).toEqual(["NEW"]);
  });
});
