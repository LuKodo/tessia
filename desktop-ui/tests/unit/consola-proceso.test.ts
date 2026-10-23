import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TxConsolaProceso } from "../../src/components/consola-proceso";
import type { JobSnapshot } from "../../src/types/api";
import { ApiError } from "../../src/utils/api-client";
import { dialogs } from "../../src/utils/dialogs";
import { fakeApi, useApi } from "../support/ejecuciones-api";

vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
await import("../../src/components/consola-proceso");

const running: JobSnapshot = {
  id: "job-1",
  tipo: "proceso run",
  corriendo: true,
  cancelable: true,
  iniciado_en: "2026-10-01T10:00:00Z",
  salida: "Inicio 10:00:00 · proceso run · cargue CARGUE_1\nConvenio preparar: 3 preparadas · 1 excluidas · 0 revisión manual · 4 evaluadas",
  fases: [{ fase: "derechos", etiqueta: "derechos validar", en_curso: 1, terminadas: 3, revision_manual: 0, excluidas: 0, con_errores: 0, tiempo_ms: 192000 }],
  convenio: { estado: "ok", evaluadas: 4, total: 4, preparadas: 3, excluidas: 1, revision_manual: 0 },
  estadisticas: { esperadas: 4, procesadas: 4, reemplazadas: 0, gestionadas: 3, terminadas: 3, con_errores: 0, excluidas: 0, revision_manual: 0, evaluadas: 0, limite: 0 },
};
const idleJob: JobSnapshot = { ...running, id: "", corriendo: false, cancelable: false, salida: "", fases: undefined, convenio: undefined };

async function mountConsole(job: JobSnapshot, extra: Record<string, unknown> = {}, open = true) {
  const api = fakeApi({ jobsCurrent: vi.fn(async () => job), ...extra });
  useApi(api);
  const el = document.createElement("tx-consola-proceso") as TxConsolaProceso;
  if (open) el.setAttribute("default-open", "");
  document.body.append(el);
  await vi.advanceTimersByTimeAsync(0);
  await el.updateComplete;
  return { el, api };
}

const q = (el: TxConsolaProceso, sel: string) => el.shadowRoot!.querySelector<HTMLElement>(sel);
const text = (el: TxConsolaProceso) => el.shadowRoot!.textContent!.replace(/\s+/g, " ");
const click = (e: Element | null) => e!.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));

