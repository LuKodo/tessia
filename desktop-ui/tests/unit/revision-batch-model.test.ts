import { expect, test } from "vitest";
import type { ManualReviewReport, ReportItem } from "../../src/types/api";
import {
  CATEGORY_ALL,
  allVisibleState,
  categoryOptions,
  cargueScope,
  filterRows,
  mergeReports,
  phaseOptions,
  retryableRows,
  rowKey,
  runBatchRetry,
  toggleAllVisible,
  toggleRow,
  type BatchActions,
  type CargueReport,
} from "../../src/utils/revision-batch-model";

const item = (over: Partial<ReportItem> = {}): ReportItem => ({
  factura: "EVS1",
  fase: "fev_valor",
  categoria: "revision_manual",
  codigo: "VALOR_DIFERENTE",
  motivo: "VALOR_DIFERENTE XML=1 convenio=2",
  actualizado_en: "",
  ...over,
});

const report = (items: ReportItem[], over: Partial<ManualReviewReport> = {}): ManualReviewReport => ({
  cargue: "",
  generado_en: "",
  incompleto: false,
  conteos: { revision_manual: items.filter((i) => i.categoria === "revision_manual").length, excluidas: 0, fallas: 0 },
  items,
  ...over,
});

const reports: CargueReport[] = [
  {
    cargue: "A",
    report: report([
      item({ factura: "A1" }),
      item({ factura: "A2", fase: "derechos", codigo: "", motivo: "sin derechos" }),
      item({ factura: "A3", categoria: "excluida", fase: "fev_ajuste", codigo: "", motivo: "Monto excedido" }),
    ]),
  },
  { cargue: "B", report: report([item({ factura: "B1" }), item({ factura: "B2", categoria: "falla", fase: "rips", motivo: "boom" })]) },
  { cargue: "C", report: report([item({ factura: "C1" })], { incompleto: true }) },
  { cargue: "D", report: null, error: "informe no disponible" },
];

const merged = () => mergeReports(reports);
const facturas = (rows: { factura: string }[]) => rows.map((r) => r.factura);
const noFilter = { text: "", cargue: "", fase: "", categoria: "", includeOthers: false };

test("mergeReports tags every row with its cargue, keeps order and skips failed cargues", () => {
  const rows = merged();
  expect(rows.map((r) => `${r.cargue}:${r.factura}`)).toEqual(["A:A1", "A:A2", "A:A3", "B:B1", "B:B2", "C:C1"]);
  expect(mergeReports([])).toEqual([]);
  expect(mergeReports([{ cargue: "X", report: { ...report([]), items: undefined as unknown as ReportItem[] } }])).toEqual([]);
});

test("filterRows keeps only manual reviews unless the switch includes excluidas and fallas", () => {
  expect(facturas(filterRows(merged(), noFilter))).toEqual(["A1", "A2", "B1", "C1"]);
  expect(facturas(filterRows(merged(), { ...noFilter, includeOthers: true }))).toEqual(["A1", "A2", "A3", "B1", "B2", "C1"]);
});

test("filterRows narrows by cargue, fase and categoria", () => {
  const all = { ...noFilter, includeOthers: true };
  expect(facturas(filterRows(merged(), { ...all, cargue: "B" }))).toEqual(["B1", "B2"]);
  expect(facturas(filterRows(merged(), { ...all, fase: "fev_valor" }))).toEqual(["A1", "B1", "C1"]);
  expect(facturas(filterRows(merged(), { ...all, categoria: "falla" }))).toEqual(["B2"]);
  expect(facturas(filterRows(merged(), { ...all, cargue: "A", categoria: "excluida" }))).toEqual(["A3"]);
  expect(filterRows(merged(), { ...noFilter, categoria: CATEGORY_ALL })).toHaveLength(4);
});

test("filterRows searches factura, codigo and motivo case-insensitively and ignores blanks", () => {
  expect(facturas(filterRows(merged(), { ...noFilter, text: "b1" }))).toEqual(["B1"]);
  expect(facturas(filterRows(merged(), { ...noFilter, text: "valor_diferente" }))).toEqual(["A1", "B1", "C1"]);
  expect(facturas(filterRows(merged(), { ...noFilter, text: "SIN DERECHOS" }))).toEqual(["A2"]);
  expect(facturas(filterRows(merged(), { ...noFilter, text: "   " }))).toEqual(["A1", "A2", "B1", "C1"]);
  expect(filterRows(merged(), { ...noFilter, text: "zzz" })).toEqual([]);
});

