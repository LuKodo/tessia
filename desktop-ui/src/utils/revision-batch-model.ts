// Pure logic for the "Revisión manual" page: the reports of several cargues merged into one
// table, its filters, the row selection and the sequential batch retry. No Vue, no DOM: the page
// passes the side effects in as `actions`.
import type { ManualReviewReport, ReportItem } from "../types/api";
import { categoryLabel, filterItems, phaseLabel } from "./informe-model";
import { canRetryValueDifference, runInvoiceRetry } from "./retry-model";
import type { RetryActions, RetryItem, RetryResult } from "./retry-model";

/** The report of one cargue as loaded by the page: either the report or why it failed. */
export interface CargueReport {
  cargue: string;
  report: ManualReviewReport | null;
  error?: string;
}

/** A report item tagged with the cargue it belongs to. */
export type RevisionRow = ReportItem & { cargue: string };

export interface RevisionFilters {
  text: string;
  /** "" = every cargue. */
  cargue: string;
  /** "" = every phase. */
  fase: string;
  /** "" = every category (within what `includeOthers` allows). */
  categoria: string;
  includeOthers: boolean;
}

/** Sentinel of the selects: Reka rejects an empty item value, so "(todas)" maps back to "". */
export const CATEGORY_ALL = "todas";

export function mergeReports(reports: readonly CargueReport[]): RevisionRow[] {
  return reports.flatMap(({ cargue, report }) =>
    Array.isArray(report?.items) ? report.items.map((item) => ({ ...item, cargue })) : [],
  );
}

function matchesText(row: RevisionRow, text: string): boolean {
  const needle = text.trim().toLowerCase();
  if (!needle) return true;
  return [row.factura, row.codigo, row.motivo].some((field) => String(field ?? "").toLowerCase().includes(needle));
}

/** Rows that pass the switch (manual reviews, or everything) and every filter that is set. */
export function filterRows(rows: readonly RevisionRow[], f: RevisionFilters): RevisionRow[] {
  const categoria = f.categoria === CATEGORY_ALL ? "" : f.categoria;
  return filterItems(rows, { includeOthers: f.includeOthers }).filter(
    (row) =>
      (!f.cargue || row.cargue === f.cargue) &&
      (!f.fase || row.fase === f.fase) &&
      (!categoria || row.categoria === categoria) &&
      matchesText(row, f.text),
  );
}

/** Distinct phases of the rows, in first-seen order; a label shared by two phases carries its raw key. */
export function phaseOptions(rows: readonly RevisionRow[]): { value: string; label: string }[] {
  const phases = [...new Set(rows.map((r) => r.fase))];
  const labels = phases.map(phaseLabel);
  return phases.map((value, i) => ({
    value,
    label: labels.filter((l) => l === labels[i]).length > 1 ? `${labels[i]} (${value})` : labels[i]!,
  }));
}

export function categoryOptions(includeOthers: boolean): { value: string; label: string }[] {
  const values = includeOthers ? ["revision_manual", "excluida", "falla"] : ["revision_manual"];
  return values.map((value) => ({ value, label: categoryLabel(value) }));
}

/** What the page header shows for the chosen cargue ("" = all): summed counts, warnings and load errors. */
export function cargueScope(reports: readonly CargueReport[], cargue: string) {
  const inScope = reports.filter((r) => !cargue || r.cargue === cargue);
  const conteos = { revision_manual: 0, excluidas: 0, fallas: 0 };
  for (const { report } of inScope) {
    conteos.revision_manual += report?.conteos?.revision_manual || 0;
    conteos.excluidas += report?.conteos?.excluidas || 0;
    conteos.fallas += report?.conteos?.fallas || 0;
  }
  return {
    conteos,
    incompleto: inScope.some((r) => !!r.report?.incompleto),
    errors: inScope.filter((r) => r.error).map((r) => ({ cargue: r.cargue, error: r.error! })),
  };
}

function incompleteOf(reports: readonly CargueReport[], cargue: string): boolean {
  return !!reports.find((r) => r.cargue === cargue)?.report?.incompleto;
}

/** Retry eligibility stays the legacy rule, evaluated against the report of the row's own cargue. */
export function isRetryable(row: RevisionRow, reports: readonly CargueReport[]): boolean {
  return canRetryValueDifference(row, incompleteOf(reports, row.cargue));
}

export function retryableRows(rows: readonly RevisionRow[], reports: readonly CargueReport[]): RevisionRow[] {
  return rows.filter((row) => isRetryable(row, reports));
}

export function rowKey(row: RevisionRow): string {
  return [row.cargue, row.factura, row.fase].join("\u0000");
}

