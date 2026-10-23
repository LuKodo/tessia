import type { ReactiveController, ReactiveControllerHost } from "lit";
import { alertTitle, detectJobFinish, detectServerRestart, type JobAlert } from "../utils/job-alert";
import type { JobSnapshot } from "../types/api";
import type { TessiaApi } from "../utils/tessia-api";
import { JobPollController } from "./job-poll-controller";

/**
 * Watches /api/jobs/current on its own (independent of the console panel, so it works from any
 * page) and exposes `alert`, null or the descriptor from detectJobFinish, until the operator calls
 * `dismiss()`. While the tab is in the background the tab title also carries the outcome; it is
 * restored when the tab becomes visible again or the alert is dismissed. Port of useJobAlert.
 */
export class JobAlertController implements ReactiveController {
  alert: JobAlert | null = null;

  private poll: JobPollController;
  private previous: JobSnapshot | null = null;
  private savedTitle: string | null = null;

  constructor(
    private host: ReactiveControllerHost,
    api: Pick<TessiaApi, "jobsCurrent">,
  ) {
    this.poll = new JobPollController(host, api, (job) => this.observe(job));
    host.addController(this);
  }

  hostConnected(): void {
    document.addEventListener("visibilitychange", this.onVisibility);
  }

  hostDisconnected(): void {
    document.removeEventListener("visibilitychange", this.onVisibility);
    this.previous = null;
    this.restoreTitle();
  }

  dismiss(): void {
    this.alert = null;
    this.restoreTitle();
    this.host.requestUpdate();
  }

  // A server restart mid-job takes priority: once the instance id changes, the old job's normal
  // finish can no longer be observed through this instance.
  private observe(next: JobSnapshot): void {
    const found = detectServerRestart(this.previous, next) || detectJobFinish(this.previous, next);
    this.previous = next;
    if (!found) return;
    this.alert = found;
    if (document.hidden) {
      if (this.savedTitle === null) this.savedTitle = document.title;
      document.title = alertTitle(found.outcome, this.savedTitle);
    }
    this.host.requestUpdate();
  }

  private restoreTitle(): void {
    if (this.savedTitle === null) return;
    document.title = this.savedTitle;
    this.savedTitle = null;
  }

  private onVisibility = (): void => {
    if (!document.hidden) this.restoreTitle();
  };
}