test("phaseOptions lists the distinct phases found in the rows with their labels", () => {
  expect(phaseOptions(merged())).toEqual([
    { value: "fev_valor", label: "FEV (fev_valor)" },
    { value: "derechos", label: "Derechos" },
    { value: "fev_ajuste", label: "FEV (fev_ajuste)" },
    { value: "rips", label: "RIPS" },
  ]);
});

test("categoryOptions only offers excluidas and fallas when the switch includes them", () => {
  expect(categoryOptions(false).map((o) => o.value)).toEqual(["revision_manual"]);
  expect(categoryOptions(true).map((o) => o.value)).toEqual(["revision_manual", "excluida", "falla"]);
});

test("cargueScope sums the counts and flags incomplete or failed cargues of the selection", () => {
  const all = cargueScope(reports, "");
  expect(all.conteos).toEqual({ revision_manual: 4, excluidas: 0, fallas: 0 });
  expect(all.incompleto).toBe(true);
  expect(all.errors).toEqual([{ cargue: "D", error: "informe no disponible" }]);
  const b = cargueScope(reports, "B");
  expect(b.incompleto).toBe(false);
  expect(b.errors).toEqual([]);
});

test("only value-difference rows of a complete report are retryable", () => {
  expect(retryableRows(merged(), reports).map((r) => `${r.cargue}:${r.factura}`)).toEqual(["A:A1", "B:B1"]);
});

test("rowKey distinguishes the same invoice in different cargues and phases", () => {
  const [a1] = merged();
  expect(rowKey(a1!)).not.toBe(rowKey({ ...a1!, cargue: "B" }));
  expect(rowKey(a1!)).not.toBe(rowKey({ ...a1!, fase: "derechos" }));
});

test("toggleRow adds and removes a key without mutating the previous selection", () => {
  const empty = new Set<string>();
  const one = toggleRow(empty, "k", true);
  expect([...one]).toEqual(["k"]);
  expect(empty.size).toBe(0);
  expect([...toggleRow(one, "k", false)]).toEqual([]);
});

test("toggleAllVisible selects only the retryable visible rows and can clear them", () => {
  const rows = filterRows(merged(), noFilter);
  const selected = toggleAllVisible(new Set(), rows, reports, true);
  expect([...selected].sort()).toEqual(retryableRows(rows, reports).map(rowKey).sort());
  expect(selected.size).toBe(2);
  expect(allVisibleState(selected, rows, reports)).toBe("all");
  const cleared = toggleAllVisible(selected, rows, reports, false);
  expect(cleared.size).toBe(0);
  expect(allVisibleState(cleared, rows, reports)).toBe("none");
});

test("toggleAllVisible leaves selected rows hidden by a filter alone and allVisibleState reports partial selection", () => {
  const all = filterRows(merged(), noFilter);
  const onlyA = filterRows(merged(), { ...noFilter, cargue: "A" });
  const selected = toggleAllVisible(new Set(), all, reports, true);
  const afterClearA = toggleAllVisible(selected, onlyA, reports, false);
  expect([...afterClearA]).toEqual([rowKey(all.find((r) => r.factura === "B1")!)]);
  expect(allVisibleState(afterClearA, all, reports)).toBe("some");
  expect(allVisibleState(new Set(), filterRows(merged(), { ...noFilter, text: "zzz" }), reports)).toBe("none");
});

// --- batch runner ------------------------------------------------------------------------

function batchHarness(over: Partial<BatchActions> = {}) {
  const state = { busy: false, status: [] as string[], error: "", output: "", submitted: [] as string[], confirmations: [] as number[], refreshes: 0 };
  const actions: BatchActions = {
    isEligible: () => true,
    isBusy: () => state.busy,
    setBusy: (value) => {
      state.busy = value;
    },
    confirm: async (count) => {
      state.confirmations.push(count);
      return true;
    },
    submit: async (cargue, factura) => {
      state.submitted.push(`${cargue}:${factura}`);
      return { id: `job-${state.submitted.length}` };
    },
    currentJob: async () => ({ id: `job-${state.submitted.length}`, corriendo: false, salida: "ok" }),
    history: async () => [],
    refresh: async () => {
      state.refreshes++;
    },
    setStatus: (value) => state.status.push(value),
    setError: (value) => {
      state.error = value;
    },
    setOutput: (value) => {
      state.output = value;
    },
    delay: async () => {},
    ...over,
  };
  return { state, actions };
}

