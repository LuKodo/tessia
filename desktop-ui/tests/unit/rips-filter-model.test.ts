import { describe, expect, it } from "vitest";
import {
  computeApplied,
  countActive,
  emptyDraft,
  hasCargueFilter,
  isKPIActive,
  isTextActive,
  toConsultaParams,
  toggleAlertDraft,
  togglePipelineDraft,
  toggleTextDraft,
} from "../../src/utils/rips-filter-model";
import type { FilterDraft } from "../../src/utils/rips-filter-model";

const applyDraft = (patch: Partial<FilterDraft>) => computeApplied({ ...emptyDraft(), ...patch });

// Ported from webui-src/src/rips/filterModel.test.js (same cases and expectations).
describe("rips filter model", () => {
  it("emptyDraft has every text filter blank and no alerts", () => {
    const d = emptyDraft();
    expect(Object.keys(d.text).sort()).toEqual(["autorizacion", "cargue", "factura", "mipres", "ssc", "subplan"]);
    expect(Object.values(d.text).every((v) => v === "")).toBe(true);
    expect(d.estado).toBe("");
    expect(d.pipeline).toBe("");
    expect(d.desde).toBe("");
    expect(d.hasta).toBe("");
    expect(d.alerts.size).toBe(0);
  });

  it("computeApplied trims, lowercases and drops empty text filters", () => {
    const d = emptyDraft();
    d.text.factura = "  FVS14  ";
    d.text.ssc = "   ";
    d.alerts.add("falta_zip");
    const a = computeApplied(d);
    expect(a.text).toEqual({ factura: "fvs14" });
    expect(a.alerts).toEqual(["falta_zip"]);
  });

  it("countActive counts text, estado, pipeline, dates and alerts", () => {
    expect(countActive(computeApplied(emptyDraft()))).toBe(0);
    const d = emptyDraft();
    d.text.factura = "x";
    d.estado = "TERMINADO";
    d.pipeline = "radicada";
    d.desde = "2026-01-01";
    d.hasta = "2026-02-01";
    d.alerts.add("a");
    d.alerts.add("b");
    expect(countActive(computeApplied(d))).toBe(7);
  });

  it("togglePipelineDraft toggles on and off without mutating the input", () => {
    const d = emptyDraft();
    const on = togglePipelineDraft(d, "radicada");
    expect(on.pipeline).toBe("radicada");
    expect(d.pipeline).toBe("");
    expect(togglePipelineDraft(on, "radicada").pipeline).toBe("");
    expect(togglePipelineDraft(on, "rechazada").pipeline).toBe("rechazada");
  });

  it("toggleAlertDraft adds/removes an alert on a copy of the set", () => {
    const d = emptyDraft();
    const on = toggleAlertDraft(d, "falta_zip");
    expect(on.alerts.has("falta_zip")).toBe(true);
    expect(d.alerts.size).toBe(0);
    expect(toggleAlertDraft(on, "falta_zip").alerts.has("falta_zip")).toBe(false);
  });

  it("isKPIActive reads the draft; total and unknown KPIs are never active", () => {
    const d = togglePipelineDraft(emptyDraft(), "radicada");
    expect(isKPIActive(d, "radicadas")).toBe(true);
    expect(isKPIActive(d, "rechazadas")).toBe(false);
    expect(isKPIActive(toggleAlertDraft(emptyDraft(), "nota_ajuste"), "nota_ajuste")).toBe(true);
    expect(isKPIActive(d, "total")).toBe(false);
    expect(isKPIActive(d, "nope")).toBe(false);
  });

  it("toggleTextDraft sets a text filter, and clears it when it already holds that value", () => {
    const on = toggleTextDraft(emptyDraft(), "cargue", "c1");
    expect(on.text.cargue).toBe("c1");
    expect(isTextActive(on, "cargue", "C1")).toBe(true);
    expect(isTextActive(on, "cargue", "c2")).toBe(false);
    const swapped = toggleTextDraft(on, "cargue", "c2");
    expect(swapped.text.cargue).toBe("c2");
    const off = toggleTextDraft(on, "cargue", " C1 ");
    expect(off.text.cargue).toBe("");
    expect(isTextActive(off, "cargue", "c1")).toBe(false);
    expect(on.text.factura, "other filters stay untouched").toBe("");
    expect(isTextActive(emptyDraft(), "cargue", "")).toBe(false);
  });

  it("hasCargueFilter is true only when an applied cargue text filter is set", () => {
    expect(hasCargueFilter(computeApplied(emptyDraft()))).toBe(false);
    const withFactura = computeApplied({ ...emptyDraft(), text: { ...emptyDraft().text, factura: "FVS1" } });
    expect(hasCargueFilter(withFactura)).toBe(false);
    const withCargue = computeApplied({ ...emptyDraft(), text: { ...emptyDraft().text, cargue: "Convenio_A" } });
    expect(hasCargueFilter(withCargue)).toBe(true);
  });

  it("toConsultaParams sends only the active filters, as the server expects them", () => {
    expect(toConsultaParams(applyDraft({}), { key: null, dir: 1 })).toEqual({});
    const d = emptyDraft();
    d.text.factura = "  FVS1 ";
    d.text.cargue = "Convenio Uno";
    d.estado = "TERMINADO";
    d.pipeline = "radicada";
    d.desde = "2026-05-01";
    d.hasta = "2026-05-31";
    d.alerts.add("falta_zip");
    d.alerts.add("nota_ajuste");
    expect(toConsultaParams(computeApplied(d), { key: null, dir: 1 })).toEqual({
      factura: "fvs1",
      cargue: "convenio uno",
      estado: "TERMINADO",
      pipeline: "radicada",
      desde: "2026-05-01",
      hasta: "2026-05-31",
      alerta: ["falta_zip", "nota_ajuste"],
    });
  });

  it("toConsultaParams maps the table sort: dir 1 is asc, -1 is desc, no key sends neither", () => {
    expect(toConsultaParams(applyDraft({}), { key: "fecha_factura", dir: -1 })).toEqual({ sort: "fecha_factura", dir: "desc" });
    expect(toConsultaParams(applyDraft({}), { key: "actualizado_en", dir: 1 })).toEqual({ sort: "actualizado_en", dir: "asc" });
    expect(toConsultaParams(applyDraft({}), { key: null, dir: -1 })).toEqual({});
  });
});
