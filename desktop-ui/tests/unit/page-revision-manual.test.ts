import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/pages/page-revision-manual";
import type { PageRevisionManual } from "../../src/pages/page-revision-manual";
import type { Cargue, JobSnapshot, ManualReviewReport, ReportItem } from "../../src/types/api";
import { dialogs } from "../../src/utils/dialogs";
import { deep, deepAll, setChecked, settle, toggle, typeInto } from "../support/revision-dom";
import { API_BASE, routedApi } from "../support/revision-routed-api";

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("../../src/api", () => ({
  get api() {
    return holder.api;
  },
}));

const cargue = (id: string, over: Partial<Cargue> = {}): Cargue => ({
  id,
  nombre: `Convenio ${id}`,
  facturas: 10,
  excluidas: 0,
  revision_manual: 0,
  modificado_en: "2026-09-16T10:00:00Z",
  estado: "COMPLETADO",
  ...over,
});

const item = (over: Partial<ReportItem> = {}): ReportItem => ({
  factura: "EVS1",
  fase: "fev_valor",
  categoria: "revision_manual",
  codigo: "VALOR_DIFERENTE",
  motivo: "VALOR_DIFERENTE XML=1 convenio=2",
  actualizado_en: "",
  ...over,
});

const report = (id: string, items: ReportItem[], over: Partial<ManualReviewReport> = {}): ManualReviewReport => ({
  cargue: id,
  generado_en: "2026-10-01T10:00:00Z",
  incompleto: false,
  conteos: {
    revision_manual: items.filter((i) => i.categoria === "revision_manual").length,
    excluidas: items.filter((i) => i.categoria === "excluida").length,
    fallas: 0,
  },
  items,
  ...over,
});

const reportA = report("A", [
  item({ factura: "A1" }),
  item({ factura: "A2", fase: "derechos", codigo: "", motivo: "sin derechos" }),
  item({ factura: "A3", categoria: "excluida", fase: "fev_ajuste", codigo: "", motivo: "monto" }),
]);
const reportB = report("B", [item({ factura: "B1" })]);

const finished = (id: string): JobSnapshot => ({
  id,
  tipo: "reintentar factura",
  corriendo: false,
  cancelable: false,
  iniciado_en: "2026-10-01T10:05:00Z",
  terminado_en: "2026-10-01T10:05:09Z",
  salida: `salida ${id}`,
});

const failure = (message: string, status = 500) => new Response(JSON.stringify({ error: message }), { status });

let retryCount = 0;
async function mountLoaded(routes: Parameters<typeof routedApi>[0] = {}) {
  retryCount = 0;
  const started = () => ({ ...finished(`r${++retryCount}`), corriendo: true, terminado_en: "0001-01-01T00:00:00Z", salida: "" });
  const routed = routedApi({
    "GET /api/cargues": [cargue("A"), cargue("B")],
    "GET /api/cargues/A/informe": reportA,
    "GET /api/cargues/B/informe": reportB,
    "POST /api/cargues/A/reintentar": started,
    "POST /api/cargues/B/reintentar": started,
    "GET /api/jobs/current": () => finished(`r${retryCount}`),
    "GET /api/jobs/history": null,
    ...routes,
  });
  holder.api = routed.api;
  const el = document.createElement("tx-page-revision-manual") as PageRevisionManual;
  document.body.append(el);
  await settle(el);
  return { ...routed, el };
}

type Mounted = Awaited<ReturnType<typeof mountLoaded>>;
const facturas = (m: Mounted) => deepAll(m.el, "tr.revision-row").map((r) => r.querySelectorAll("td")[2]?.textContent?.trim());
const text = (m: Mounted) => m.el.shadowRoot!.textContent!.replace(/\s+/g, " ");
const pending = () => dialogs.confirm;
const answer = async (m: Mounted, accepted: boolean) => {
  dialogs.answerConfirm(accepted);
  await settle(m.el);
};
const click = async (m: Mounted, selector: string) => {
  deep(m.el, selector).click();
  await settle(m.el);
};

