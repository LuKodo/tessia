import { expect, test } from "vitest";
import { detectJobFinish, detectServerRestart, alertTitle, formatDuration } from "../../src/utils/job-alert";

const running = { id: "job-1", tipo: "proceso run", corriendo: true, iniciado_en: "2026-10-01T10:00:00Z" };
const done = { ...running, corriendo: false, terminado_en: "2026-10-01T10:02:05Z" };

test("never alerts on the first observation, even for a finished job", () => {
  expect(detectJobFinish(null, done)).toBe(null);
  expect(detectJobFinish(undefined, done)).toBe(null);
});

test("alerts ok when a running job finishes without error", () => {
  const a = detectJobFinish(running, done)!;
  expect(a.id).toBe("job-1");
  expect(a.tipo).toBe("proceso run");
  expect(a.outcome).toBe("ok");
  expect(a.durationText).toBe("2 min 5 s");
  expect(a.message).toMatch(/terminado/i);
  expect(a.message).toMatch(/proceso run/);
});

test("alerts error when the job finishes with a non-cancellation error", () => {
  const a = detectJobFinish(running, { ...done, error: "conexión rechazada por Medicar" })!;
  expect(a.outcome).toBe("error");
  expect(a.message).toMatch(/error/i);
});

test("detects the server cancellation message case-insensitively", () => {
  expect(detectJobFinish(running, { ...done, error: "cancelado por el operador" })!.outcome).toBe("cancelado");
  expect(detectJobFinish(running, { ...done, error: "Cancelado por el Operador" })!.outcome).toBe("cancelado");
  expect(detectJobFinish(running, { ...done, error: "context canceled" })!.outcome).toBe("cancelado");
  expect(detectJobFinish(running, { ...done, error: "cancelado por el operador" })!.message).toMatch(/cancelado/i);
});

test("does not alert twice for the same finished id", () => {
  expect(detectJobFinish(done, { ...done })).toBe(null);
});

test("does not alert while the job is still running", () => {
  expect(detectJobFinish(running, { ...running, salida: "more" })).toBe(null);
});

test("alerts when the id changes and the new job is already finished", () => {
  const next = { ...done, id: "job-2" };
  expect(detectJobFinish(done, next)!.id).toBe("job-2");
  expect(detectJobFinish(running, next)!.id).toBe("job-2");
});

test("alerts for a short job observed after an idle poll", () => {
  expect(detectJobFinish({}, done)!.id).toBe("job-1");
});

test("idle states never alert", () => {
  expect(detectJobFinish({}, {})).toBe(null);
  expect(detectJobFinish(running, {})).toBe(null);
  expect(detectJobFinish(running, null)).toBe(null);
  expect(detectJobFinish(running, { id: "", corriendo: false })).toBe(null);
});

test("durationText is empty when timestamps are unknown", () => {
  expect(detectJobFinish(running, { ...done, terminado_en: undefined })!.durationText).toBe("");
  expect(detectJobFinish(running, { ...done, iniciado_en: undefined })!.durationText).toBe("");
  expect(detectJobFinish(running, { ...done, terminado_en: "0001-01-01T00:00:00Z" })!.durationText).toBe("");
  expect(detectJobFinish(running, { ...done, terminado_en: "garbage" })!.durationText).toBe("");
});

test("formatDuration covers seconds, minutes and hours", () => {
  expect(formatDuration("2026-10-01T10:00:00Z", "2026-10-01T10:00:00Z")).toBe("0 s");
  expect(formatDuration("2026-10-01T10:00:00Z", "2026-10-01T10:00:42Z")).toBe("42 s");
  expect(formatDuration("2026-10-01T10:00:00Z", "2026-10-01T10:02:05Z")).toBe("2 min 5 s");
  expect(formatDuration("2026-10-01T10:00:00Z", "2026-10-01T11:03:09Z")).toBe("1 h 3 min");
  expect(formatDuration("2026-10-01T10:00:10Z", "2026-10-01T10:00:00Z")).toBe("");
});

test("detectServerRestart never alerts on the first observation", () => {
  expect(detectServerRestart(null, { ...running, instancia_servidor: "A" })).toBe(null);
  expect(detectServerRestart(undefined, { ...running, instancia_servidor: "A" })).toBe(null);
});

test("detectServerRestart stays quiet while the instance id is unchanged", () => {
  const prev = { ...running, instancia_servidor: "A" };
  const next = { ...running, instancia_servidor: "A" };
  expect(detectServerRestart(prev, next)).toBe(null);
});

test("detectServerRestart stays quiet when no job was running", () => {
  const prev = { ...done, instancia_servidor: "A" };
  const next = { ...done, instancia_servidor: "B" };
  expect(detectServerRestart(prev, next)).toBe(null);
});

test("detectServerRestart stays quiet when instance ids are missing", () => {
  expect(detectServerRestart({ ...running }, { ...running })).toBe(null);
  expect(detectServerRestart({ ...running, instancia_servidor: "A" }, { ...running })).toBe(null);
});

test("detectServerRestart alerts once when a running job's server instance changes", () => {
  const prev = { ...running, instancia_servidor: "A" };
  const next = { ...running, instancia_servidor: "B" };
  const a = detectServerRestart(prev, next)!;
  expect(a.id).toBe("job-1");
  expect(a.tipo).toBe("proceso run");
  expect(a.outcome).toBe("interrumpido");
  expect(a.message).toMatch(/interrumpi/i);
  expect(a.message).toMatch(/reinici/i);

  // Fires only once: the next poll's "prev" is already instance B, so comparing
  // consecutive polls after the restart never re-detects the same event.
  expect(detectServerRestart(next, { ...next })).toBe(null);
});

test("alertTitle prefixes the tab title by outcome", () => {
  expect(alertTitle("ok", "Tessia")).toBe("✔ Proceso terminado · Tessia");
  expect(alertTitle("error", "Tessia")).toBe("✖ Proceso con error · Tessia");
  expect(alertTitle("cancelado", "Tessia")).toBe("■ Proceso cancelado · Tessia");
});
