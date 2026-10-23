import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/nota-tecnica";
import type { TxNotaTecnica } from "../../src/components/nota-tecnica";
import { apiSlot, routedApi, settle } from "../support/parametros-routed-api";

vi.mock("../../src/api", async () => ({ api: (await import("../support/parametros-routed-api")).forwardingApi }));

type Routes = Parameters<typeof routedApi>[0];

async function mountCard(routes: Routes) {
  const routed = routedApi(routes);
  apiSlot.api = routed.api;
  const el = document.createElement("tx-nota-tecnica") as TxNotaTecnica;
  document.body.append(el);
  await settle(el);
  return { ...routed, el };
}
type Card = Awaited<ReturnType<typeof mountCard>>["el"];

const q = <T extends Element>(el: Card, selector: string) => el.shadowRoot!.querySelector<T>(selector);
const status = (el: Card) => q(el, "[data-testid=nota-status]")!.textContent!.trim();
const message = (el: Card) => q(el, "[data-testid=nota-msg]")!;

afterEach(() => {
  document.body.innerHTML = "";
});

describe("<tx-nota-tecnica>", () => {
  it("shows the Redis version and row count, read-only", async () => {
    const { el } = await mountCard({
      "GET /api/nota-tecnica": { exists: true, file_name: "ab12cd34ef56", version: "ab12cd34ef56".padEnd(64, "0"), filas: "12385" },
    });
    expect(status(el)).toBe("Versión ab12cd34ef56 · 12.385 filas");
    expect(message(el).textContent).toContain("solo lectura publicado en Redis");
    expect(q(el, "tx-file-upload")).toBeNull();
    expect(q(el, "[data-testid=nota-reemplazar]")).toBeNull();
  });

  it("says it is verifying before the first answer", async () => {
    apiSlot.api = { notaTecnica: () => new Promise(() => {}) } as never;
    const el = document.createElement("tx-nota-tecnica") as TxNotaTecnica;
    document.body.append(el);
    await el.updateComplete;
    expect(status(el)).toBe("verificando…");
    expect(message(el).textContent).not.toContain("paths.automatic");
  });

  it("says when Redis has no published Nota Técnica", async () => {
    const { el } = await mountCard({ "GET /api/nota-tecnica": { exists: false } });
    expect(status(el)).toBe("Sin Nota Técnica");
    expect(message(el).textContent).toContain("No hay una Nota Técnica disponible");
  });

  it("shows the error when the status cannot be read", async () => {
    const { el } = await mountCard({ "GET /api/nota-tecnica": new Response(JSON.stringify({ error: "sin carpeta" }), { status: 500 }) });
    expect(status(el)).toBe("No disponible");
    expect(message(el).textContent!.trim()).toBe("sin carpeta");
    expect(message(el).classList.contains("error")).toBe(true);
  });

  it("refreshes with a GET only and never posts", async () => {
    let filas = "10";
    const { el, callsTo, calls } = await mountCard({
      "GET /api/nota-tecnica": () => ({ exists: true, file_name: "ab12cd34ef56", filas }),
    });
    expect(status(el)).toBe("Versión ab12cd34ef56 · 10 filas");
    filas = "11";
    q<HTMLElement>(el, "[data-testid=nota-actualizar]")!.click();
    await settle(el);
    expect(status(el)).toBe("Versión ab12cd34ef56 · 11 filas");
    expect(callsTo("GET", "/api/nota-tecnica")).toHaveLength(2);
    expect(calls.every((c) => c.method === "GET")).toBe(true);
  });
});