describe("<tx-page-revision-manual>", () => {
  beforeEach(() => {
    window.history.replaceState(null, "", "/revision-manual");
  });
  afterEach(() => {
    dialogs.answerConfirm(false);
    document.body.innerHTML = "";
  });

  it("sets the title and loads the report of every cargue into one table", async () => {
    const m = await mountLoaded();
    expect(document.title).toBe("Revisión manual — Tessia");
    expect(deep(m.el, "h2").textContent).toBe("Revisión manual");
    expect(m.callsTo("GET", "/api/cargues")).toHaveLength(1);
    expect(m.callsTo("GET", "/api/cargues/A/informe")).toHaveLength(1);
    expect(m.callsTo("GET", "/api/cargues/B/informe")).toHaveLength(1);
    expect(facturas(m)).toEqual(["A1", "A2", "B1"]);
    expect(text(m)).toContain("Revisión manual: 3");
  });

  it("shows the progress bar while loading", async () => {
    const routed = routedApi({ "GET /api/cargues": () => new Promise(() => {}) });
    holder.api = routed.api;
    const el = document.createElement("tx-page-revision-manual") as PageRevisionManual;
    document.body.append(el);
    await el.updateComplete;
    expect(deepAll(el, "tx-progress")).toHaveLength(1);
    expect(deepAll(el, "tx-revision-tabla")).toHaveLength(0);
  });

  it("shows the load error and reloads from its Reintentar button", async () => {
    let calls = 0;
    const m = await mountLoaded({ "GET /api/cargues": () => (++calls === 1 ? failure("sin conexión", 503) : [cargue("A"), cargue("B")]) });
    const alert = deep(m.el, "tx-alert");
    expect(alert.getAttribute("heading")).toBe("No se pudo cargar el informe");
    expect(alert.getAttribute("description")).toContain("sin conexión");
    await click(m, "tx-alert tx-button");
    expect(deepAll(m.el, "tx-alert")).toHaveLength(0);
    expect(facturas(m)).toEqual(["A1", "A2", "B1"]);
  });

  it("keeps the other cargues visible when one report fails", async () => {
    const m = await mountLoaded({ "GET /api/cargues/A/informe": failure("informe no disponible", 503) });
    expect(facturas(m)).toEqual(["B1"]);
    const alert = deep(m.el, "[data-testid=cargue-error]");
    expect(alert.getAttribute("heading")).toContain("A");
    expect(alert.getAttribute("description")).toContain("informe no disponible");
  });

  it("preselects the cargue from the ?cargue= query and links its CSV", async () => {
    window.history.replaceState(null, "", "/revision-manual?cargue=B");
    const m = await mountLoaded();
    expect(facturas(m)).toEqual(["B1"]);
    const csv = deep(m.el, "[data-testid=informe-csv]");
    expect(csv.getAttribute("href")).toBe(`${API_BASE}/api/cargues/B/informe?formato=csv`);
    expect(csv.getAttribute("download")).toBe("informe-revision-manual-B.csv");
  });

  it("ignores a ?cargue= that is not in the list", async () => {
    window.history.replaceState(null, "", "/revision-manual?cargue=ZZZ");
    const m = await mountLoaded();
    expect(facturas(m)).toEqual(["A1", "A2", "B1"]);
    expect(deep(m.el, "[data-testid=informe-csv]").hasAttribute("href")).toBe(false);
  });

  it("filters by search text and by the include-others switch", async () => {
    const m = await mountLoaded();
    typeInto(deep(m.el, "[data-testid=revision-search]"), "sin derechos");
    await settle(m.el);
    expect(facturas(m)).toEqual(["A2"]);
    typeInto(deep(m.el, "[data-testid=revision-search]"), "");
    toggle(deep(m.el, "[data-testid=revision-others]"));
    await settle(m.el);
    expect(facturas(m)).toEqual(["A1", "A2", "A3", "B1"]);
    expect(text(m)).toContain("Excluida: 1");
  });

  it("filters by cargue from the select and then links that CSV", async () => {
    const m = await mountLoaded();
    const select = deep(m.el, "[data-testid=revision-cargue]");
    const native = select.shadowRoot!.querySelector("select")!;
    native.value = "A";
    native.dispatchEvent(new Event("change", { bubbles: true }));
    await settle(m.el);
    expect(facturas(m)).toEqual(["A1", "A2"]);
    expect(deep(m.el, "[data-testid=informe-csv]").getAttribute("href")).toBe(`${API_BASE}/api/cargues/A/informe?formato=csv`);
  });

  it("warns when a report is incomplete and offers no retry for its rows", async () => {
    const m = await mountLoaded({ "GET /api/cargues/A/informe": { ...reportA, incompleto: true } });
    expect(deep(m.el, "tx-alert[color=warning]").getAttribute("heading")).toContain("Base de datos no disponible");
    expect(deepAll(m.el, "[data-testid=retry-invoice]")).toHaveLength(1);
  });

  it("only offers Reintentar for value-difference rows", async () => {
    const m = await mountLoaded();
    expect(deepAll(m.el, "[data-testid=retry-invoice]")).toHaveLength(2);
    expect(deepAll(m.el, "[data-testid=row-select]")).toHaveLength(2);
  });

  it("retries one invoice after confirmation, follows the job and reloads the reports", async () => {
    const m = await mountLoaded();
    await click(m, "[data-testid=retry-invoice]");
    expect(pending()).toMatchObject({ title: "Reintentar factura", confirmText: "Sí, reintentar" });
    expect(pending()?.text).toContain("A1");
    await answer(m, true);

    const retries = m.callsTo("POST", "/api/cargues/A/reintentar");
    expect(retries).toHaveLength(1);
    expect(retries[0]?.body).toEqual({ factura: "A1" });
    expect(m.callsTo("GET", "/api/cargues/A/informe")).toHaveLength(2);
    expect(m.callsTo("GET", "/api/cargues/B/informe")).toHaveLength(2);
    expect(deep(m.el, "tx-alert[color=info]").getAttribute("heading")).toBe("Informe actualizado.");
    expect(deep(m.el, "pre.retry-output").textContent).toContain("salida r1");
  });

  it("does nothing when the operator declines a single retry", async () => {
    const m = await mountLoaded();
    await click(m, "[data-testid=retry-invoice]");
    await answer(m, false);
    expect(m.callsTo("POST", "/api/cargues/A/reintentar")).toHaveLength(0);
  });

  it("shows the server message when a retry is rejected", async () => {
    const m = await mountLoaded({ "POST /api/cargues/A/reintentar": failure("ya hay un proceso en curso") });
    await click(m, "[data-testid=retry-invoice]");
    await answer(m, true);
    expect(deep(m.el, "tx-alert[color=error][heading*='ya hay']").getAttribute("heading")).toContain("ya hay un proceso en curso");
  });

  it("selects every visible retryable row and retries them in order after one confirmation", async () => {
    const m = await mountLoaded();
    const button = () => deep(m.el, "[data-testid=retry-selected]");
    expect(button().textContent).toContain("(0)");
    expect(button().hasAttribute("disabled")).toBe(true);

    toggle(deep(m.el, "[data-testid=select-all]"));
    await settle(m.el);
    expect(button().textContent).toContain("(2)");
    await click(m, "[data-testid=retry-selected]");
    expect(pending()?.text).toContain("2 facturas");
    expect(pending()).toMatchObject({ title: "Reintentar facturas seleccionadas" });
    await answer(m, true);

    expect(m.callsTo("POST", "/api/cargues/A/reintentar").map((c) => c.body)).toEqual([{ factura: "A1" }]);
    expect(m.callsTo("POST", "/api/cargues/B/reintentar").map((c) => c.body)).toEqual([{ factura: "B1" }]);
    expect(m.calls.filter((c) => c.method === "POST").map((c) => c.path)).toEqual(["/api/cargues/A/reintentar", "/api/cargues/B/reintentar"]);
    // One reload for the whole batch.
    expect(m.callsTo("GET", "/api/cargues")).toHaveLength(2);
    expect(deep(m.el, "tx-alert[color=info]").getAttribute("heading")).toBe("Informe actualizado.");
    expect(pending()).toBeNull();
  });

  it("selects single rows and retries only those", async () => {
    const m = await mountLoaded();
    setChecked(deepAll(m.el, "[data-testid=row-select]")[1]!, true);
    await settle(m.el);
    expect(deep(m.el, "[data-testid=retry-selected]").textContent).toContain("(1)");
    await click(m, "[data-testid=retry-selected]");
    expect(pending()?.text).toContain("1 factura,");
    await answer(m, true);
    expect(m.callsTo("POST", "/api/cargues/A/reintentar")).toHaveLength(0);
    expect(m.callsTo("POST", "/api/cargues/B/reintentar")).toHaveLength(1);
  });

  it("continues past a failed row and reports it", async () => {
    const m = await mountLoaded({ "POST /api/cargues/A/reintentar": failure("ya hay un proceso en curso") });
    toggle(deep(m.el, "[data-testid=select-all]"));
    await settle(m.el);
    await click(m, "[data-testid=retry-selected]");
    await answer(m, true);
    expect(m.callsTo("POST", "/api/cargues/B/reintentar")).toHaveLength(1);
    expect(deep(m.el, "tx-alert[color=error][heading*='A / A1']").getAttribute("heading")).toContain("A / A1: ya hay un proceso en curso");
  });

  it("does not retry the selection when the operator declines", async () => {
    const m = await mountLoaded();
    toggle(deep(m.el, "[data-testid=select-all]"));
    await settle(m.el);
    await click(m, "[data-testid=retry-selected]");
    await answer(m, false);
    expect(m.calls.filter((c) => c.method === "POST")).toHaveLength(0);
    expect(deep(m.el, "[data-testid=retry-selected]").textContent).toContain("(2)");
  });

  it("drops from the selection the rows that are no longer retryable after a reload", async () => {
    const m = await mountLoaded();
    toggle(deep(m.el, "[data-testid=select-all]"));
    await settle(m.el);
    expect(deep(m.el, "[data-testid=retry-selected]").textContent).toContain("(2)");
    // The next report no longer lists A1.
    holder.api = routedApi({
      "GET /api/cargues": [cargue("A"), cargue("B")],
      "GET /api/cargues/A/informe": report("A", [reportA.items[1]!]),
      "GET /api/cargues/B/informe": reportB,
    }).api;
    await m.el.reload();
    await settle(m.el);
    expect(deep(m.el, "[data-testid=retry-selected]").textContent).toContain("(1)");
  });
});
