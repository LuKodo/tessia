import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TxProcesoPanel } from "../../src/components/proceso-panel";
import type { TxSelect } from "../../src/components/tx-select";
import type { TxSelectMenu } from "../../src/components/tx-select-menu";
import type { Cargue } from "../../src/types/api";
import { dialogs } from "../../src/utils/dialogs";
import { fakeApi, routedApi, useApi } from "../support/ejecuciones-api";

vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
await import("../../src/components/proceso-panel");

const cargue = (over: Partial<Cargue>): Cargue => ({
  id: "A",
  nombre: "Convenio A",
  fecha_inicio: "2026-09-01",
  fecha_fin: "2026-09-15",
  facturas: 10,
  excluidas: 2,
  revision_manual: 1,
  modificado_en: "2026-09-16T10:00:00Z",
  estado: "COMPLETADO",
  ...over,
});

const started = { id: "r1", tipo: "proceso run", corriendo: true, cancelable: true, iniciado_en: "2026-10-01T10:00:00Z", salida: "" };

async function flush(el?: TxProcesoPanel) {
  await vi.advanceTimersByTimeAsync(0);
  await el?.updateComplete;
}

async function mountPanel(cargues: Cargue[], routes: Parameters<typeof routedApi>[0] = {}) {
  const routed = routedApi({ "GET /api/cargues": cargues, "POST /api/run": started, ...routes });
  useApi(routed.api);
  const el = document.createElement("tx-proceso-panel") as TxProcesoPanel;
  document.body.append(el);
  await flush(el);
  await flush(el);
  return { ...routed, el };
}

const q = <T extends HTMLElement>(el: TxProcesoPanel, sel: string) => el.shadowRoot!.querySelector<T>(sel)!;
const text = (el: TxProcesoPanel) => el.shadowRoot!.textContent!.replace(/\s+/g, " ");
const status = (el: TxProcesoPanel) => q(el, "[data-testid=run-status]");
const click = (e: Element) => e.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));

async function choosePhase(el: TxProcesoPanel, testid: string, value: string) {
  const select = q<TxSelect>(el, `[data-testid=${testid}]`);
  const native = select.shadowRoot!.querySelector("select")!;
  native.value = value;
  native.dispatchEvent(new Event("change", { bubbles: true }));
  await el.updateComplete;
}

async function setLimit(el: TxProcesoPanel, value: string) {
  const input = q(el, "[data-testid=limit]").shadowRoot!.querySelector("input")!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  input.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  await el.updateComplete;
}

