import { describe, expect, it } from "vitest";
import {
  MAX_DESCRIPTION,
  buildPayload,
  descriptionError,
  filterHallazgos,
  needsSave,
  normalizeDescription,
  summaryText,
} from "../../src/utils/hallazgos-model";

const items = [
  { codigo: "RVC012", detalle: "Valor de la Atención no coincide", genera_glosa: false, descripcion: "" },
  { codigo: "RVC040", detalle: "Código CUPS inválido", genera_glosa: true, descripcion: "Se glosa" },
  { codigo: "XYZ001", detalle: "Diagnóstico principal vacío", genera_glosa: false, descripcion: "" },
];

// Ported from webui-src/src/hallazgosModel.test.js (same cases and expectations).
describe("filterHallazgos", () => {
  it("returns every item for a blank query", () => {
    expect(filterHallazgos(items, "")).toEqual(items);
    expect(filterHallazgos(items, "   ")).toEqual(items);
    expect(filterHallazgos(items, null)).toEqual(items);
    expect(filterHallazgos(items, undefined)).toEqual(items);
  });

  it("matches the code ignoring case", () => {
    expect(filterHallazgos(items, "rvc0")).toEqual([items[0], items[1]]);
    expect(filterHallazgos(items, "xyz001")).toEqual([items[2]]);
  });

  it("matches the detail ignoring case and accents", () => {
    expect(filterHallazgos(items, "codigo cups")).toEqual([items[1]]);
    expect(filterHallazgos(items, "ATENCION")).toEqual([items[0]]);
    expect(filterHallazgos(items, "diagnóstico")).toEqual([items[2]]);
  });

  it("returns nothing when no field matches", () => {
    expect(filterHallazgos(items, "inexistente")).toEqual([]);
  });

  it("tolerates null items and missing fields", () => {
    expect(filterHallazgos(null, "a")).toEqual([]);
    expect(filterHallazgos(undefined, "")).toEqual([]);
    expect(filterHallazgos([{ codigo: "A1" }, null], "a1")).toEqual([{ codigo: "A1" }]);
  });
});

describe("needsSave", () => {
  it("is false when nothing changed", () => {
    const item = { genera_glosa: true, descripcion: "texto" };
    expect(needsSave(item, { genera_glosa: true, descripcion: "texto" })).toBe(false);
  });

  it("ignores surrounding whitespace and CRLF in the description", () => {
    const item = { genera_glosa: false, descripcion: "a\nb" };
    expect(needsSave(item, { genera_glosa: false, descripcion: "  a\r\nb \n" })).toBe(false);
  });

  it("is true when the checkbox or the trimmed description differ", () => {
    const item = { genera_glosa: false, descripcion: "texto" };
    expect(needsSave(item, { genera_glosa: true, descripcion: "texto" })).toBe(true);
    expect(needsSave(item, { genera_glosa: false, descripcion: "otro" })).toBe(true);
    expect(needsSave({ genera_glosa: false, descripcion: "" }, { genera_glosa: false, descripcion: "x" })).toBe(true);
  });

  it("treats missing values as empty / false", () => {
    expect(needsSave({}, { genera_glosa: false, descripcion: "" })).toBe(false);
    expect(needsSave({ descripcion: null }, { genera_glosa: undefined, descripcion: undefined })).toBe(false);
    expect(needsSave({ genera_glosa: true }, {})).toBe(true);
  });

  it("is true when only devolucion differs", () => {
    const item = { genera_glosa: true, devolucion: false, descripcion: "texto" };
    expect(needsSave(item, { genera_glosa: true, devolucion: true, descripcion: "texto" })).toBe(true);
    expect(needsSave({ ...item, devolucion: true }, { genera_glosa: true, devolucion: false, descripcion: "texto" })).toBe(true);
  });

  it("is false when devolucion is unchanged", () => {
    const item = { genera_glosa: true, devolucion: true, descripcion: "texto" };
    expect(needsSave(item, { genera_glosa: true, devolucion: true, descripcion: " texto " })).toBe(false);
  });

  it("treats a missing devolucion as false", () => {
    expect(needsSave({ genera_glosa: false, descripcion: "" }, { genera_glosa: false, devolucion: false, descripcion: "" })).toBe(false);
    expect(needsSave({ genera_glosa: false, devolucion: false, descripcion: "" }, { genera_glosa: false, descripcion: "" })).toBe(false);
    expect(needsSave({ genera_glosa: false, descripcion: "" }, { genera_glosa: false, devolucion: true, descripcion: "" })).toBe(true);
  });
});

describe("buildPayload", () => {
  it("always carries the three fields with a normalized description", () => {
    expect(buildPayload({ genera_glosa: true, devolucion: true, descripcion: "  a\r\nb " })).toEqual({
      genera_glosa: true,
      devolucion: true,
      descripcion: "a\nb",
    });
    expect(buildPayload({})).toEqual({ genera_glosa: false, devolucion: false, descripcion: "" });
    expect(buildPayload({ genera_glosa: true, descripcion: "x" })).toEqual({
      genera_glosa: true,
      devolucion: false,
      descripcion: "x",
    });
  });
});

describe("normalizeDescription", () => {
  it("trims and collapses CRLF", () => {
    expect(normalizeDescription("  hola\r\nmundo\r\n")).toBe("hola\nmundo");
    expect(normalizeDescription("a\rb")).toBe("a\nb");
    expect(normalizeDescription(null)).toBe("");
    expect(normalizeDescription(undefined)).toBe("");
  });
});

describe("descriptionError", () => {
  it("accepts up to 2000 code points and rejects more", () => {
    expect(MAX_DESCRIPTION).toBe(2000);
    expect(descriptionError("a".repeat(2000))).toBeNull();
    expect(descriptionError("")).toBeNull();
    expect(descriptionError(null)).toBeNull();
    expect(descriptionError("a".repeat(2001))).toBe("La descripción no puede superar 2000 caracteres.");
  });

  it("counts code points, not UTF-16 units", () => {
    // 2000 emoji are 4000 UTF-16 units but 2000 code points: still valid.
    expect(descriptionError("😀".repeat(2000))).toBeNull();
    expect(descriptionError("😀".repeat(2001))).toBe("La descripción no puede superar 2000 caracteres.");
  });
});

describe("summaryText", () => {
  it("shows shown of total with singular for one", () => {
    expect(summaryText(3, 10)).toBe("3 de 10 hallazgos");
    expect(summaryText(0, 10)).toBe("0 de 10 hallazgos");
    expect(summaryText(1, 1)).toBe("1 de 1 hallazgo");
    expect(summaryText(0, 0)).toBe("0 de 0 hallazgos");
  });
});