const targets = (...pairs: [string, string][]) => pairs.map(([cargue, factura]) => ({ cargue, item: item({ factura }) }));
test("runBatchRetry asks once for the whole batch and retries each row in order", async () => {
  const { state, actions } = batchHarness();
  const result = await runBatchRetry(targets(["A", "A1"], ["A", "A9"], ["B", "B1"]), { ...actions });
  expect(state.confirmations).toEqual([3]);
  expect(state.submitted).toEqual(["A:A1", "A:A9", "B:B1"]);
  expect(result.status).toBe("done");
  expect(result.results.map((r) => [r.cargue, r.factura, r.result.status])).toEqual([
    ["A", "A1", "completed"],
    ["A", "A9", "completed"],
    ["B", "B1", "completed"],
  ]);
  expect(state.refreshes).toBe(1);
  expect(state.busy).toBe(false);
});

test("runBatchRetry keeps going after a failed row and reports it per row", async () => {
  const { state, actions } = batchHarness({
    submit: async (cargue, factura) => {
      state.submitted.push(`${cargue}:${factura}`);
      if (factura === "A9") throw new Error("ya hay un proceso en curso");
      return { id: `job-${state.submitted.length}` };
    },
  });
  const result = await runBatchRetry(targets(["A", "A1"], ["A", "A9"], ["B", "B1"]), { ...actions });
  expect(state.submitted).toEqual(["A:A1", "A:A9", "B:B1"]);
  expect(result.results.map((r) => r.result.status)).toEqual(["completed", "error", "completed"]);
  expect(state.error).toContain("A9");
  expect(state.error).toContain("ya hay un proceso en curso");
  expect(state.refreshes).toBe(1);
});

test("runBatchRetry does nothing when the operator declines", async () => {
  const { state, actions } = batchHarness({ confirm: async () => false });
  const result = await runBatchRetry(targets(["A", "A1"]), { ...actions });
  expect(result).toEqual({ status: "cancelled", results: [] });
  expect(state.submitted).toEqual([]);
  expect(state.refreshes).toBe(0);
  expect(state.busy).toBe(false);
});

test("runBatchRetry ignores an empty batch, a busy page and ineligible rows", async () => {
  const { state, actions } = batchHarness();
  expect(await runBatchRetry([], { ...actions })).toEqual({ status: "ignored", results: [] });
  expect(state.confirmations).toEqual([]);

  const busy = batchHarness({ isBusy: () => true });
  expect(await runBatchRetry(targets(["A", "A1"]), { ...busy.actions })).toEqual({ status: "ignored", results: [] });

  const mixed = batchHarness();
  const result = await runBatchRetry(targets(["A", "A1"], ["B", "B1"]), { ...mixed.actions, isEligible: (t) => t.cargue === "B" });
  expect(mixed.state.confirmations).toEqual([1]);
  expect(mixed.state.submitted).toEqual(["B:B1"]);
  expect(result.results.map((r) => r.factura)).toEqual(["B1"]);
});

test("runBatchRetry de-duplicates a cargue/factura pair selected twice", async () => {
  const { state, actions } = batchHarness();
  await runBatchRetry(targets(["A", "A1"], ["A", "A1"]), { ...actions });
  expect(state.submitted).toEqual(["A:A1"]);
});

test("runBatchRetry stays busy across the rows and reports progress", async () => {
  const busyDuring: boolean[] = [];
  const progress: string[] = [];
  const { state, actions } = batchHarness({
    submit: async () => {
      busyDuring.push(state.busy);
      return { id: "job" };
    },
    currentJob: async () => ({ id: "job", corriendo: false }),
  });
  await runBatchRetry(targets(["A", "A1"], ["B", "B1"]), { ...actions, onProgress: (index, total, t) => progress.push(`${index}/${total} ${t.cargue}:${t.item.factura}`) });
  expect(busyDuring).toEqual([true, true]);
  expect(progress).toEqual(["1/2 A:A1", "2/2 B:B1"]);
});

test("runBatchRetry surfaces a refresh failure without losing the row results", async () => {
  const { state, actions } = batchHarness({
    refresh: async () => {
      throw new Error("sin red");
    },
  });
  const result = await runBatchRetry(targets(["A", "A1"]), { ...actions });
  expect(result.results).toHaveLength(1);
  expect(state.error).toContain("No se pudo actualizar el informe: sin red");
  expect(state.busy).toBe(false);
});
