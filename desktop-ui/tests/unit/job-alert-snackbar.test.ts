import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TxJobAlertSnackbar } from "../../src/components/job-alert-snackbar";
import type { TxAlert } from "../../src/components/tx-alert";
import type { JobSnapshot } from "../../src/types/api";
import { fakeApi, useApi } from "../support/ejecuciones-api";

vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
await import("../../src/components/job-alert-snackbar");

const idle: JobSnapshot = { id: "", tipo: "", corriendo: false, cancelable: false, iniciado_en: "0001-01-01T00:00:00Z", salida: "", instancia_servidor: "A" };
const running: JobSnapshot = { ...idle, id: "job-1", tipo: "proceso run", corriendo: true, cancelable: true, iniciado_en: "2026-10-01T10:00:00Z" };
const finished: JobSnapshot = { ...running, corriendo: false, cancelable: false, terminado_en: "2026-10-01T10:02:05Z" };

async function mountSnackbar(snapshots: JobSnapshot[]) {
  const queue = [...snapshots];
  useApi(fakeApi({ jobsCurrent: vi.fn(async () => (queue.length > 1 ? queue.shift()! : queue[0]!)) }));
  const el = document.createElement("tx-job-alert-snackbar") as TxJobAlertSnackbar;
  document.body.append(el);
  await vi.advanceTimersByTimeAsync(0);
  return el;
}

// A running job is polled every 5 s.
async function settle(el: TxJobAlertSnackbar) {
  await vi.advanceTimersByTimeAsync(5000);
  await el.updateComplete;
}

const toast = (el: TxJobAlertSnackbar) => el.shadowRoot!.querySelector<TxAlert>("tx-alert");
const verLog = (el: TxJobAlertSnackbar) => [...el.shadowRoot!.querySelectorAll("tx-button")].find((b) => b.textContent!.includes("Ver log"))!;

describe("<tx-job-alert-snackbar>", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    document.title = "Tessia";
    history.replaceState(null, "", "/");
  });
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("raises a persistent alert with the outcome and duration when a job finishes", async () => {
    const el = await mountSnackbar([running, finished]);
    await settle(el);
    expect(toast(el)!.heading).toBe("Proceso terminado: proceso run");
    expect(toast(el)!.description).toBe("Duración: 2 min 5 s");
    expect(toast(el)!.color).toBe("success");
    expect(toast(el)!.closable).toBe(true);
    // Stays until the operator closes it, however long it takes.
    await vi.advanceTimersByTimeAsync(60_000);
    await el.updateComplete;
    expect(toast(el)).not.toBeNull();
  });

  it("colors a failed job as an error and a cancelled one as a warning", async () => {
    const failed = await mountSnackbar([running, { ...finished, error: "boom" }]);
    await settle(failed);
    expect(toast(failed)!.heading).toBe("Proceso terminado con error: proceso run");
    expect(toast(failed)!.color).toBe("error");
    document.body.innerHTML = "";

    const cancelled = await mountSnackbar([running, { ...finished, id: "job-2", error: "cancelado por el operador" }]);
    await settle(cancelled);
    expect(toast(cancelled)!.heading).toContain("Proceso cancelado");
    expect(toast(cancelled)!.color).toBe("warning");
  });

  it("flags a service restart without a duration", async () => {
    const el = await mountSnackbar([running, { ...idle, instancia_servidor: "B" }]);
    await settle(el);
    expect(toast(el)!.heading).toBe("La ejecución se interrumpió inesperadamente: el servicio se reinició.");
    expect(toast(el)!.color).toBe("warning");
    expect(toast(el)!.description).toBeFalsy();
  });

  it("opens the history page from Ver log and closes the alert", async () => {
    const el = await mountSnackbar([running, { ...finished, id: "job-3" }]);
    await settle(el);
    (verLog(el) as HTMLElement).click();
    await el.updateComplete;
    expect(location.pathname).toBe("/ejecuciones");
    expect(toast(el)).toBeNull();
  });

  it("closes the alert with its own close button", async () => {
    const el = await mountSnackbar([running, finished]);
    await settle(el);
    toast(el)!.dispatchEvent(new CustomEvent("close"));
    await el.updateComplete;
    expect(toast(el)).toBeNull();
  });

  it("stays quiet while no job has finished", async () => {
    const el = await mountSnackbar([idle]);
    await settle(el);
    expect(toast(el)).toBeNull();
  });
});
