import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/pages/page-dashboard";
import { CONSULTA, consultaCalls, fakeConsulta } from "../support/dashboard-consulta";
import { mountEl, q, qa, settle } from "../support/dashboard-dom";
import { routedApi } from "../support/dashboard-routed-api";

const holder = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("../../src/api", () => ({ api: new Proxy({}, { get: (_t, k) => holder.api[k as string] }) }));

const IDLE_JOB = { id: "", tipo: "", corriendo: false, cancelable: false, iniciado_en: "0001-01-01T00:00:00Z", salida: "" };

async function mountPage() {
  const routed = routedApi({
    "GET /api/health": [{ nombre: "PostgreSQL", disponible: true, latencia_ms: 2 }],
    "GET /api/nota-tecnica": { exists: false },
    "GET /api/cargues": [],
    "GET /api/jobs/current": IDLE_JOB,
    [CONSULTA]: fakeConsulta([]),
    "GET /api/resumen": { desde: "", hasta: "", kpis: {}, cargues: [] },
  });
  holder.api = routed.api as unknown as Record<string, unknown>;
  const el = await mountEl("tx-page-dashboard");
  await settle(el);
  return { el, ...routed };
}

const tabs = (el: Element) => q(el, "tx-tabs") as HTMLElement & { updateComplete: Promise<boolean> };
const tabButtons = (el: Element) => qa<HTMLButtonElement>(tabs(el), "[role=tab]");

beforeEach(() => {
  document.title = "antes";
});
afterEach(() => {
  document.body.innerHTML = "";
});

describe("<tx-page-dashboard>", () => {
  it("opens on Operar with the health cards, new cargue, process panel and an open console", async () => {
    const { el } = await mountPage();
    expect(tabButtons(el).map((b) => b.textContent!.trim())).toEqual(["Operar", "Analizar"]);
    expect(tabButtons(el)[0]!.getAttribute("aria-selected")).toBe("true");
    for (const tag of ["tx-health-cards", "tx-nuevo-cargue", "tx-proceso-panel", "tx-consola-proceso"]) expect(q(el, tag), tag).not.toBeNull();
    expect(q(el, "tx-consola-proceso")!.hasAttribute("default-open")).toBe(true);
    expect(q(el, "rips-dashboard")).toBeNull();
  });

  it("ignores tx-change events bubbling from controls inside a tab (picking another cargue keeps Operar)", async () => {
    const { el } = await mountPage();
    const panel = q(el, "tx-proceso-panel")!;
    // What the cargue select-menu of the process panel emits when another cargue is chosen.
    panel.dispatchEvent(new CustomEvent("tx-change", { detail: { value: "20260205" }, bubbles: true, composed: true }));
    await settle(el);
    await settle(tabs(el));
    expect(tabButtons(el)[0]!.getAttribute("aria-selected")).toBe("true");
    expect(q(el, "tx-proceso-panel")).not.toBeNull();
    expect(q(el, "tx-health-cards")).not.toBeNull();
  });

  it("Analizar mounts the RIPS dashboard and unmounts the operate content (its polls stop)", async () => {
    const { el, calls } = await mountPage();
    tabButtons(el)[1]!.click();
    await settle(el);
    await settle(tabs(el));
    expect(q(el, "rips-dashboard")).not.toBeNull();
    expect(q(el, "tx-health-cards")).toBeNull();
    expect(q(el, "tx-proceso-panel")).toBeNull();
    expect(consultaCalls(calls).length).toBeGreaterThanOrEqual(2);
    tabButtons(el)[0]!.click();
    await settle(el);
    await settle(tabs(el));
    expect(q(el, "rips-dashboard")).toBeNull();
    expect(q(el, "tx-health-cards")).not.toBeNull();
  });

  it("sets the document title while it is on screen and restores it", async () => {
    const { el } = await mountPage();
    expect(document.title).toBe("Dashboard — Tessia");
    el.remove();
    expect(document.title).toBe("antes");
  });

  it("is auto-registered as the page of the dashboard route", () => {
    expect(customElements.get("tx-page-dashboard")).toBeDefined();
  });
});
