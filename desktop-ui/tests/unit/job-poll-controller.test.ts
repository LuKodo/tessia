import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LitElement, html } from "lit";
import { JOB_POLL_IDLE_MS, JOB_POLL_RUNNING_MS, JobPollController } from "../../src/controllers/job-poll-controller";
import type { JobSnapshot } from "../../src/types/api";

const idle: JobSnapshot = { id: "", tipo: "", corriendo: false, cancelable: false, iniciado_en: "0001-01-01T00:00:00Z", salida: "" };
const running: JobSnapshot = { ...idle, id: "job-1", tipo: "proceso run", corriendo: true, cancelable: true };

class PollHost extends LitElement {
  jobsCurrent!: () => Promise<JobSnapshot>;
  poll!: JobPollController;
  seen: JobSnapshot[] = [];
  init(jobsCurrent: () => Promise<JobSnapshot>, observe?: (job: JobSnapshot) => void) {
    this.jobsCurrent = jobsCurrent;
    this.poll = new JobPollController(this, { jobsCurrent }, observe);
  }
  render() {
    return html`<span>${this.poll.data?.id ?? ""}</span>`;
  }
}
customElements.define("poll-host", PollHost);

async function mountPoll(snapshots: JobSnapshot[], observe?: (job: JobSnapshot) => void) {
  const queue = [...snapshots];
  const jobsCurrent = vi.fn(async () => (queue.length > 1 ? queue.shift()! : queue[0]!));
  const el = document.createElement("poll-host") as PollHost;
  el.init(jobsCurrent, observe);
  document.body.append(el);
  await vi.advanceTimersByTimeAsync(0);
  return { el, jobsCurrent };
}

describe("JobPollController", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("polls the cheap idle snapshot every 2 seconds", async () => {
    expect(JOB_POLL_IDLE_MS).toBe(2000);
    const { jobsCurrent } = await mountPoll([idle]);
    await vi.advanceTimersByTimeAsync(4000);
    expect(jobsCurrent).toHaveBeenCalledTimes(3);
  });

  it("slows down to 5 seconds while a job runs and speeds up again when it ends", async () => {
    expect(JOB_POLL_RUNNING_MS).toBe(5000);
    const { jobsCurrent } = await mountPoll([running, running, idle]);
    expect(jobsCurrent).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(4999);
    expect(jobsCurrent).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(jobsCurrent).toHaveBeenCalledTimes(2); // still running: next wait is 5 s
    await vi.advanceTimersByTimeAsync(5000);
    expect(jobsCurrent).toHaveBeenCalledTimes(3); // answers idle: back to 2 s
    await vi.advanceTimersByTimeAsync(4000);
    expect(jobsCurrent).toHaveBeenCalledTimes(5);
  });

  it("exposes the latest job and re-renders the host with it", async () => {
    const { el } = await mountPoll([running]);
    await el.updateComplete;
    expect(el.poll.data?.id).toBe("job-1");
    expect(el.shadowRoot!.querySelector("span")!.textContent).toBe("job-1");
  });

  it("records when the last answer arrived, even if the snapshot did not change", async () => {
    vi.setSystemTime(new Date("2026-10-01T10:00:00Z"));
    const { el } = await mountPoll([running]);
    expect(el.poll.polledAt).toBe(Date.parse("2026-10-01T10:00:00Z"));
    await vi.advanceTimersByTimeAsync(5000);
    expect(el.poll.polledAt).toBe(Date.parse("2026-10-01T10:00:05Z"));
  });

  it("reports every answer to the observer and stops after the host disconnects", async () => {
    const seen: string[] = [];
    const { el, jobsCurrent } = await mountPoll([running, idle], (job) => seen.push(job.id || "idle"));
    await vi.advanceTimersByTimeAsync(5000);
    expect(seen).toEqual(["job-1", "idle"]);
    el.remove();
    await vi.advanceTimersByTimeAsync(10000);
    expect(jobsCurrent).toHaveBeenCalledTimes(2);
  });
});
