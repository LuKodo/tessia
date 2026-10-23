import { describe, expect, it } from "vitest";
import { stabilizeRows } from "../../src/components/rips/stabilize-rows";
import { trackingRow } from "../support/dashboard-fixtures";

describe("stabilizeRows", () => {
  it("keeps the previous object of every row that did not change, so a poll does not re-render it", () => {
    const prev = [trackingRow({ factura: "A" }), trackingRow({ factura: "B" })];
    const next = [trackingRow({ factura: "A" }), trackingRow({ factura: "B", estado: "TERMINADO" })];
    const out = stabilizeRows(prev, next);
    expect(out[0]).toBe(prev[0]);
    expect(out[1]).toBe(next[1]);
  });

  it("returns the previous array itself when nothing changed at all", () => {
    const prev = [trackingRow({ factura: "A" })];
    expect(stabilizeRows(prev, [trackingRow({ factura: "A" })])).toBe(prev);
  });

  it("follows the order and the rows of the new answer", () => {
    const prev = [trackingRow({ factura: "A" }), trackingRow({ factura: "B" })];
    const out = stabilizeRows(prev, [trackingRow({ factura: "C" }), trackingRow({ factura: "A" })]);
    expect(out.map((r) => r.factura)).toEqual(["C", "A"]);
    expect(out[1]).toBe(prev[0]);
  });

  it("tells apart the same invoice number in two cargues", () => {
    const prev = [trackingRow({ factura: "A", cargue: "c1" })];
    const out = stabilizeRows(prev, [trackingRow({ factura: "A", cargue: "c2" })]);
    expect(out[0]).not.toBe(prev[0]);
  });

  it("works from nothing", () => {
    const next = [trackingRow()];
    expect(stabilizeRows(null, next)).toBe(next);
  });
});
