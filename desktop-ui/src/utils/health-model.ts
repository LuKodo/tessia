// Pure view-model for the service health cards (/api/health rows) and the nota técnica card
// (/api/nota-tecnica, read-only from Redis in the Rust port). Port of webui-src/src/healthModel.js.
// The one-line aggregate of the app bar lives in health.ts (summarizeHealth).

import type { NotaTecnicaStatus, ServiceHealth } from "../types/api";

export type HealthCardKey = "ok" | "down" | "unknown";

export interface HealthCardState {
  key: HealthCardKey;
  /** Color name. */
  color: "success" | "error" | "neutral";
  /** Icon key; the paths live with the component. */
  icon: HealthCardKey;
  label: string;
}

const STATES: Record<HealthCardKey, HealthCardState> = {
  ok: { key: "ok", color: "success", icon: "ok", label: "disponible" },
  down: { key: "down", color: "error", icon: "down", label: "no disponible" },
  unknown: { key: "unknown", color: "neutral", icon: "unknown", label: "" },
};

/**
 * Classifies a service row. Anything whose `disponible` is not a boolean (null row, missing
 * field) is "unknown" and rendered neutral.
 */
export function healthState(s: { disponible?: unknown } | null | undefined): HealthCardState {
  if (s?.disponible === true) return { ...STATES.ok };
  if (s?.disponible === false) return { ...STATES.down };
  return { ...STATES.unknown };
}

/** Name the API gives the database row; only that row is probed with a ping instead of a TCP dial. */
export const DATABASE_SERVICE_NAME = "PostgreSQL";

/** Secondary line of a card. */
export function healthDetail(s: Pick<ServiceHealth, "nombre" | "disponible"> & Partial<ServiceHealth>): string {
  return s.disponible
    ? `${s.nombre === DATABASE_SERVICE_NAME ? "ping" : "alcance TCP"} · ${s.latencia_ms} ms`
    : s.detalle || "no disponible";
}

// --- Nota técnica card -------------------------------------------------------

/**
 * Informational only: colour depends on whether a Nota Técnica is published in Redis, never on how
 * old it is (the operator decides if it is current).
 */
export function notaTecnicaState(s: Partial<NotaTecnicaStatus> | null | undefined): HealthCardState {
  if (s?.exists === true) return { key: "ok", color: "success", icon: "ok", label: "cargada" };
  if (s?.exists === false) return { key: "down", color: "error", icon: "down", label: "sin Nota Técnica" };
  return { ...STATES.unknown };
}

/** Short version for display: the API's `file_name` (12 hex), else the first 12 characters of `version`. */
export function notaTecnicaVersion(s: Partial<NotaTecnicaStatus> | null | undefined): string {
  return s?.file_name || s?.version?.slice(0, 12) || "";
}

/** Row count in es-CO ("12.385 filas"); "" when the API did not send a numeric `filas`. */
export function notaTecnicaFilas(s: Partial<NotaTecnicaStatus> | null | undefined): string {
  const raw = s?.filas;
  const n = typeof raw === "number" ? raw : typeof raw === "string" && raw.trim() !== "" ? Number(raw) : NaN;
  if (!Number.isFinite(n)) return "";
  return `${n.toLocaleString("es-CO")} ${n === 1 ? "fila" : "filas"}`;
}

/** Secondary line of the card: "Versión ab12cd34ef56 · 12.385 filas". */
export function notaTecnicaDetail(s: Partial<NotaTecnicaStatus> | null | undefined): string {
  if (!s?.exists) return "No hay una Nota Técnica publicada en Redis";
  const version = notaTecnicaVersion(s);
  return [version && `Versión ${version}`, notaTecnicaFilas(s)].filter(Boolean).join(" · ") || "versión no disponible";
}
