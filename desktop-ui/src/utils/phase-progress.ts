// Pure model of the per-phase live cards in the process log card. A phase is one entry of
// `job.fases` (serve_jobs.go's jobPhaseProgress): the counters of the invoices that are in the phase
// (`en_curso`) and of those that already left it (`terminadas`, `revision_manual`, `excluidas`,
// `con_errores`). Percent = settled / (in progress + settled).
import type { JobPhaseProgress } from "../types/api";

export type PhaseColor = "primary" | "success" | "warning" | "neutral" | "error";

export interface PhaseBadge {
  key: "en_curso" | "terminadas" | "revision_manual" | "excluidas" | "con_errores";
  label: string;
  color: PhaseColor;
}

export interface PhaseCard {
  key: string;
  name: string;
  /** 0-100. */
  pct: number;
  /** "settled/total". */
  text: string;
  /** Busy time of the phase ("3 min 12 s"), empty when it never ran. */
  time: string;
  /** Nothing has entered the phase yet. */
  queued: boolean;
  /** Bar color: error with failures, success once nothing is in flight, primary otherwise. */
  color: PhaseColor;
  badges: PhaseBadge[];
}

const n = (v: number | undefined): number => v || 0;

export function phaseSettled(f: JobPhaseProgress): number {
  return n(f.terminadas) + n(f.revision_manual) + n(f.excluidas) + n(f.con_errores);
}

export function phaseTotal(f: JobPhaseProgress): number {
  return n(f.en_curso) + phaseSettled(f);
}

export function phasePct(f: JobPhaseProgress): number {
  const total = phaseTotal(f);
  return total ? Math.round((100 * phaseSettled(f)) / total) : 0;
}

export function phaseDoneText(f: JobPhaseProgress): string {
  return `${phaseSettled(f)}/${phaseTotal(f)}`;
}

// Friendly dashboard names keyed by the pipeline phase key (job.fases[].fase). The CLI labels
// (`etiqueta`, e.g. "derechos validar") feed terminal output and logs.
const PHASE_NAMES: Record<string, string> = {
  derechos: "Validación de Derechos",
  thefactory: "Descarga de Factura TF",
  fev: "Validación FEV",
  rips: "Generación RIPS (JSON)",
  cuv: "Generación CUV (Ministerio)",
  soporte: "Generación de Soportes (XML, JSON)",
  conciliar: "Conciliación de Factura",
  radicacion: "Radicación",
};

export function phaseName(f: Pick<JobPhaseProgress, "fase"> & Partial<JobPhaseProgress>): string {
  return PHASE_NAMES[f.fase] || f.etiqueta || f.fase || "";
}

/** Busy milliseconds as "42 s", "3 min 12 s" or "1 h 3 min"; empty below one second or when unknown. */
export function formatPhaseTime(ms: number | undefined): string {
  if (!ms || ms < 1000) return "";
  const total = Math.round(ms / 1000);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  if (h > 0) return `${h} h ${m} min`;
  if (m > 0) return `${m} min ${s} s`;
  return `${s} s`;
}

function badges(f: JobPhaseProgress, doneLabel: string): PhaseBadge[] {
  const all: PhaseBadge[] = [
    { key: "en_curso", label: `${n(f.en_curso)} en curso`, color: "primary" },
    { key: "terminadas", label: `${n(f.terminadas)} ${doneLabel}`, color: "success" },
    { key: "revision_manual", label: `${n(f.revision_manual)} revisión`, color: "warning" },
    { key: "excluidas", label: `${n(f.excluidas)} excluidas`, color: "neutral" },
    { key: "con_errores", label: `${n(f.con_errores)} con error`, color: "error" },
  ];
  return all.filter((b) => n(f[b.key]) > 0);
}

function card(f: JobPhaseProgress, doneLabel = "ok"): PhaseCard {
  const total = phaseTotal(f);
  const color: PhaseColor = n(f.con_errores) > 0 ? "error" : total > 0 && n(f.en_curso) === 0 ? "success" : "primary";
  return {
    key: f.fase,
    name: phaseName(f),
    pct: phasePct(f),
    text: phaseDoneText(f),
    time: formatPhaseTime(f.tiempo_ms),
    queued: total === 0,
    color,
    badges: badges(f, doneLabel),
  };
}

/**
 * One card per phase, in the order the job reports them. The done badge of the radicación phase
 * reads "N radicadas". `radicadas` is the job's confirmed radicación total: if the job has it but no
 * radicación phase entry, a radicación card is added at the end.
 */
export function phaseCards(fases: JobPhaseProgress[] | undefined, radicadas?: number): PhaseCard[] {
  const phases = fases ?? [];
  const cards = phases.map((f) => card(f, f.fase === "radicacion" ? "radicadas" : "ok"));
  if (radicadas && radicadas > 0 && !phases.some((f) => f.fase === "radicacion")) {
    cards.push(card({ fase: "radicacion", etiqueta: "radicacion", en_curso: 0, terminadas: radicadas, revision_manual: 0, excluidas: 0, con_errores: 0 }, "radicadas"));
  }
  return cards;
}
