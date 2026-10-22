import { describe, expect, it } from "vitest";
import { NO_FILTERS, hasFilters, rangeError, summaryText, toQuery } from "../../src/components/reportes-model";
import type { RadicadasReport } from "../../src/types/api";

const report = (over: Partial<RadicadasReport> = {}): RadicadasReport => ({
  generado_en: "2026-09-20T09:00:00Z",
  total: 3,
  truncado: false,
  filas: [],
  ...over,
});

describe("reportes model", () => {
  it("has no filters at first and notices each one", () => {
    expect(hasFilters(NO_FILTERS)).toBe(false);
    expect(hasFilters({ ...NO_FILTERS, cargues: ["A"] })).toBe(true);
    expect(hasFilters({ ...NO_FILTERS, desde: "2026-09-01" })).toBe(true);
    expect(hasFilters({ ...NO_FILTERS, hasta: "2026-09-01" })).toBe(true);
    expect(hasFilters({ ...NO_FILTERS, q: "fvs" })).toBe(true);
    expect(hasFilters({ ...NO_FILTERS, q: "   " })).toBe(false);
  });

  it("turns the filters into the API query, trimming the search", () => {
    expect(toQuery(NO_FILTERS)).toEqual({ cargue: [], desde: "", hasta: "", q: "" });
    expect(toQuery({ cargues: ["A", "B"], desde: "2026-09-01", hasta: "2026-09-30", q: "  FVS1 " })).toEqual({
      cargue: ["A", "B"],
      desde: "2026-09-01",
      hasta: "2026-09-30",
      q: "FVS1",
    });
  });

  it("rejects a range that ends before it starts, and only that", () => {
    expect(rangeError({ ...NO_FILTERS, desde: "2026-09-10", hasta: "2026-09-09" })).toBe("La fecha «desde» no puede ser posterior a «hasta».");
    expect(rangeError({ ...NO_FILTERS, desde: "2026-09-10", hasta: "2026-09-10" })).toBe("");
    expect(rangeError({ ...NO_FILTERS, desde: "2026-09-10" })).toBe("");
    expect(rangeError({ ...NO_FILTERS, hasta: "2026-09-10" })).toBe("");
  });

  it("describes how many invoices the table shows", () => {
    expect(summaryText(report({ total: 0 }))).toBe("Sin facturas radicadas con estos filtros");
    expect(summaryText(report({ total: 1 }))).toBe("1 factura radicada");
    expect(summaryText(report({ total: 3 }))).toBe("3 facturas radicadas");
    expect(summaryText(report({ total: 1234, truncado: true, filas: new Array(500).fill(null) }))).toBe(
      "Mostrando 500 de 1.234 facturas radicadas. Descarga el CSV para obtenerlas todas.",
    );
  });
});
