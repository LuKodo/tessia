// Pure view-model for the service health cards (/api/health rows) and the nota técnica card
// (/api/nota-tecnica, read-only from Redis in the Rust port). Port of webui-src/src/healthModel.js. The one-line aggregate of the app
// bar lives in health.ts (summarizeHealth).

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

/** Secondary line of a card. */
export function healthDetail(s: Pick<ServiceHealth, "nombre" | "disponible"> & Partial<ServiceHealth>): string {
  return s.disponible
    ? `${s.nombre === "PostgreSQL" ? "ping" : "alcance TCP"} · ${s.latencia_ms} ms`
    : s.detalle || "no disponible";
}

// --- Nota técnica card -------------------------------------------------------

const DAY_MS = 24 * 60 * 60 * 1000;

function plural(n: number, one: string, many: string): string {
  return `hace ${n} ${n === 1 ? one : many}`;
}

/**
 * How long ago `iso` was, in coarse Spanish units. Elapsed time is floored to whole days (a month
 * is 30 days, a year 365); future dates and anything under 24 h are "hoy". Missing or invalid
 * input yields "".
 */
export function relativeAge(iso: string | null | undefined, now: Date = new Date()): string {
  if (!iso) return "";
  const then = new Date(iso);
  if (Number.isNaN(then.getTime())) return "";
  const days = Math.floor((now.getTime() - then.getTime()) / DAY_MS);
  if (days < 1) return "hoy";
  if (days === 1) return "ayer";
  if (days < 30) return plural(days, "día", "días");
  if (days < 365) return plural(Math.min(Math.floor(days / 30), 11), "mes", "meses");
  return plural(Math.floor(days / 365), "año", "años");
}

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

/** Row count in es-CO ("12.385 filas"); "" when the API did not send it. */
export function notaTecnicaFilas(s: Partial<NotaTecnicaStatus> | null | undefined): string {
  if (typeof s?.total !== "number" || !Number.isFinite(s.total)) return "";
  return `${s.total.toLocaleString("es-CO")} ${s.total === 1 ? "fila" : "filas"}`;
}

/** "Bases faltantes: A, B" when the catalog lacks bases, otherwise "". */
export function notaTecnicaWarning(s: Partial<NotaTecnicaStatus> | null | undefined): string {
  const missing = s?.bases_faltantes ?? [];
  return missing.length > 0 ? `Bases faltantes: ${missing.join(", ")}` : "";
}

/**
 * Secondary lines of the card. Line 1: short version and row count. Line 2: generated date (shown
 * as sent: local time without zone) and published date (UTC, formatted by `fmt`, with its relative
 * age). `fmt` formats an ISO date for display; `now` is injectable for tests.
 */
export function notaTecnicaLines(
  s: Partial<NotaTecnicaStatus> | null | undefined,
  fmt: (iso?: string) => string,
  now: Date = new Date(),
): string[] {
  if (!s?.exists) return ["No hay una Nota Técnica publicada en Redis"];
  const head = [notaTecnicaVersion(s) && `Versión ${notaTecnicaVersion(s)}`, notaTecnicaFilas(s)].filter(Boolean).join(" · ");
  const published = fmt(s.publicado_en);
  const age = published ? relativeAge(s.publicado_en, now) : "";
  const dates = [
    s.generado_en && `Generada: ${s.generado_en}`,
    published && `Publicada: ${published}${age ? ` · ${age}` : ""}`,
  ]
    .filter(Boolean)
    .join(" · ");
  return [head || "versión no disponible", dates || "fecha no disponible"];
}
