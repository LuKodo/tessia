import { describe, expect, it } from "vitest";
import { healthDetail, healthState, notaTecnicaDetail, notaTecnicaState, relativeAge } from "../../src/utils/health-model";

// Ported from webui-src/src/healthModel.test.js (same cases and expectations).
describe("healthState", () => {
  it("maps an available service to success", () => {
    expect(healthState({ disponible: true })).toEqual({ key: "ok", color: "success", icon: "ok", label: "disponible" });
  });

  it("maps an unavailable service to error", () => {
    expect(healthState({ disponible: false })).toEqual({ key: "down", color: "error", icon: "down", label: "no disponible" });
  });

  it("is neutral while the state is unknown", () => {
    expect(healthState(null)).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
    expect(healthState({})).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
    expect(healthState({ disponible: "yes" })).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
  });
});

describe("healthDetail", () => {
  it("shows ping for MySQL and TCP reach for the rest", () => {
    expect(healthDetail({ nombre: "MySQL", disponible: true, latencia_ms: 3 })).toBe("ping · 3 ms");
    expect(healthDetail({ nombre: "Mutual Ser", disponible: true, latencia_ms: 41 })).toBe("alcance TCP · 41 ms");
  });

  it("falls back for an unavailable service", () => {
    expect(healthDetail({ nombre: "X", disponible: false, detalle: "timeout" })).toBe("timeout");
    expect(healthDetail({ nombre: "X", disponible: false })).toBe("no disponible");
    expect(healthDetail({ nombre: "X", disponible: false, detalle: "" })).toBe("no disponible");
  });
});

const DAY = 24 * 60 * 60 * 1000;
const NOW = new Date("2026-10-01T12:00:00Z");
const ago = (ms: number) => new Date(NOW.getTime() - ms).toISOString();

describe("relativeAge", () => {
  it("says hoy for the same day and for future dates", () => {
    expect(relativeAge(ago(0), NOW)).toBe("hoy");
    expect(relativeAge(ago(23 * 60 * 60 * 1000), NOW)).toBe("hoy");
    expect(relativeAge(ago(-3 * DAY), NOW)).toBe("hoy");
  });

  it("says ayer after one day and counts days up to 29", () => {
    expect(relativeAge(ago(1 * DAY), NOW)).toBe("ayer");
    expect(relativeAge(ago(2 * DAY), NOW)).toBe("hace 2 días");
    expect(relativeAge(ago(29 * DAY), NOW)).toBe("hace 29 días");
  });

  it("switches to months at 30 days with correct plurals", () => {
    expect(relativeAge(ago(30 * DAY), NOW)).toBe("hace 1 mes");
    expect(relativeAge(ago(31 * DAY), NOW)).toBe("hace 1 mes");
    expect(relativeAge(ago(60 * DAY), NOW)).toBe("hace 2 meses");
    expect(relativeAge(ago(330 * DAY), NOW)).toBe("hace 11 meses");
    expect(relativeAge(ago(364 * DAY), NOW)).toBe("hace 11 meses");
  });

  it("switches to years at 365 days with correct plurals", () => {
    expect(relativeAge(ago(365 * DAY), NOW)).toBe("hace 1 año");
    expect(relativeAge(ago(729 * DAY), NOW)).toBe("hace 1 año");
    expect(relativeAge(ago(730 * DAY), NOW)).toBe("hace 2 años");
  });

  it("returns an empty string for missing or invalid input", () => {
    expect(relativeAge("", NOW)).toBe("");
    expect(relativeAge(undefined, NOW)).toBe("");
    expect(relativeAge(null, NOW)).toBe("");
    expect(relativeAge("not a date", NOW)).toBe("");
  });
});

describe("notaTecnicaState", () => {
  it("is informational: only exists decides the color", () => {
    expect(notaTecnicaState({ exists: true })).toEqual({ key: "ok", color: "success", icon: "ok", label: "cargada" });
    expect(notaTecnicaState({ exists: false })).toEqual({ key: "down", color: "error", icon: "down", label: "no cargada" });
    expect(notaTecnicaState(null)).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
    expect(notaTecnicaState({})).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
  });
});

describe("notaTecnicaDetail", () => {
  it("shows the update date with its relative age", () => {
    const fmt = (s?: string) => `<${s}>`;
    const iso = ago(3 * DAY);
    expect(notaTecnicaDetail({ exists: true, modified_at: iso }, fmt, NOW)).toBe(`Actualizada: <${iso}> · hace 3 días`);
    expect(notaTecnicaDetail({ exists: true, modified_at: ago(0) }, fmt, NOW)).toBe(`Actualizada: <${ago(0)}> · hoy`);
  });

  it("falls back when the date is missing or the file is absent", () => {
    const fmt = (s?: string) => (s ? `<${s}>` : "");
    expect(notaTecnicaDetail({ exists: true }, fmt, NOW)).toBe("fecha no disponible");
    expect(notaTecnicaDetail({ exists: true, modified_at: "garbage" }, () => "", NOW)).toBe("fecha no disponible");
    expect(notaTecnicaDetail({ exists: false }, fmt, NOW)).toBe("No hay archivo compartido");
  });
});
