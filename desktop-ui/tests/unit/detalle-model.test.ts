import { describe, expect, it } from "vitest";
import type { AgreementLine, TrackingRow } from "../../src/types/api";
import {
  formatMoney,
  glosaRisk,
  hasConvenio,
  notesOf,
  pasoOk,
  plazoInfo,
  rejectionCode,
  stageCards,
  stepperSteps,
  timelineItems,
  valueDiff,
} from "../../src/utils/detalle-model";

const OK4: TrackingRow["radicacion"] = ["ok", "ok", "ok", "ok"];
const base = (over: Partial<TrackingRow> = {}): Partial<TrackingRow> => ({
  resultado: "preparada",
  artefactos: OK4,
  radicacion: ["pendiente", "pendiente", "pendiente", "pendiente"],
  ...over,
});

// Ported from webui-src/src/rips/detalleModel.test.js (same cases and expectations).
describe("detalle model", () => {
  it("pasoOk reads the radicacion dot", () => {
    const r = base({ radicacion: ["ok", "pendiente", "error", "en_curso"] });
    expect(pasoOk(r, 1)).toBe(true);
    expect(pasoOk(r, 2)).toBe(false);
    expect(pasoOk(r, 3)).toBe(false);
    expect(pasoOk(r, 4)).toBe(false);
    expect(pasoOk({}, 1)).toBe(false);
  });

  it("pasoOk is false for steps 3-4 when a nota de ajuste is required", () => {
    const r = base({ radicacion: OK4, requiere_nota_ajuste: true });
    expect([1, 2, 3, 4].map((p) => pasoOk(r, p))).toEqual([true, true, false, false]);
  });

  it("stepperSteps lists the 8 steps in order", () => {
    const s = stepperSteps(
      base({ artefactos: ["ok", "ok", "error", "pendiente"], radicacion: ["ok", "pendiente", "pendiente", "pendiente"] }),
    );
    expect(s.map((x) => x.label)).toEqual([
      "ZIP XML/PDF",
      "JSON",
      "TXT/CUV",
      "ZIP CRC",
      "Radicación ZIP XML/PDF",
      "Radicación TXT/CUV",
      "Radicación JSON",
      "Radicación ZIP CRC",
    ]);
    expect(s.map((x) => x.ok)).toEqual([true, true, false, false, true, false, false, false]);
  });

  it("rejectionCode extracts Mutual Ser codes", () => {
    expect(rejectionCode("fallo FE-AAA-123 algo")).toBe("FE-AAA-123");
    expect(rejectionCode("x RF-FE-12 y")).toBe("RF-FE-12");
    expect(rejectionCode("RVG45 invalido")).toBe("RVG45");
    expect(rejectionCode("sin codigo")).toBeNull();
    expect(rejectionCode(undefined)).toBeNull();
  });

  it("plazoInfo: RFFE022 expired never computed from dates", () => {
    const p = plazoInfo(base({ vencida_rffe022: true, fecha_factura: "2020-01-01" }));
    expect(p.kind).toBe("vencida");
    expect(p.badge?.text).toBe("NOTA CRÉDITO · vencido (RFFE022)");
    expect(p.badge?.color).toBe("error");
    const old = plazoInfo(base({ fecha_factura: "2000-01-01" }));
    expect(old.kind).not.toBe("vencida");
  });

  it("plazoInfo: complete filing", () => {
    const p = plazoInfo(base({ radicacion: OK4 }));
    expect(p.kind).toBe("completa");
    expect(p.badge).toEqual({ text: "Radicación completa", color: "success" });
  });

  it("plazoInfo: first pending step, plain or rejected", () => {
    const plain = plazoInfo(base({ radicacion: ["ok", "pendiente", "pendiente", "pendiente"], fecha_factura: "2026-09-01" }));
    expect(plain.kind).toBe("pendiente");
    expect(plain.fecha).toBe("2026-09-01");
    expect(plain.badge).toEqual({ text: "Falta Paso 2", color: "neutral" });
    const rej = plazoInfo(base({ radicacion: ["ok", "ok", "error", "pendiente"], estado_detalle: "rechazo RF-FE-99" }));
    expect(rej.badge).toEqual({ text: "Paso 3 rechazado (RF-FE-99)", color: "error" });
  });

  it("plazoInfo: no badge for excluded rows", () => {
    expect(plazoInfo(base({ resultado: "excluida" })).badge).toBeNull();
  });

  it("valueDiff and validity", () => {
    expect(valueDiff({ valor_xml: "100", valor_convenio: "100" })?.diff).toBe(0);
    expect(valueDiff({ valor_xml: "100", valor_convenio: "100" })?.ok).toBe(true);
    expect(valueDiff({ valor_xml: "100.005", valor_convenio: "100" })?.ok).toBe(true);
    const d = valueDiff({ valor_xml: "150", valor_convenio: "100" });
    expect(d?.diff).toBe(50);
    expect(d?.ok).toBe(false);
    expect(valueDiff({ valor_xml: "", valor_convenio: "100" })).toBeNull();
    expect(valueDiff({ valor_convenio: "100" })).toBeNull();
    expect(valueDiff({ valor_xml: "abc", valor_convenio: "1" })).toBeNull();
  });

  it("glosaRisk: step 4 done and no authorization", () => {
    expect(glosaRisk(base({ radicacion: OK4 }))).toBe(true);
    expect(glosaRisk(base({ radicacion: OK4, autorizacion: "123" }))).toBe(false);
    expect(glosaRisk(base({ autorizacion: "" }))).toBe(false);
  });

  it("hasConvenio", () => {
    expect(hasConvenio({})).toBe(false);
    expect(hasConvenio({ ssc: "1" })).toBe(true);
    expect(hasConvenio({ lineas_convenio: [{ ssc: "1" } as AgreementLine] })).toBe(true);
  });

  it("timelineItems states", () => {
    const r = base({
      radicacion: ["ok", "ok", "pendiente", "pendiente"],
      requiere_nota_ajuste: true,
    });
    const t = timelineItems(r);
    expect(t.map((x) => x.label)).toEqual([
      "Cargue ZIP XML/PDF a Mutualser",
      "Radicación TXT/CUV",
      "Radicación JSON",
      "Radicación ZIP CRC",
    ]);
    expect(t.map((x) => x.state)).toEqual(["completado", "completado", "reproceso", "reproceso"]);
    const t2 = timelineItems(base());
    expect(t2.map((x) => x.state)).toEqual(["pendiente", "pendiente", "pendiente", "pendiente"]);
  });

  it("stageCards from artefactos", () => {
    const c = stageCards(base({ artefactos: ["ok", "error", "ok", "pendiente"] }));
    expect(c.map((x) => x.titulo)).toEqual(["ZIP XML/PDF", "JSON", "TXT/CUV", "ZIP CRC"]);
    expect(c.map((x) => x.ok)).toEqual([true, false, true, false]);
  });

  it("notesOf", () => {
    expect(notesOf({ requiere_nota_ajuste: true })).toEqual({ ajuste: true, credito: false });
    expect(notesOf({ vencida_rffe022: true })).toEqual({ ajuste: false, credito: true });
    expect(notesOf({})).toEqual({ ajuste: false, credito: false });
  });

  it("formatMoney keeps zero", () => {
    expect(formatMoney(0)).toMatch(/^\$ 0$/);
    expect(formatMoney(1234.5)).toMatch(/^\$ 1[.,\s]?234[.,]5$/);
  });
});
