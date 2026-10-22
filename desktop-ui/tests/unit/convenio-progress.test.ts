import { expect, test } from "vitest";
import { convenioStrip } from "../../src/utils/convenio-progress";
import type { JobConvenio } from "../../src/types/api";

const conv = (over?: Partial<JobConvenio>): JobConvenio => ({
  estado: "preparando", evaluadas: 0, total: 0, preparadas: 0, excluidas: 0, revision_manual: 0, ...over,
});

test("no convenio info hides the strip", () => {
  expect(convenioStrip(undefined)).toBe(null);
  expect(convenioStrip(null)).toBe(null);
  expect(convenioStrip({})).toBe(null);
  expect(convenioStrip({ estado: "desconocido" })).toBe(null);
});

test("preparando without a total is indeterminate (the CSV and nota técnica are still loading)", () => {
  const s = convenioStrip(conv())!;
  expect(s.estado).toBe("preparando");
  expect(s.title).toBe("Preparación del convenio");
  expect(s.determinate).toBe(false);
  expect(s.pct).toBe(null);
  expect(s.text).toBe("Leyendo el CSV y la nota técnica…");
  expect(s.chips).toEqual([]);
  expect(s.color).toBe("primary");
});

test("preparando with a total is a determinate percentage and reads evaluated of total", () => {
  const s = convenioStrip(conv({ evaluadas: 40, total: 160 }))!;
  expect(s.determinate).toBe(true);
  expect(s.pct).toBe(25);
  expect(s.text).toBe("Evaluadas 40 de 160 facturas");
});

test("preparando at the start of the walk is 0 percent, not indeterminate", () => {
  const s = convenioStrip(conv({ evaluadas: 0, total: 10 }))!;
  expect(s.determinate).toBe(true);
  expect(s.pct).toBe(0);
  expect(s.text).toBe("Evaluadas 0 de 10 facturas");
});

test("the percentage is clamped and rounded", () => {
  expect(convenioStrip(conv({ evaluadas: 1, total: 3 }))!.pct).toBe(33);
  expect(convenioStrip(conv({ evaluadas: 9, total: 3 }))!.pct).toBe(100);
  expect(convenioStrip(conv({ evaluadas: -4, total: 3 }))!.pct).toBe(0);
});

test("a single invoice uses the singular", () => {
  expect(convenioStrip(conv({ evaluadas: 0, total: 1 }))!.text).toBe("Evaluadas 0 de 1 factura");
});

test("ok settles into the prepared, excluded and manual review chips", () => {
  const s = convenioStrip(conv({ estado: "ok", evaluadas: 100, total: 100, preparadas: 90, excluidas: 7, revision_manual: 3 }))!;
  expect(s.estado).toBe("ok");
  expect(s.determinate).toBe(true);
  expect(s.pct).toBe(100);
  expect(s.text).toBe("Convenio preparado");
  expect(s.color).toBe("success");
  expect(s.chips.map(c => [c.key, c.label, c.color])).toEqual([
    ["preparadas", "Preparadas: 90", "success"],
    ["excluidas", "Excluidas: 7", "neutral"],
    ["revision_manual", "Revisión manual: 3", "warning"],
  ]);
});

test("ok keeps every chip even when a count is zero", () => {
  const s = convenioStrip(conv({ estado: "ok", preparadas: 5 }))!;
  expect(s.chips.map(c => c.label)).toEqual(["Preparadas: 5", "Excluidas: 0", "Revisión manual: 0"]);
});

test("ok is full even when --limit stopped the walk early", () => {
  const s = convenioStrip(conv({ estado: "ok", evaluadas: 12, total: 500, preparadas: 10, excluidas: 2 }))!;
  expect(s.pct).toBe(100);
});

test("error shows the message with the error color", () => {
  const s = convenioStrip(conv({ estado: "error", evaluadas: 5, total: 8, mensaje: "mipres: sin conexión" }))!;
  expect(s.estado).toBe("error");
  expect(s.color).toBe("error");
  expect(s.text).toBe("No se pudo preparar el convenio");
  expect(s.message).toBe("mipres: sin conexión");
  expect(s.determinate).toBe(true);
  expect(s.pct).toBe(63);
  expect(s.chips).toEqual([]);
});

test("error without a message still explains itself", () => {
  const s = convenioStrip(conv({ estado: "error" }))!;
  expect(s.message).toBe("La preparación terminó con un error; revisa la consola.");
  expect(s.pct).toBe(0);
});

test("aria values describe the bar", () => {
  const s = convenioStrip(conv({ evaluadas: 40, total: 160 }))!;
  expect(s.aria).toEqual({ label: "Preparación del convenio", min: 0, max: 100, now: 25, text: "Evaluadas 40 de 160 facturas" });
  const i = convenioStrip(conv())!;
  expect(i.aria.now).toBe(undefined);
});