describe("<tx-consola-proceso>", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
    dialogs.answerConfirm(false);
    dialogs.dismissError();
  });

  it("shows idle text without a job", async () => {
    const { el } = await mountConsole(idleJob);
    expect(q(el, "[data-testid=console-output]")!.textContent).toBe("sin actividad");
    expect(q(el, "[data-testid=job-elapsed]")).toBeNull();
    expect(q(el, "[data-testid=job-state]")).toBeNull();
    expect(q(el, "[data-testid=cancel-job]")).toBeNull();
  });

  it("renders the job kind, state, elapsed time and the synthetic log while running", async () => {
    vi.setSystemTime(new Date("2026-10-01T10:03:12Z"));
    const { el } = await mountConsole(running);
    expect(text(el)).toContain("Registro del proceso — proceso run (en curso)");
    expect(q(el, "[data-testid=job-state]")!.textContent!.trim()).toBe("en curso");
    expect(q(el, "[data-testid=job-state]")!.getAttribute("color")).toBe("warning");
    expect(q(el, "[data-testid=job-elapsed]")!.textContent).toBe("3 min 12 s");
    expect(q(el, "[data-testid=console-output]")!.textContent).toBe(running.salida);
  });

  it("moves the elapsed time with each poll, not with a local clock", async () => {
    vi.setSystemTime(new Date("2026-10-01T10:00:10Z"));
    const { el } = await mountConsole(running);
    expect(q(el, "[data-testid=job-elapsed]")!.textContent).toBe("10 s");
    await vi.advanceTimersByTimeAsync(4000);
    await el.updateComplete;
    expect(q(el, "[data-testid=job-elapsed]")!.textContent).toBe("10 s");
    await vi.advanceTimersByTimeAsync(1000);
    await el.updateComplete;
    expect(q(el, "[data-testid=job-elapsed]")!.textContent).toBe("15 s");
  });

  it("shows the live phase cards while the job runs, even with the log collapsed", async () => {
    const { el } = await mountConsole(
      {
        ...running,
        fases: [
          { fase: "derechos", etiqueta: "derechos validar", en_curso: 1, terminadas: 3, revision_manual: 0, excluidas: 0, con_errores: 0, tiempo_ms: 192000 },
          { fase: "fev", etiqueta: "fev validar", en_curso: 0, terminadas: 0, revision_manual: 0, excluidas: 0, con_errores: 0 },
        ],
        radicadas: 2,
      },
      {},
      false,
    );
    const fases = q(el, "tx-fases-proceso") as HTMLElement & { fases: unknown[]; radicadas: number; shadowRoot: ShadowRoot };
    expect(fases).not.toBeNull();
    expect(q(el, "#console-body")!.hidden).toBe(true);
    expect(fases.closest("#console-body")).toBeNull();
    await (fases as unknown as { updateComplete: Promise<unknown> }).updateComplete;
    const cards = [...fases.shadowRoot.querySelectorAll<HTMLElement>("[data-testid=fase-card]")];
    expect(cards.map((c) => c.dataset.fase)).toEqual(["derechos", "fev", "radicacion"]);
    expect(cards[0]!.textContent).toContain("3/4");
    expect(cards[1]!.dataset.estado).toBe("en-cola");
    expect(cards[2]!.textContent).toContain("2 radicadas");
    // The cards sit between the header and the log.
    const order = [...el.shadowRoot!.querySelectorAll("tx-fases-proceso, #console-body, .head")].map((n) => n.tagName.toLowerCase() + (n.id ? `#${n.id}` : ""));
    expect(order).toEqual(["div", "tx-fases-proceso", "div#console-body"]);
  });

  it("keeps the phase summary after the job ends", async () => {
    const { el } = await mountConsole({ ...running, corriendo: false, cancelable: false, terminado_en: "2026-10-01T10:05:00Z" });
    const fases = q(el, "tx-fases-proceso") as HTMLElement;
    expect(fases).not.toBeNull();
    await (fases as unknown as { updateComplete: Promise<unknown> }).updateComplete;
    expect(fases.shadowRoot!.querySelectorAll("[data-testid=fase-card]")).toHaveLength(1);
  });

  it("hides the phase cards without a job or before the pipeline reports phases", async () => {
    const idle = await mountConsole(idleJob);
    expect(q(idle.el, "tx-fases-proceso")).toBeNull();
    document.body.innerHTML = "";
    const starting = await mountConsole({ ...running, fases: undefined });
    expect(q(starting.el, "tx-fases-proceso")).toBeNull();
    document.body.innerHTML = "";
    const empty = await mountConsole({ ...running, fases: [] });
    expect(q(empty.el, "tx-fases-proceso")).toBeNull();
  });

  it("does not poll more for the phase cards", async () => {
    const { api } = await mountConsole(running);
    expect(api.jobsCurrent).toHaveBeenCalledTimes(1);
  });

  it("keeps the invoice stat chips for the end of the run only", async () => {
    const live = await mountConsole(running);
    expect(text(live.el)).not.toContain("Terminadas:");
    document.body.innerHTML = "";
    const done = await mountConsole({ ...running, corriendo: false, cancelable: false, terminado_en: "2026-10-01T10:05:00Z" });
    expect(text(done.el)).toContain("Terminadas: 3");
    expect(q(done.el, "[data-testid=job-elapsed]")!.textContent).toBe("5 min 0 s");
  });

  it("shows the convenio strip as a static summary once step 0 is over, never while preparing", async () => {
    const live = await mountConsole(running);
    expect(q(live.el, "tx-convenio-progreso")).not.toBeNull();
    document.body.innerHTML = "";
    const preparing = await mountConsole({ ...running, convenio: { estado: "preparando", evaluadas: 40, total: 160, preparadas: 0, excluidas: 0, revision_manual: 0 } });
    expect(q(preparing.el, "tx-convenio-progreso")).toBeNull();
  });

  it("toggles the console body from the header button", async () => {
    const { el } = await mountConsole(running);
    const toggle = q(el, "button[aria-controls=console-body]")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    click(toggle);
    await el.updateComplete;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    expect(q(el, "#console-body")!.hidden).toBe(true);
    click(toggle);
    await el.updateComplete;
    expect(q(el, "#console-body")!.hidden).toBe(false);
  });

  it("starts collapsed unless default-open is set", async () => {
    const { el } = await mountConsole(running, {}, false);
    expect(q(el, "button[aria-controls=console-body]")!.getAttribute("aria-expanded")).toBe("false");
    expect(q(el, "#console-body")!.hidden).toBe(true);
  });

  it("distinguishes finished, failed and cancelled jobs without repeating the error", async () => {
    const base = { ...running, corriendo: false, cancelable: false, terminado_en: "2026-10-01T10:02:00Z" };
    const ok = await mountConsole(base);
    expect(q(ok.el, "[data-testid=job-state]")!.textContent!.trim()).toBe("terminado");
    document.body.innerHTML = "";

    const failedOutput = "Inicio\nProceso con error tras 2 min 0 s: CUV falló";
    const failed = await mountConsole({ ...base, error: "CUV falló", salida: failedOutput });
    expect(q(failed.el, "[data-testid=job-state]")!.textContent!.trim()).toBe("con error");
    expect(q(failed.el, "[data-testid=job-state]")!.getAttribute("color")).toBe("error");
    expect(q(failed.el, "[data-testid=console-output]")!.textContent).toBe(failedOutput);
    document.body.innerHTML = "";

    const cancelled = await mountConsole({ ...base, error: "cancelado por el operador" });
    expect(q(cancelled.el, "[data-testid=job-state]")!.textContent!.trim()).toBe("cancelado");
    expect(text(cancelled.el)).toContain("— proceso run (cancelado)");
  });

  it("polls the current job every 5 seconds while it runs and every 2 seconds when idle", async () => {
    const live = await mountConsole(running);
    expect(live.api.jobsCurrent).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(4999);
    expect(live.api.jobsCurrent).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(live.api.jobsCurrent).toHaveBeenCalledTimes(2);
    document.body.innerHTML = "";

    const idle = await mountConsole({ ...running, id: "", corriendo: false, cancelable: false });
    await vi.advanceTimersByTimeAsync(2000);
    expect(idle.api.jobsCurrent).toHaveBeenCalledTimes(2);
  });

  it("does not poll the tessia.log feed", async () => {
    const log = vi.fn();
    await mountConsole(running, { log });
    await vi.advanceTimersByTimeAsync(20000);
    expect(log).not.toHaveBeenCalled();
  });

  it("cancels only after the operator confirms", async () => {
    const { el, api } = await mountConsole(running);
    click(q(el, "[data-testid=cancel-job]"));
    expect(dialogs.confirm?.text).toContain("¿Cancelar el proceso en curso?");
    dialogs.answerConfirm(false);
    await vi.advanceTimersByTimeAsync(0);
    expect(api.cancelJob).not.toHaveBeenCalled();

    click(q(el, "[data-testid=cancel-job]"));
    dialogs.answerConfirm(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(api.cancelJob).toHaveBeenCalledTimes(1);
  });

  it("reports a failed cancel through the error dialog", async () => {
    const cancelJob = vi.fn(async () => {
      throw new ApiError(500, "no hay proceso en curso");
    });
    const { el } = await mountConsole(running, { cancelJob });
    click(q(el, "[data-testid=cancel-job]"));
    dialogs.answerConfirm(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(dialogs.error?.text).toBe("no hay proceso en curso");
  });
});
