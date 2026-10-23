// Retry of one invoice from the manual review report (POST /api/cargues/{id}/reintentar):
// eligibility, and the flow that submits the retry job and waits for its outcome. No Vue, no
// DOM: the Revisión manual page passes the side effects in as `actions`.
import type { JobHistoryEntry, JobSnapshot, ReportItem } from "../types/api";

/** `categoria` stays a string: only a "revision_manual" one is ever retryable. */
export type RetryItem = Partial<Omit<ReportItem, "categoria">> & { categoria?: string };
type JobObservation = Partial<JobSnapshot>;
type HistoryObservation = Partial<JobHistoryEntry>;

export type RetryResult =
  | { status: "ignored" }
  | { status: "cancelled" }
  | { status: "completed"; output: string }
  | { status: "failed"; error: string; output: string }
  | { status: "unknown"; error: string }
  | { status: "error"; error: string };

export interface RetryActions {
  isBusy(): boolean;
  setBusy(value: boolean): void;
  isEligible(item: RetryItem): boolean;
  confirm(item: RetryItem): Promise<boolean>;
  submit(item: RetryItem): Promise<JobObservation | null | undefined>;
  currentJob(): Promise<JobObservation | null | undefined>;
  history(): Promise<HistoryObservation[] | null | undefined>;
  refresh(): Promise<void>;
  setStatus(value: string): void;
  setError(value: string): void;
  setOutput(value: string): void;
  /** Test seams: wait between polls and attempt budgets. */
  delay?: (ms: number) => Promise<void>;
  maxAttempts?: number;
  maxHistoryAttempts?: number;
}

/** Only a complete report with an exact VALOR_DIFERENTE manual review of the fev_valor phase is retryable. */
export function canRetryValueDifference(item: RetryItem | null | undefined, incomplete: boolean | undefined): boolean {
  const reason = String(item?.motivo || "").trim();
  const exactReason = item?.codigo === "VALOR_DIFERENTE" || /^VALOR_DIFERENTE(?:\s|$)/.test(reason);
  return !incomplete && item?.fase === "fev_valor" && item?.categoria === "revision_manual" && exactReason;
}

const wait = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error));

type Settled = Extract<RetryResult, { status: "completed" | "failed" }>;

function terminalJob(job: { error?: string; salida?: string; salida_final?: string }, output = ""): Settled {
  const tail = job.salida_final || job.salida || output;
  return job.error ? { status: "failed", error: job.error, output: tail } : { status: "completed", output: tail };
}

interface ResolveOptions {
  currentJob: RetryActions["currentJob"];
  history: RetryActions["history"];
  setOutput: RetryActions["setOutput"];
  delay: (ms: number) => Promise<void>;
  maxAttempts: number;
  maxHistoryAttempts: number;
}

// Follows the retry job through /api/jobs/current while it is the current job and falls
// back to /api/jobs/history once another job replaced it.
async function resolveRetryJob(id: string, { currentJob, history, setOutput, delay, maxAttempts, maxHistoryAttempts }: ResolveOptions): Promise<Settled | Extract<RetryResult, { status: "unknown" }>> {
  let lastError = "";
  let historyAttempts = 0;
  for (let attempt = 0; attempt < maxAttempts; attempt++) {
    try {
      const current = await currentJob();
      if (current?.id === id) {
        setOutput(current.salida || "");
        if (!current.corriendo) return terminalJob(current, current.salida || "");
        await delay(1200);
        continue;
      }
    } catch (error) {
      lastError = messageOf(error);
    }

    try {
      const entries = await history();
      const finished = Array.isArray(entries) ? entries.find((entry) => entry?.id === id) : null;
      if (finished) {
        const result = terminalJob(finished);
        setOutput(result.output);
        return result;
      }
    } catch (error) {
      lastError = messageOf(error);
    }
    historyAttempts++;
    if (historyAttempts >= maxHistoryAttempts) break;
    if (attempt + 1 < maxAttempts) await delay(500);
  }
  return {
    status: "unknown",
    error: `No se pudo confirmar el resultado del reintento${lastError ? `: ${lastError}` : "."}`,
  };
}

export async function runInvoiceRetry(item: RetryItem, actions: RetryActions): Promise<RetryResult> {
  const { isBusy, setBusy, isEligible, confirm, submit, currentJob, history, refresh, setStatus, setError, setOutput } = actions;
  if (isBusy() || !isEligible(item)) return { status: "ignored" };

  setBusy(true);
  let submitted = false;
  let result: RetryResult = { status: "cancelled" };
  setError("");
  setOutput("");
  try {
    if (!(await confirm(item))) return result;
    setStatus("Programando reintento…");
    const job = await submit(item);
    submitted = true;
    if (!job?.id) {
      result = { status: "unknown", error: "El servidor no devolvió el identificador del reintento." };
    } else {
      setStatus("Reconciliando y generando RIPS/CUV…");
      setOutput(job.salida || "");
      result = await resolveRetryJob(job.id, {
        currentJob,
        history,
        setOutput,
        delay: actions.delay || wait,
        maxAttempts: actions.maxAttempts || 900,
        maxHistoryAttempts: actions.maxHistoryAttempts || 6,
      });
    }
    if (result.status === "completed") setStatus("Reintento terminado. Actualizando el informe…");
    else {
      setStatus("");
      if ("error" in result && result.error) setError(result.error);
    }
    return result;
  } catch (error) {
    const message = messageOf(error);
    result = submitted ? { status: "unknown", error: `No se pudo confirmar el resultado: ${message}` } : { status: "error", error: message };
    setStatus("");
    setError(result.error);
    return result;
  } finally {
    if (submitted) {
      try {
        await refresh();
        if (result.status === "completed") setStatus("Informe actualizado.");
      } catch (error) {
        const refreshError = messageOf(error);
        const prior = "error" in result && result.error ? `${result.error} ` : "";
        setError(`${prior}No se pudo actualizar el informe: ${refreshError}`);
        setStatus("");
      }
    }
    setBusy(false);
  }
}
