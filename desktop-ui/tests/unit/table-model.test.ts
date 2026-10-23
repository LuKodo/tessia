import { describe, expect, it } from "vitest";
import { nextSort, subLine } from "../../src/utils/table-model";

describe("tracking table model", () => {
  it("nextSort starts descending on a new column, then toggles direction", () => {
    const s1 = nextSort({ key: null, dir: 1 }, "fecha_factura");
    expect(s1).toEqual({ key: "fecha_factura", dir: -1 });
    const s2 = nextSort(s1, "fecha_factura");
    expect(s2).toEqual({ key: "fecha_factura", dir: 1 });
    expect(nextSort(s2, "fecha_factura")).toEqual({ key: "fecha_factura", dir: -1 });
    expect(nextSort(s2, "actualizado_en")).toEqual({ key: "actualizado_en", dir: -1 });
  });

  it("subLine counts the invoices of the server, marks filters and says how many are loaded", () => {
    expect(subLine({ total: 3, loaded: 3, filtered: false })).toBe("3 facturas");
    expect(subLine({ total: 1, loaded: 1, filtered: false })).toBe("1 factura");
    expect(subLine({ total: 3, loaded: 3, filtered: true })).toBe("3 facturas (filtradas)");
    expect(subLine({ total: 950, loaded: 300, filtered: false })).toBe("950 facturas · mostrando 300");
    expect(subLine({ total: 0, loaded: 0, filtered: true })).toBe("0 facturas (filtradas)");
  });

  it("subLine falls back to the loaded count while the total is unknown", () => {
    expect(subLine({ total: undefined, loaded: 2, filtered: false })).toBe("2 facturas cargadas");
    expect(subLine({ total: undefined, loaded: 1, filtered: true })).toBe("1 factura cargada");
  });
});
