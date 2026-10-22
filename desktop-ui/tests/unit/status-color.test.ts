import { expect, test } from "vitest";
import { statusColor } from "../../src/utils/status-color";
import { PHASES } from "../../src/utils/phases";

test("statusColor maps in-progress states to warning", () => {
  for (const s of ["procesando", "Cargue", "en proceso", "EN_PROCESO", "iniciado", "sin preparar"]) {
    expect(statusColor(s), s).toBe("warning");
  }
});

test("statusColor maps finished states to success", () => {
  for (const s of ["completado", "Terminado"]) {
    expect(statusColor(s), s).toBe("success");
  }
});

test("statusColor maps failure states to error", () => {
  for (const s of ["conincidencias", "rechazado", "vencido", "nota ajuste", "terminado con error"]) {
    expect(statusColor(s), s).toBe("error");
  }
});

test("statusColor maps empty and unknown states to neutral", () => {
  for (const s of ["sin datos", "sin facturas", "excluida", "no iniciado", "otra cosa", "", null, undefined]) {
    expect(statusColor(s), String(s)).toBe("neutral");
  }
});

test("PHASES lists the pipeline phases in order with their Spanish labels", () => {
  expect(PHASES.map(([key]) => key)).toEqual([
    "convenio", "thefactory", "fev", "conciliar", "derechos", "rips", "cuv", "soporte", "radicacion",
  ]);
  expect(Object.fromEntries(PHASES).radicacion).toBe("Radicación");
});
