// Pure model for the "Preparación del convenio" strip in ConsolaProceso. The input is
// job.convenio (serve_jobs.go's jobConvenio). It returns everything the markup needs, or
// null when the job has no convenio info (the strip is then hidden).
import type { JobConvenio } from "../types/api";

const TITLE = "Preparación del convenio";
const ESTADOS: readonly string[] = ["preparando", "ok", "error"];

/** `estado` stays a string: an unknown value hides the strip instead of failing. */
export type ConvenioInput = Partial<Omit<JobConvenio, "estado">> & { estado?: string };

/** Color names (badge / progress). */
export type ConvenioChipColor = "success" | "warning" | "neutral";
export type ConvenioBarColor = "success" | "error" | "primary";

export interface ConvenioChip {
  key: string;
  label: string;
  color: ConvenioChipColor;
}

export interface ConvenioStrip {
  estado: JobConvenio["estado"];
  title: string;
  determinate: boolean;
  /** 0-100, or null while there is no honest percentage. */
  pct: number | null;
  text: string;
  color: ConvenioBarColor;
  message: string;
  chips: ConvenioChip[];
  aria: { label: string; min: number; max: number; now?: number; text: string };
}

const clampPct = (evaluadas: number | undefined, total: number): number => {
  if (!(total > 0)) return 0;
  return Math.min(100, Math.max(0, Math.round((100 * (evaluadas || 0)) / total)));
};

const evaluatedText = (evaluadas: number | undefined, total: number): string =>
  `Evaluadas ${evaluadas || 0} de ${total} ${total === 1 ? "factura" : "facturas"}`;

export function convenioStrip(c: ConvenioInput | null | undefined): ConvenioStrip | null {
  if (!c || !c.estado || !ESTADOS.includes(c.estado)) return null;
  const estado = c.estado as JobConvenio["estado"];
  const total = c.total || 0;
  const base = { estado, title: TITLE, chips: [] as ConvenioChip[], message: "" };

  let strip: Omit<ConvenioStrip, "aria">;
  if (estado === "ok") {
    strip = {
      ...base,
      determinate: true,
      pct: 100,
      text: "Convenio preparado",
      color: "success",
      chips: [
        { key: "preparadas", label: `Preparadas: ${c.preparadas || 0}`, color: "success" },
        { key: "excluidas", label: `Excluidas: ${c.excluidas || 0}`, color: "neutral" },
        { key: "revision_manual", label: `Revisión manual: ${c.revision_manual || 0}`, color: "warning" },
      ],
    };
  } else if (estado === "error") {
    strip = {
      ...base,
      determinate: true,
      pct: clampPct(c.evaluadas, total),
      text: "No se pudo preparar el convenio",
      color: "error",
      message: c.mensaje || "La preparación terminó con un error; revisa la consola.",
    };
  } else if (total > 0) {
    strip = { ...base, determinate: true, pct: clampPct(c.evaluadas, total), text: evaluatedText(c.evaluadas, total), color: "primary" };
  } else {
    // The walk has not started (CSV and nota técnica still loading): no honest percentage.
    strip = { ...base, determinate: false, pct: null, text: "Leyendo el CSV y la nota técnica…", color: "primary" };
  }
  const aria: ConvenioStrip["aria"] = { label: TITLE, min: 0, max: 100, text: strip.text };
  if (strip.determinate && strip.pct !== null) aria.now = strip.pct;
  return { ...strip, aria };
}