describe("<tx-proceso-panel>", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
    dialogs.answerConfirm(false);
  });

  it("selects the first cargue and summarizes it", async () => {
    const { el } = await mountPanel([cargue({}), cargue({ id: "B", nombre: "Convenio B" })]);
    expect(q<TxSelectMenu>(el, "[data-testid=cargue-select]").value).toBe("A");
    expect(text(el)).toContain("COMPLETADO");
    expect(text(el)).toContain("2026-09-01 → 2026-09-15");
    expect(text(el)).toContain("10 preparada(s) · 2 excluida(s) · 1 en revisión manual");
  });

  it("shows the selected cargue name and a status badge", async () => {
    const { el } = await mountPanel([cargue({ estado: "COMPLETADO" })]);
    const menu = q<TxSelectMenu>(el, "[data-testid=cargue-select]");
    await menu.updateComplete;
    expect(menu.shadowRoot!.querySelector("input")!.value).toBe("Convenio A");
    const badge = q(el, "tx-badge");
    expect(badge.textContent!.trim()).toBe("COMPLETADO");
    expect(badge.getAttribute("color")).toBe("success");
  });

  it("falls back to the id when a cargue has no name", async () => {
    const { el } = await mountPanel([cargue({ nombre: "" })]);
    expect(q<TxSelectMenu>(el, "[data-testid=cargue-select]").items).toEqual([{ value: "A", label: "A" }]);
  });

  it("switches to another cargue from the menu and keeps it across polls", async () => {
    const { el } = await mountPanel([cargue({}), cargue({ id: "B", nombre: "Convenio B", estado: "SIN_PREPARAR" })]);
    const menu = q<TxSelectMenu>(el, "[data-testid=cargue-select]");
    menu.shadowRoot!.querySelector("input")!.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));
    await menu.updateComplete;
    menu.shadowRoot!.querySelectorAll<HTMLElement>('[role="option"]')[1]!.click();
    await el.updateComplete;
    expect(text(el)).toContain("SIN_PREPARAR");
    await vi.advanceTimersByTimeAsync(5000);
    await el.updateComplete;
    expect(menu.value).toBe("B");
  });

  it("falls back to the first cargue when the selected one disappears", async () => {
    let list = [cargue({}), cargue({ id: "B", nombre: "Convenio B" })];
    const { el } = await mountPanel([], { "GET /api/cargues": () => list });
    expect(q<TxSelectMenu>(el, "[data-testid=cargue-select]").value).toBe("A");
    list = [cargue({ id: "C", nombre: "Convenio C" })];
    await vi.advanceTimersByTimeAsync(5000);
    await el.updateComplete;
    expect(q<TxSelectMenu>(el, "[data-testid=cargue-select]").value).toBe("C");
  });

  it("maps the (completo) option back to an empty phase", async () => {
    const { el, callsTo } = await mountPanel([cargue({})]);
    await choosePhase(el, "from-phase", "derechos");
    await choosePhase(el, "from-phase", "completo");
    click(q(el, "[data-testid=run-section]"));
    expect(dialogs.confirm?.text).toContain("el proceso COMPLETO");
    dialogs.answerConfirm(true);
    await flush(el);
    expect(callsTo("POST", "/api/run")[0]?.body).toEqual({ Cargue: "A", From: "", To: "", Limit: 0 });
  });

  it("offers the (completo) option and the pipeline phases in order", async () => {
    const { el } = await mountPanel([cargue({})]);
    const labels = [...q(el, "[data-testid=from-phase]").shadowRoot!.querySelectorAll("option")].map((o) => o.textContent!.trim());
    expect(labels).toEqual(["(completo)", "Convenio", "TheFactory", "FEV", "Conciliar", "Derechos", "RIPS", "CUV", "Soporte", "Radicación"]);
  });

  it("polls the cargue list every 5 seconds", async () => {
    const { callsTo } = await mountPanel([cargue({})]);
    expect(callsTo("GET", "/api/cargues")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(callsTo("GET", "/api/cargues")).toHaveLength(2);
  });

  it("explains an unprepared cargue", async () => {
    const { el } = await mountPanel([cargue({ estado: "SIN_PREPARAR" })]);
    expect(text(el)).toContain("Pendiente de preparación; iniciar el proceso prepara el CSV primero");
  });

  it("asks to select a cargue when there is none", async () => {
    const { el, calls } = await mountPanel([]);
    expect(q<TxSelectMenu>(el, "[data-testid=cargue-select]").placeholder).toBe("sin cargues");
    click(q(el, "[data-testid=run-full]"));
    await el.updateComplete;
    expect(status(el).textContent).toBe("selecciona un cargue primero");
    expect(status(el).classList.contains("text-error")).toBe(true);
    expect(calls.filter((c) => c.method === "POST")).toHaveLength(0);
  });

  it("shows a loading placeholder until the list arrives", async () => {
    useApi(fakeApi({ cargues: () => new Promise(() => {}) }));
    const el = document.createElement("tx-proceso-panel") as TxProcesoPanel;
    document.body.append(el);
    await flush(el);
    expect(q<TxSelectMenu>(el, "[data-testid=cargue-select]").placeholder).toBe("cargando…");
  });

  it("rejects a non-integer limit before confirming", async () => {
    const { el } = await mountPanel([cargue({})]);
    await setLimit(el, "1.5");
    click(q(el, "[data-testid=run-full]"));
    await el.updateComplete;
    expect(status(el).textContent).toBe("El límite debe ser un entero mayor o igual a 0.");
    expect(dialogs.confirm).toBeNull();
  });

  it("rejects a negative limit", async () => {
    const { el } = await mountPanel([cargue({})]);
    await setLimit(el, "-3");
    click(q(el, "[data-testid=run-full]"));
    await el.updateComplete;
    expect(status(el).textContent).toBe("El límite debe ser un entero mayor o igual a 0.");
  });

  it("runs the full process after confirmation with the capitalized keys", async () => {
    const { el, callsTo } = await mountPanel([cargue({})]);
    click(q(el, "[data-testid=run-full]"));
    expect(dialogs.confirm?.text).toBe('¿Ejecutar el proceso COMPLETO sobre "A"? Esto escribe de verdad (Medicar, SISPRO, Postgres, Redis, disco).');
    dialogs.answerConfirm(true);
    await flush(el);
    const sent = callsTo("POST", "/api/run");
    expect(sent).toHaveLength(1);
    expect(sent[0]?.body).toEqual({ Cargue: "A", From: "", To: "", Limit: 0 });
    expect(status(el).textContent).toBe("en curso — sigue el progreso justo abajo");
    expect(status(el).classList.contains("text-success")).toBe(true);
  });

  it("runs a section with phases and limit, and mentions the preparation of an unprepared cargue", async () => {
    const { el, callsTo } = await mountPanel([cargue({ estado: "SIN_PREPARAR" })]);
    await choosePhase(el, "from-phase", "derechos");
    await choosePhase(el, "to-phase", "cuv");
    await setLimit(el, "5");
    click(q(el, "[data-testid=run-section]"));
    expect(dialogs.confirm?.text).toContain('¿Ejecutar las fases derechos → cuv sobre "A"? Primero se preparará el convenio desde su CSV.');
    dialogs.answerConfirm(true);
    await flush(el);
    expect(callsTo("POST", "/api/run")[0]?.body).toEqual({ Cargue: "A", From: "derechos", To: "cuv", Limit: 5 });
  });

  it("sends Reprepare only when the checkbox is ticked", async () => {
    const { el, callsTo } = await mountPanel([cargue({})]);
    const box = q<HTMLElement & { checked: boolean }>(el, "[data-testid=reprepare]");
    box.checked = true;
    box.dispatchEvent(new CustomEvent("tx-change", { detail: { value: "", checked: true }, bubbles: true, composed: true }));
    await el.updateComplete;
    click(q(el, "[data-testid=run-full]"));
    dialogs.answerConfirm(true);
    await flush(el);
    expect(callsTo("POST", "/api/run")[0]?.body).toEqual({ Cargue: "A", From: "", To: "", Limit: 0, Reprepare: true });
  });

  it("does not run when the operator declines", async () => {
    const { el, callsTo } = await mountPanel([cargue({})]);
    click(q(el, "[data-testid=run-full]"));
    dialogs.answerConfirm(false);
    await flush(el);
    expect(callsTo("POST", "/api/run")).toHaveLength(0);
  });

  it("shows the job conflict (HTTP 500 {error}) as the run status", async () => {
    const { el } = await mountPanel([cargue({})], {
      "POST /api/run": new Response(JSON.stringify({ error: "ya hay un proceso en curso" }), { status: 500 }),
    });
    click(q(el, "[data-testid=run-full]"));
    dialogs.answerConfirm(true);
    await flush(el);
    expect(status(el).textContent).toBe("ya hay un proceso en curso");
    expect(status(el).classList.contains("text-error")).toBe(true);
  });

  it("links the manual review report of the selected cargue to its page", async () => {
    const { el, calls } = await mountPanel([cargue({ id: "A b" })]);
    const link = q(el, "[data-testid=open-informe]");
    expect(link.getAttribute("href")).toBe("/revision-manual?cargue=A%20b");
    expect(link.textContent!.trim()).toBe("Informe de revisión manual");
    // The report is not fetched by the panel: the page loads it.
    expect(calls.filter((c) => c.path.includes("/informe"))).toHaveLength(0);
  });

  it("shows no summary nor link without a cargue", async () => {
    const { el } = await mountPanel([]);
    expect(el.shadowRoot!.querySelector("[data-testid=open-informe]")).toBeNull();
  });
});