export function toggleRow(selected: ReadonlySet<string>, key: string, checked: boolean): Set<string> {
  const next = new Set(selected);
  if (checked) next.add(key);
  else next.delete(key);
  return next;
}

/** Selects (or clears) every retryable visible row; selected rows hidden by a filter are left alone. */
export function toggleAllVisible(
  selected: ReadonlySet<string>,
  visible: readonly RevisionRow[],
  reports: readonly CargueReport[],
  checked: boolean,
): Set<string> {
  const next = new Set(selected);
  for (const row of retryableRows(visible, reports)) {
    if (checked) next.add(rowKey(row));
    else next.delete(rowKey(row));
  }
  return next;
}

export function allVisibleState(
  selected: ReadonlySet<string>,
  visible: readonly RevisionRow[],
  reports: readonly CargueReport[],
): "none" | "some" | "all" {
  const keys = retryableRows(visible, reports).map(rowKey);
  const chosen = keys.filter((k) => selected.has(k)).length;
  if (chosen === 0) return "none";
  return chosen === keys.length ? "all" : "some";
}

// --- sequential batch retry --------------------------------------------------------------

export interface BatchTarget {
  cargue: string;
  item: RetryItem;
}

export interface BatchRowResult {
  cargue: string;
  factura: string;
  result: RetryResult;
}

export interface BatchResult {
  status: "ignored" | "cancelled" | "done";
  results: BatchRowResult[];
}

export interface BatchActions
  extends Pick<
    RetryActions,
    "isBusy" | "setBusy" | "currentJob" | "history" | "setStatus" | "setError" | "setOutput" | "delay" | "maxAttempts" | "maxHistoryAttempts"
  > {
  isEligible(target: BatchTarget): boolean;
  /** Asked once for the whole batch, with the number of invoices about to be retried. */
  confirm(count: number): Promise<boolean>;
  submit(cargue: string, factura: string): ReturnType<RetryActions["submit"]>;
  /** Called once after the batch, only when at least one retry was submitted. */
  refresh(): Promise<void>;
  onProgress?(index: number, total: number, target: BatchTarget): void;
}

const messageOf = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * Retries the targets one after another (the backend runs one job at a time and a retry takes a
 * single invoice) after one confirmation for all of them. A row that fails does not stop the
 * rest; every row reports its own result and the report is refreshed once at the end.
 */
export async function runBatchRetry(targets: readonly BatchTarget[], actions: BatchActions): Promise<BatchResult> {
  const seen = new Set<string>();
  const batch = targets.filter((t) => {
    const key = `${t.cargue}\u0000${t.item.factura}`;
    if (seen.has(key) || !actions.isEligible(t)) return false;
    seen.add(key);
    return true;
  });
  if (actions.isBusy() || batch.length === 0) return { status: "ignored", results: [] };

  actions.setBusy(true);
  const results: BatchRowResult[] = [];
  const failures: string[] = [];
  let log = "";
  let submitted = false;
  try {
    if (!(await actions.confirm(batch.length))) return { status: "cancelled", results };
    actions.setError("");
    actions.setOutput("");
    for (const [index, target] of batch.entries()) {
      const factura = target.item.factura ?? "";
      actions.onProgress?.(index + 1, batch.length, target);
      // The batch owns the busy flag and the confirmation, so each invoice runs through the
      // single-invoice flow with both already settled.
      const result = await runInvoiceRetry(target.item, {
        isBusy: () => false,
        setBusy: () => {},
        isEligible: () => true,
        confirm: async () => true,
        submit: async () => {
          submitted = true;
          return actions.submit(target.cargue, factura);
        },
        currentJob: actions.currentJob,
        history: actions.history,
        refresh: async () => {},
        setStatus: (value) => actions.setStatus(value && `(${index + 1}/${batch.length}) ${factura}: ${value}`),
        setError: () => {},
        setOutput: (value) => actions.setOutput(`${log}${value ? `[${factura}]\n${value}` : ""}`),
        delay: actions.delay,
        maxAttempts: actions.maxAttempts,
        maxHistoryAttempts: actions.maxHistoryAttempts,
      });
      results.push({ cargue: target.cargue, factura, result });
      if ("output" in result && result.output) log += `[${factura}]\n${result.output}\n`;
      if ("error" in result && result.error) failures.push(`${target.cargue} / ${factura}: ${result.error}`);
    }
    actions.setStatus("");
    return { status: "done", results };
  } finally {
    if (submitted) {
      try {
        await actions.refresh();
        if (failures.length === 0) actions.setStatus("Informe actualizado.");
      } catch (error) {
        failures.push(`No se pudo actualizar el informe: ${messageOf(error)}`);
      }
    }
    if (failures.length) actions.setError(failures.join("\n"));
    actions.setBusy(false);
  }
}
