import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ROUTES } from "../../src/router/router";
import { routedApi, useApi } from "../support/ejecuciones-api";

vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
await import("../../src/pages/page-ejecuciones");

async function mountPage(history: unknown = null) {
  const routed = routedApi({ "GET /api/jobs/history": history });
  useApi(routed.api);
  const el = document.createElement("tx-page-ejecuciones");
  document.body.append(el);
  await vi.advanceTimersByTimeAsync(0);
  return { ...routed, el: el as HTMLElement & { updateComplete: Promise<boolean> } };
}

describe("<tx-page-ejecuciones>", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    document.title = "Tessia";
  });
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("is the element of the /ejecuciones route", () => {
    expect(ROUTES.find((r) => r.path === "/ejecuciones")!.tag).toBe("tx-page-ejecuciones");
    expect(customElements.get("tx-page-ejecuciones")).toBeDefined();
  });

  it("sets the title and renders the heading, the explanation and the process history", async () => {
    const { el } = await mountPage();
    expect(document.title).toBe("Ejecuciones — Tessia");
    const root = el.shadowRoot!;
    expect(root.querySelector("h2")!.textContent).toBe("Ejecuciones");
    expect(root.querySelector("p")!.textContent).toContain("Historial de procesos ya terminados. El progreso de un proceso en curso se ve en el Dashboard.");
    const historial = root.querySelector("tx-historial")!;
    expect(historial.shadowRoot!.textContent).toContain("Historial de procesos");
    expect(historial.shadowRoot!.querySelector("[data-testid=history-empty]")!.textContent).toBe("sin historial todavía");
  });

  it("restores the previous title when it leaves", async () => {
    const { el } = await mountPage();
    el.remove();
    expect(document.title).toBe("Tessia");
  });

  it("does not start jobs or touch other endpoints", async () => {
    const { calls } = await mountPage([]);
    expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual(["GET /api/jobs/history"]);
  });
});
