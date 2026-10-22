import type { ReactiveControllerHost } from "lit";
import type { JobSnapshot } from "../types/api";
import type { TessiaApi } from "../utils/tessia-api";
import { PollController } from "./poll-controller";

/** While a job runs the dashboard only needs a calm heartbeat: the job's log is not streamed. */
export const JOB_POLL_RUNNING_MS = 5000;
/** The idle snapshot is cheap, so a job started elsewhere shows up quickly. */
export const JOB_POLL_IDLE_MS = 2000;

/**
 * Polls /api/jobs/current: every 5 s while a job runs, every 2 s otherwise (port of useJobPoll).
 * `polledAt` is the time of the last answer, so an elapsed-time display moves with each poll instead
 * of a local ticking clock. `observe` sees every answer, in arrival order, before the host re-renders
 * (JobAlertController uses it to compare consecutive snapshots); answers that arrive after the host
 * disconnected are not reported.
 */
export class JobPollController extends PollController<JobSnapshot> {
  /** `Date.now()` when the latest answer arrived; 0 before the first one. */
  polledAt = 0;
  private live = false;

  constructor(
    host: ReactiveControllerHost,
    api: Pick<TessiaApi, "jobsCurrent">,
    observe?: (job: JobSnapshot) => void,
  ) {
    super(
      host,
      async () => {
        const job = await api.jobsCurrent();
        if (this.live) {
          this.polledAt = Date.now();
          observe?.(job);
        }
        return job;
      },
      (job) => (job?.corriendo ? JOB_POLL_RUNNING_MS : JOB_POLL_IDLE_MS),
    );
  }

  override hostConnected(): void {
    this.live = true;
    super.hostConnected();
  }

  override hostDisconnected(): void {
    this.live = false;
    super.hostDisconnected();
  }
}
