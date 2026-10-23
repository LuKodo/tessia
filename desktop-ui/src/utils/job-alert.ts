// Pure logic for the "a process finished" alert. No Vue, no DOM: useJobAlert feeds it
// consecutive /api/jobs/current values.
import type { JobSnapshot } from "../types/api";

/** An observation of /api/jobs/current; every field may be missing (idle snapshot, first poll). */
export type JobObservation = Partial<JobSnapshot>;

export type AlertOutcome = "ok" | "error" | "cancelado" | "interrumpido";

export interface JobAlert {
  id: string;
  tipo: string;
  outcome: AlertOutcome;
  message: string;
  durationText: string;
}

// The server (serve_jobs.go) sets error to "cancelado por el operador" when the operator
// cancels; a bare "context canceled" can also surface. Match both case-insensitively so a
// cancellation is never reported as a failure.
const CANCEL_RE = /cancelad[oa]|cancell?ed/i;

const OUTCOME_TEXT = {
  ok: "Proceso terminado",
  error: "Proceso terminado con error",
  cancelado: "Proceso cancelado",
} as const;

const TITLE_TEXT: Record<AlertOutcome, string> = {
  ok: "✔ Proceso terminado",
  error: "✖ Proceso con error",
  cancelado: "■ Proceso cancelado",
  interrumpido: "⚠ Servicio reiniciado",
};

const RESTART_MESSAGE = "La ejecución se interrumpió inesperadamente: el servicio se reinició.";

// Go serialises an unset time.Time as year 1; treat anything before 2000 as unknown.
function parseTime(s: string | undefined): number | null {
  if (!s) return null;
  const t = Date.parse(s);
  if (Number.isNaN(t) || new Date(t).getUTCFullYear() < 2000) return null;
  return t;
}

/**
 * Renders the span between two ISO timestamps as "42 s", "2 min 5 s" or "1 h 3 min".
 * Returns "" when either end is unknown or the span is negative.
 */
export function formatDuration(startISO: string | undefined, endISO: string | undefined): string {
  const a = parseTime(startISO);
  const b = parseTime(endISO);
  if (a === null || b === null || b < a) return "";
  const total = Math.round((b - a) / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min ${s} s`;
  return `${s} s`;
}

function outcomeOf(job: JobObservation): "ok" | "error" | "cancelado" {
  if (!job.error) return "ok";
  return CANCEL_RE.test(job.error) ? "cancelado" : "error";
}

/**
 * Compares two consecutive observations of the (single) current job and returns an alert
 * when `next` is a job that finished since `prev`, otherwise null. The very first
 * observation (prev == null) never alerts, so reloading the page with an old finished job
 * stays quiet.
 */
export function detectJobFinish(prev: JobObservation | null | undefined, next: JobObservation | null | undefined): JobAlert | null {
  if (!prev || !next || !next.id || next.corriendo) return null;
  const finishedNow = prev.id === next.id ? prev.corriendo === true : true;
  if (!finishedNow) return null;
  const outcome = outcomeOf(next);
  return {
    id: next.id,
    tipo: next.tipo || "",
    outcome,
    message: next.tipo ? `${OUTCOME_TEXT[outcome]}: ${next.tipo}` : OUTCOME_TEXT[outcome],
    durationText: formatDuration(next.iniciado_en, next.terminado_en),
  };
}

/**
 * Compares two consecutive /api/jobs/current observations and flags the case where a job
 * was seen running (`prev.corriendo === true`) and the next poll reports a different
 * `instancia_servidor` (the tessia serve process restarted) without ever recording that job
 * as finished. The server always sets `instancia_servidor`, even on the "no job" snapshot,
 * so a changed value reliably means a new process. Fires at most once per restart: once
 * `next` becomes the current observation, a later poll compares it as `prev` with the new
 * instance id already in place, so the same event is never re-detected.
 */
export function detectServerRestart(prev: JobObservation | null | undefined, next: JobObservation | null | undefined): JobAlert | null {
  if (!prev || !next || prev.corriendo !== true) return null;
  const prevInstance = prev.instancia_servidor;
  const nextInstance = next.instancia_servidor;
  if (!prevInstance || !nextInstance || prevInstance === nextInstance) return null;
  return {
    id: prev.id ?? "",
    tipo: prev.tipo || "",
    outcome: "interrumpido",
    message: RESTART_MESSAGE,
    durationText: "",
  };
}

/** Browser-tab title shown while the tab is in the background. */
export function alertTitle(outcome: AlertOutcome, baseTitle: string): string {
  return `${TITLE_TEXT[outcome] || TITLE_TEXT.ok} · ${baseTitle}`;
}
