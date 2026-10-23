import { describe, expect, it } from "vitest";
import {
  DATABASE_SERVICE_NAME,
  healthDetail,
  healthState,
  notaTecnicaDetail,
  notaTecnicaFilas,
  notaTecnicaState,
  notaTecnicaVersion,
} from "../../src/utils/health-model";

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
  it("shows ping for PostgreSQL and TCP reach for the rest", () => {
    expect(DATABASE_SERVICE_NAME).toBe("PostgreSQL");
    expect(healthDetail({ nombre: "PostgreSQL", disponible: true, latencia_ms: 3 })).toBe("ping · 3 ms");
    expect(healthDetail({ nombre: "MySQL", disponible: true, latencia_ms: 3 })).toBe("alcance TCP · 3 ms");
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

describe("notaTecnicaState", () => {
  it("is informational: only exists decides the color", () => {
    expect(notaTecnicaState({ exists: true })).toEqual({ key: "ok", color: "success", icon: "ok", label: "cargada" });
    expect(notaTecnicaState({ exists: false })).toEqual({ key: "down", color: "error", icon: "down", label: "sin Nota Técnica" });
    expect(notaTecnicaState(null)).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
    expect(notaTecnicaState({})).toEqual({ key: "unknown", color: "neutral", icon: "unknown", label: "" });
  });
});

describe("notaTecnicaVersion", () => {
  it("prefers file_name and falls back to the first 12 characters of version", () => {
    expect(notaTecnicaVersion({ file_name: "ab12cd34ef56", version: "ab12cd34ef56".padEnd(64, "0") })).toBe("ab12cd34ef56");
    expect(notaTecnicaVersion({ version: "ab12cd34ef56".padEnd(64, "0") })).toBe("ab12cd34ef56");
    expect(notaTecnicaVersion({ exists: true })).toBe("");
    expect(notaTecnicaVersion(null)).toBe("");
  });
});

describe("notaTecnicaFilas", () => {
  it("formats the row count as es-CO, from a number or the string the API sends", () => {
    expect(notaTecnicaFilas({ filas: "12385" })).toBe("12.385 filas");
    expect(notaTecnicaFilas({ filas: 12385 })).toBe("12.385 filas");
    expect(notaTecnicaFilas({ filas: "1" })).toBe("1 fila");
  });

  it("is empty when filas is missing or not numeric", () => {
    expect(notaTecnicaFilas({ exists: true })).toBe("");
    expect(notaTecnicaFilas({ filas: "" })).toBe("");
    expect(notaTecnicaFilas({ filas: "varias" })).toBe("");
    expect(notaTecnicaFilas(null)).toBe("");
  });
});

describe("notaTecnicaDetail", () => {
  it("shows the short version and the row count", () => {
    expect(notaTecnicaDetail({ exists: true, file_name: "ab12cd34ef56", version: "x", filas: "12385" })).toBe("Versión ab12cd34ef56 · 12.385 filas");
  });

  it("degrades when the version or the rows are missing", () => {
    expect(notaTecnicaDetail({ exists: true, file_name: "ab12cd34ef56" })).toBe("Versión ab12cd34ef56");
    expect(notaTecnicaDetail({ exists: true, filas: "3" })).toBe("3 filas");
    expect(notaTecnicaDetail({ exists: true })).toBe("versión no disponible");
  });

  it("says there is no published Nota Técnica when it does not exist", () => {
    expect(notaTecnicaDetail({ exists: false })).toBe("No hay una Nota Técnica publicada en Redis");
    expect(notaTecnicaDetail(null)).toBe("No hay una Nota Técnica publicada en Redis");
  });
});
