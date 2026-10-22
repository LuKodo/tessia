import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TxHistorial } from "../../src/components/historial";
import type { TxAccordion } from "../../src/components/tx-accordion";
import type { JobHistoryEntry } from "../../src/types/api";
import { routedApi, useApi } from "../support/ejecuciones-api";

vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
await import("../../src/components/historial");

const entries: JobHistoryEntry[] = [
  {
    id: "h2",
    tipo: "proceso run",
    iniciado_en: "2026-10-02T10:00:00Z",
    terminado_en: "2026-10-02T10:03:12Z",
    error: "CUV falló",
    salida_final: "cola de salida",
    estadisticas: { esperadas: 10, procesadas: 10, reemplazadas: 0, gestionadas: 10, terminadas: 6, con_errores: 1, excluidas: 2, revision_manual: 1, evaluadas: 0, limite: 0 },
  },
  { id: "h1", tipo: "convenio descargar", iniciado_en: "2026-10-01T10:00:00Z", terminado_en: "2026-10-01T10:00:42Z" },
];

async function mountHistory(body: unknown) {
  const routed = routedApi({ "GET /api/jobs/history": body });
  useApi(routed.api);
  const el = document.createElement("tx-historial") as TxHistorial;
  document.body.append(el);
  await vi.advanceTimersByTimeAsync(0);
  await el.updateComplete;
  return { ...routed, el };
}

const root = (el: TxHistorial) => el.shadowRoot!;
const rows = (el: TxHistorial) => [...root(el).querySelectorAll("[data-testid=history-entry]")];
const triggers = (el: TxHistorial) => [...root(el).querySelector<TxAccordion>("tx-accordion")!.shadowRoot!.querySelectorAll<HTMLButtonElement>("button[aria-expanded]")];

describe("<tx-historial>", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("has the card title", async () => {
    const { el } = await mountHistory(null);
    expect(root(el).querySelector("h2")!.textContent).toBe("Historial de procesos");
  });

  it("shows the empty state when the history is null (nothing finished yet)", async () => {
    const { el } = await mountHistory(null);
    expect(root(el).querySelector("[data-testid=history-empty]")!.textContent).toBe("sin historial todavía");
    expect(root(el).querySelector("tx-accordion")).toBeNull();
  });

  it("shows the empty state for an empty list", async () => {
    const { el } = await mountHistory([]);
    expect(root(el).querySelector("[data-testid=history-empty]")).not.toBeNull();
  });

  it("lists finished jobs with outcome, duration and stat chips", async () => {
    const { el } = await mountHistory(entries);
    expect(rows(el)).toHaveLength(2);
    const first = rows(el)[0]!.textContent!.replace(/\s+/g, " ");
    expect(first).toContain("proceso run");
    expect(first).toContain("Terminado con error");
    expect(first).toContain("Duración: 3 min 12 s");
    expect(first).toContain("Esperadas: 10");
    expect(first).toContain("Terminadas: 6");
    const second = rows(el)[1]!.textContent!.replace(/\s+/g, " ");
    expect(second).toContain("Terminado");
    expect(second).not.toContain("Terminado con error");
    expect(second).toContain("Duración: 42 s");
    expect(second).not.toContain("Esperadas");
  });

  it("expands several entries at once and shows the error and final output", async () => {
    const { el } = await mountHistory(entries);
    expect(triggers(el)).toHaveLength(2);
    expect(root(el).textContent).not.toContain("cola de salida");
    triggers(el)[0]!.click();
    triggers(el)[1]!.click();
    await el.updateComplete;
    expect(triggers(el).map((t) => t.getAttribute("aria-expanded"))).toEqual(["true", "true"]);
    expect(root(el).textContent).toContain("CUV falló");
    expect(root(el).textContent).toContain("cola de salida");
    expect(root(el).textContent).toContain("(sin salida)");
  });

  it("opens each entry on its own when old entries share an id (the job counter restarted)", async () => {
    // job-1 of an earlier server instance and job-1 of this one: same id, different start.
    const repeated: JobHistoryEntry[] = [
      { id: "job-1", tipo: "proceso run", iniciado_en: "2026-10-14T10:00:00Z", terminado_en: "2026-10-14T10:01:00Z", salida_final: "salida nueva" },
      { id: "job-1", tipo: "convenio descargar", iniciado_en: "2026-10-01T10:00:00Z", terminado_en: "2026-10-01T10:00:42Z", salida_final: "salida vieja" },
    ];
    const { el } = await mountHistory(repeated);
    triggers(el)[1]!.click();
    await el.updateComplete;
    expect(triggers(el).map((t) => t.getAttribute("aria-expanded"))).toEqual(["false", "true"]);
    expect(root(el).textContent).toContain("salida vieja");
    expect(root(el).textContent).not.toContain("salida nueva");
    triggers(el)[0]!.click();
    await el.updateComplete;
    expect(triggers(el).map((t) => t.getAttribute("aria-expanded"))).toEqual(["true", "true"]);
    expect(root(el).textContent).toContain("salida nueva");
  });

  it("keeps an entry open across polls and collapses it again on click", async () => {
    const { el } = await mountHistory(entries);
    triggers(el)[0]!.click();
    await el.updateComplete;
    await vi.advanceTimersByTimeAsync(5000);
    await el.updateComplete;
    expect(triggers(el)[0]!.getAttribute("aria-expanded")).toBe("true");
    triggers(el)[0]!.click();
    await el.updateComplete;
    expect(root(el).textContent).not.toContain("cola de salida");
  });

  it("colors the outcome badge by result", async () => {
    const { el } = await mountHistory(entries);
    const badges = [...root(el).querySelectorAll("[data-testid=history-entry] tx-badge")].filter((b) => b.textContent!.trim().startsWith("Terminado"));
    expect(badges.map((b) => b.getAttribute("color"))).toEqual(["error", "success"]);
  });

  it("polls the history every 5 seconds", async () => {
    const { callsTo } = await mountHistory([]);
    expect(callsTo("GET", "/api/jobs/history")).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(callsTo("GET", "/api/jobs/history")).toHaveLength(2);
  });
});
