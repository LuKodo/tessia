import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LitElement, html } from "lit";
import { JobAlertController } from "../../src/controllers/job-alert-controller";
import type { JobSnapshot } from "../../src/types/api";

const idle: JobSnapshot = { id: "", tipo: "", corriendo: false, cancelable: false, iniciado_en: "0001-01-01T00:00:00Z", salida: "", instancia_servidor: "A" };
const running: JobSnapshot = { ...idle, id: "job-1", tipo: "proceso run", corriendo: true, cancelable: true, iniciado_en: "2026-10-01T10:00:00Z" };
const finished: JobSnapshot = { ...running, corriendo: false, cancelable: false, terminado_en: "2026-10-01T10:02:05Z" };

class AlertHost extends LitElement {
  state!: JobAlertController;
  renders = 0;
  init(jobsCurrent: () => Promise<JobSnapshot>) {
    this.state = new JobAlertController(this, { jobsCurrent });
  }
  render() {
    this.renders++;
    return html`<span>${this.state.alert?.message ?? ""}</span>`;
  }
}
customElements.define("alert-host", AlertHost);

async function mountAlert(snapshots: JobSnapshot[]) {
  const queue = [...snapshots];
  const jobsCurrent = vi.fn(async () => (queue.length > 1 ? queue.shift()! : queue[0]!));
  const el = document.createElement("alert-host") as AlertHost;
  el.init(jobsCurrent);
  document.body.append(el);
  await vi.advanceTimersByTimeAsync(0);
  return { el, state: el.state, jobsCurrent };
}

// The next poll after a "running" snapshot is due in 5 s (see JobPollController).
const tick = () => vi.advanceTimersByTimeAsync(5000);

function setHidden(hidden: boolean) {
  Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
  document.dispatchEvent(new Event("visibilitychange"));
}

describe("JobAlertController", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    document.title = "Tessia";
    setHidden(false);
  });
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("polls /api/jobs/current every 2 seconds", async () => {
    const { jobsCurrent } = await mountAlert([idle]);
    expect(jobsCurrent).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(2000);
    expect(jobsCurrent).toHaveBeenCalledTimes(2);
  });

  it("polls every 5 seconds while the job runs", async () => {
    const { jobsCurrent } = await mountAlert([running]);
    await vi.advanceTimersByTimeAsync(4999);
    expect(jobsCurrent).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(jobsCurrent).toHaveBeenCalledTimes(2);
  });

  it("stays quiet on the first observation of an already finished job", async () => {
    const { state } = await mountAlert([finished]);
    expect(state.alert).toBeNull();
  });

  it("raises the finish alert when a running job ends, re-renders the host, and dismiss clears it", async () => {
    const { el, state } = await mountAlert([running, finished]);
    await tick();
    await el.updateComplete;
    expect(state.alert).toMatchObject({ id: "job-1", outcome: "ok", durationText: "2 min 5 s" });
    expect(el.shadowRoot!.querySelector("span")!.textContent).toBe("Proceso terminado: proceso run");
    state.dismiss();
    await el.updateComplete;
    expect(state.alert).toBeNull();
    expect(el.shadowRoot!.querySelector("span")!.textContent).toBe("");
  });

  it("flags an interrupted run when the server instance changes mid-job", async () => {
    const { state } = await mountAlert([running, { ...idle, instancia_servidor: "B" }]);
    await tick();
    expect(state.alert).toMatchObject({ outcome: "interrumpido" });
  });

  it("prefixes the tab title while the tab is hidden and restores it when it is shown again", async () => {
    const { state } = await mountAlert([running, finished]);
    setHidden(true);
    await tick();
    expect(document.title).toBe("✔ Proceso terminado · Tessia");
    setHidden(false);
    expect(document.title).toBe("Tessia");
    expect(state.alert).not.toBeNull();
  });

  it("leaves the title alone while the tab is visible", async () => {
    await mountAlert([running, finished]);
    await tick();
    expect(document.title).toBe("Tessia");
  });

  it("restores the title on dismiss and on disconnect", async () => {
    const { el, state } = await mountAlert([running, finished]);
    setHidden(true);
    await tick();
    state.dismiss();
    expect(document.title).toBe("Tessia");

    document.title = "Tessia";
    const second = await mountAlert([running, finished]);
    await tick();
    expect(document.title).toBe("✔ Proceso terminado · Tessia");
    second.el.remove();
    el.remove();
    expect(document.title).toBe("Tessia");
  });

  it("stops listening to visibility changes once disconnected", async () => {
    const { el } = await mountAlert([running, finished]);
    const spy = vi.spyOn(document, "removeEventListener");
    el.remove();
    expect(spy).toHaveBeenCalledWith("visibilitychange", expect.any(Function));
  });
});
