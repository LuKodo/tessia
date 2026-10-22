import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/components/health-cards";
import { apiBase } from "../../src/config";
import { ApiError } from "../../src/utils/api-client";
import { deepText, mountEl, q, qa, settle, text } from "../support/dashboard-dom";

const holder = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("../../src/api", () => ({ api: new Proxy({}, { get: (_t, k) => holder.api[k as string] }) }));

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
});

async function mountCards(api: Record<string, unknown>) {
  holder.api = api;
  const el = await mountEl("tx-health-cards");
  await vi.advanceTimersByTimeAsync(0);
  await settle(el);
  return el;
}

describe("<tx-health-cards>", () => {
  it("renders a card per service with its state label and detail, plus the nota técnica card", async () => {
    const el = await mountCards({
      health: vi.fn(async () => [
        { nombre: "MySQL", disponible: true, latencia_ms: 4 },
        { nombre: "Portal", disponible: false, detalle: "timeout", latencia_ms: 0 },
      ]),
      notaTecnica: vi.fn(async () => ({ exists: true, file_name: "nota.xlsx", modified_at: "2026-10-01T10:00:00Z" })),
    });
    const cards = qa(el, "[data-testid=health-card]");
    expect(cards).toHaveLength(3);
    expect(cards.map((c) => c.getAttribute("data-state"))).toEqual(["ok", "down", "ok"]);
    expect(deepText(cards[0]!)).toContain("MySQL");
    expect(cards[0]!.textContent).toContain("disponible");
    expect(cards[0]!.textContent).toContain("ping · 4 ms");
    expect(cards[1]!.textContent).toContain("no disponible");
    expect(cards[1]!.textContent).toContain("timeout");
    expect(cards[2]!.textContent).toContain("Nota técnica");
    expect(cards[2]!.textContent).toContain("cargada");
  });

  it("shows neutral placeholders until the first answers", async () => {
    holder.api = { health: vi.fn(() => new Promise(() => {})), notaTecnica: vi.fn(() => new Promise(() => {})) };
    const el = await mountEl("tx-health-cards");
    await settle(el);
    expect(qa(el, "[data-testid=health-card]")).toHaveLength(0);
    expect(qa(el, "tx-skeleton").length).toBeGreaterThan(0);
  });

  it("shows why the first request failed, with the connectivity hint", async () => {
    const el = await mountCards({
      health: vi.fn(async () => {
        throw new ApiError(0, "Failed to fetch");
      }),
      notaTecnica: vi.fn(async () => ({ exists: false, file_name: "" })),
    });
    expect(text(el)).toContain("Failed to fetch");
    expect(q(el, "[data-testid=connectivity-hint]")!.textContent).toContain(apiBase.replace(/\/+$/, "") || window.location.origin);
    expect(text(el)).toContain("no cargada");
  });

  it("a failing nota técnica shows its own error card without hiding the services", async () => {
    const el = await mountCards({
      health: vi.fn(async () => [{ nombre: "MySQL", disponible: true, latencia_ms: 1 }]),
      notaTecnica: vi.fn(async () => {
        throw new Error("sin archivo");
      }),
    });
    const cards = qa(el, "[data-testid=health-card]");
    expect(cards).toHaveLength(2);
    expect(cards[1]!.textContent).toContain("sin archivo");
  });

  it("polls the services every 5 s and the nota técnica every 60 s, keeping the last cards on a failed tick", async () => {
    const health = vi.fn().mockResolvedValueOnce([{ nombre: "MySQL", disponible: true, latencia_ms: 4 }]).mockRejectedValue(new Error("caído"));
    const notaTecnica = vi.fn(async () => ({ exists: true, file_name: "n.xlsx" }));
    const el = await mountCards({ health, notaTecnica });
    await vi.advanceTimersByTimeAsync(5000);
    await settle(el);
    expect(health).toHaveBeenCalledTimes(2);
    expect(notaTecnica).toHaveBeenCalledTimes(1);
    expect(qa(el, "[data-testid=health-card]")[0]!.textContent).toContain("MySQL");
    await vi.advanceTimersByTimeAsync(55000);
    expect(notaTecnica).toHaveBeenCalledTimes(2);
  });
});
