import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/components/health-indicator";
import { ApiError } from "../../src/utils/api-client";
import { mountEl, q, settle } from "../support/dashboard-dom";

const holder = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("../../src/api", () => ({ api: new Proxy({}, { get: (_t, k) => holder.api[k as string] }) }));

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
});

async function mountIndicator(health: () => Promise<unknown>) {
  holder.api = { health: vi.fn(health) };
  const el = await mountEl("tx-health-indicator");
  await vi.advanceTimersByTimeAsync(0);
  await settle(el);
  return el;
}
const badge = (el: Element) => q(el, "[data-testid=health-indicator]")!;

describe("<tx-health-indicator>", () => {
  it("says it is verifying until the first answer", async () => {
    holder.api = { health: vi.fn(() => new Promise(() => {})) };
    const el = await mountEl("tx-health-indicator");
    await settle(el);
    expect(badge(el).getAttribute("data-state")).toBe("unknown");
    expect(badge(el).textContent!.trim()).toBe("Verificando…");
  });

  it("is green and explains itself in a tooltip when every service is up", async () => {
    const el = await mountIndicator(async () => [{ nombre: "PostgreSQL", disponible: true, latencia_ms: 1 }]);
    expect(badge(el).getAttribute("data-state")).toBe("ok");
    expect(badge(el).getAttribute("color")).toBe("success");
    expect(badge(el).textContent!.trim()).toBe("Servicio activo");
    expect(q(el, "tx-tooltip")!.getAttribute("text")).toBe("Todos los servicios disponibles");
  });

  it("is degraded and names what is down", async () => {
    const el = await mountIndicator(async () => [
      { nombre: "PostgreSQL", disponible: true, latencia_ms: 1 },
      { nombre: "Portal", disponible: false, latencia_ms: 0 },
    ]);
    expect(badge(el).getAttribute("data-state")).toBe("degraded");
    expect(badge(el).getAttribute("color")).toBe("warning");
    expect(q(el, "tx-tooltip")!.getAttribute("text")).toBe("No disponible: Portal");
  });

  it("is unreachable with the connectivity hint when the service does not answer", async () => {
    const el = await mountIndicator(async () => {
      throw new ApiError(0, "Failed to fetch");
    });
    expect(badge(el).getAttribute("data-state")).toBe("unreachable");
    expect(badge(el).textContent!.trim()).toBe("Sin conexión");
    expect(q(el, "tx-tooltip")!.getAttribute("text")).toContain("Failed to fetch\n");
  });

  it("re-checks every 5 s", async () => {
    const el = await mountIndicator(async () => [{ nombre: "PostgreSQL", disponible: true, latencia_ms: 1 }]);
    await vi.advanceTimersByTimeAsync(10000);
    await settle(el);
    expect((holder.api.health as ReturnType<typeof vi.fn>).mock.calls.length).toBe(3);
  });
});
