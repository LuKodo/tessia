import { expect, test } from "vitest";
import { canRetryValueDifference, runInvoiceRetry } from "../../src/utils/retry-model";

test("only complete exact value-difference manual reviews are retryable", () => {
  const exact = { fase: "fev_valor", categoria: "revision_manual", codigo: "VALOR_DIFERENTE" };
  expect(canRetryValueDifference(exact, false)).toBe(true);
  expect(canRetryValueDifference({ ...exact, codigo: "", motivo: "VALOR_DIFERENTE XML=1 convenio=2" }, false)).toBe(true);
  expect(canRetryValueDifference(exact, true)).toBe(false);
  expect(canRetryValueDifference({ ...exact, fase: "derechos" }, false)).toBe(false);
  expect(canRetryValueDifference({ ...exact, codigo: "GENERIC" }, false)).toBe(false);
  expect(canRetryValueDifference({ ...exact, codigo: "", motivo: "VALOR_DIFERENTE_OTRO XML=1" }, false)).toBe(false);
  expect(canRetryValueDifference({ ...exact, categoria: "falla" }, false)).toBe(false);
});

function fakeRetry() {
  const state = { busy: false, status: "", error: "", output: "" };
  const result = {
    state,
    isBusy: () => state.busy,
    setBusy: (value: boolean) => { state.busy = value; },
    setStatus: (value: string) => { state.status = value; },
    setError: (value: string) => { state.error = value; },
    setOutput: (value: string) => { state.output = value; },
  };
  return result;
}

test("busy is reserved before confirmation and a second invocation cannot open another dialog", async () => {
  const ui = fakeRetry();
  let acceptConfirmation!: (accepted: boolean) => void;
  const confirmation = new Promise<boolean>((resolve) => { acceptConfirmation = resolve; });
  let confirmations = 0;
  let submissions = 0;
  const actions = {
    ...ui,
    isEligible: () => true,
    confirm: () => { confirmations++; return confirmation; },
    submit: async () => { submissions++; return { id: "job-1" }; },
    currentJob: async () => ({ id: "job-1", corriendo: false }),
    history: async () => [],
    refresh: async () => {},
  };

  const first = runInvoiceRetry({ factura: "EVS1" }, actions);
  expect(ui.state.busy).toBe(true);
  expect(await runInvoiceRetry({ factura: "EVS1" }, actions)).toEqual({ status: "ignored" });
  expect(confirmations).toBe(1);
  acceptConfirmation(false);
  expect(await first).toEqual({ status: "cancelled" });
  expect(ui.state.busy).toBe(false);
  expect(submissions).toBe(0);
});

test("a replaced current job resolves from history and refreshes the report", async () => {
  const ui = fakeRetry();
  let historyCalls = 0;
  let refreshes = 0;
  const result = await runInvoiceRetry({ factura: "EVS1" }, {
    ...ui,
    isEligible: () => true,
    confirm: async () => true,
    submit: async () => ({ id: "retry-1" }),
    currentJob: async () => ({ id: "other-job", corriendo: true, salida: "other output" }),
    history: async () => ++historyCalls === 1 ? [] : [{ id: "retry-1", salida_final: "retry output" }],
    refresh: async () => { refreshes++; },
    maxAttempts: 3,
    delay: async () => {},
  });
  expect(result).toEqual({ status: "completed", output: "retry output" });
  expect(historyCalls).toBe(2);
  expect(refreshes).toBe(1);
  expect(ui.state.output).toBe("retry output");
  expect(ui.state.busy).toBe(false);
  expect(ui.state.status).toMatch(/Informe actualizado/);
});

test("unknown job outcome remains explicit and still refreshes the report", async () => {
  const ui = fakeRetry();
  let refreshes = 0;
  let historyCalls = 0;
  const result = await runInvoiceRetry({ factura: "EVS1" }, {
    ...ui,
    isEligible: () => true,
    confirm: async () => true,
    submit: async () => ({ id: "retry-1" }),
    currentJob: async () => ({ id: "other-job", corriendo: true }),
    history: async () => { historyCalls++; return []; },
    refresh: async () => { refreshes++; },
    maxAttempts: 2,
    maxHistoryAttempts: 2,
    delay: async () => {},
  });
  expect(result.status).toBe("unknown");
  expect(ui.state.error).toMatch(/No se pudo confirmar/);
  expect(ui.state.status).toBe("");
  expect(refreshes).toBe(1);
  expect(historyCalls).toBe(2);
  expect(ui.state.busy).toBe(false);
});

test("a failed job recovered from history stays a failure while the report refreshes", async () => {
  const ui = fakeRetry();
  let refreshes = 0;
  const result = await runInvoiceRetry({ factura: "EVS1" }, {
    ...ui,
    isEligible: () => true,
    confirm: async () => true,
    submit: async () => ({ id: "retry-1" }),
    currentJob: async () => { throw new Error("current unavailable"); },
    history: async () => [{ id: "retry-1", error: "CUV failed", salida_final: "failure output" }],
    refresh: async () => { refreshes++; },
    maxAttempts: 2,
    maxHistoryAttempts: 2,
    delay: async () => {},
  });
  expect(result).toEqual({ status: "failed", error: "CUV failed", output: "failure output" });
  expect(ui.state.error).toBe("CUV failed");
  expect(ui.state.output).toBe("failure output");
  expect(ui.state.status).toBe("");
  expect(refreshes).toBe(1);
});
