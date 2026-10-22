import { describe, expect, it } from "vitest";
import { ApiError } from "../../src/utils/api-client";
import { summarizeHealth } from "../../src/utils/health";

const up = { nombre: "Medicar", disponible: true, latencia_ms: 3 };
const down = { nombre: "SISPRO", disponible: false, detalle: "timeout", latencia_ms: 3000 };

describe("summarizeHealth", () => {
  it("is ok when every service is available", () => {
    expect(summarizeHealth([up, up], null).state).toBe("ok");
  });

  it("is degraded when some service is down and names it", () => {
    const s = summarizeHealth([up, down], null);
    expect(s.state).toBe("degraded");
    expect(s.detail).toContain("SISPRO");
  });

  it("is unreachable when the request failed, even with stale data", () => {
    expect(summarizeHealth([up], new ApiError(0, "sin conexion")).state).toBe("unreachable");
  });

  it("is unknown before the first answer and for an empty list", () => {
    expect(summarizeHealth(null, null).state).toBe("unknown");
    expect(summarizeHealth([], null).state).toBe("unknown");
  });

  it("appends the connectivity hint to the detail of a network failure, keeping the raw text", () => {
    const s = summarizeHealth(null, new ApiError(0, "No se pudo conectar"), {
      apiBase: "http://localhost:8090",
      origin: "http://localhost:3000",
    });
    expect(s.state).toBe("unreachable");
    expect(s.label).toBe("Sin conexión");
    expect(s.detail).toContain("No se pudo conectar");
    expect(s.detail).toContain("--allowed-origin http://localhost:3000");
  });

  it("keeps only the raw detail for an HTTP error", () => {
    const s = summarizeHealth(null, new ApiError(500, "boom"), { apiBase: "http://x", origin: "http://y" });
    expect(s.detail).toBe("boom");
  });
});
