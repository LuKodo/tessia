import type { ReactiveController, ReactiveControllerHost } from "lit";

export type PollInterval<T> = number | ((data: T | null) => number);

export interface PollOptions {
  /**
   * Skip interval ticks while `document.hidden` and refresh at once when the page becomes visible
   * again. Off by default; for dashboard data nobody sees in a hidden tab. Never for polls that
   * must keep watching in the background (JobPollController feeds the job-finished alert).
   */
  pauseWhenHidden?: boolean;
}

/**
 * Runs `fetcher` when the host connects and every `intervalMs` until it disconnects, re-rendering
 * the host as `data`/`error`/`loading` change. `intervalMs` may be a function of the latest data
 * (null before the first answer), re-evaluated after every tick, so a poll can slow down while there
 * is nothing to see (see JobPollController).
 * Port of usePoll: a failed tick sets `error` and keeps the last `data`; results arriving after
 * disconnect are ignored. Ticks are not skipped while the tab is hidden, as in the old hook, unless
 * `pauseWhenHidden` is set.
 * Beyond usePoll: an interval tick is skipped while a request is still in flight, and an explicit
 * `refresh()` always runs and supersedes older requests (the latest answer wins).
 */
export class PollController<T> implements ReactiveController {
  /** Latest successful result; kept when a later tick fails. */
  data: T | null = null;
  /** Error of the last tick, cleared by the next success. */
  error: unknown = null;
  /** True until the first tick settles, false forever after. */
  loading = true;

  private timer: ReturnType<typeof setInterval> | undefined;
  private armed = 0;
  private connected = false;
  private seq = 0;
  private pending = false; // the latest request has not settled

  constructor(
    private host: ReactiveControllerHost,
    private fetcher: () => Promise<T>,
    private intervalMs: PollInterval<T>,
    private options: PollOptions = {},
  ) {
    host.addController(this);
  }

  hostConnected(): void {
    this.connected = true;
    if (this.options.pauseWhenHidden) document.addEventListener("visibilitychange", this.onVisibility);
    this.arm();
    void this.refresh();
  }

  hostDisconnected(): void {
    this.connected = false;
    document.removeEventListener("visibilitychange", this.onVisibility);
    this.seq++; // answers of requests started before this point are dropped
    this.pending = false;
    clearInterval(this.timer);
    this.armed = 0;
  }

  /** One tick on demand (e.g. after the query the fetcher reads changed). */
  async refresh(): Promise<void> {
    const mine = ++this.seq;
    this.pending = true;
    try {
      const value = await this.fetcher();
      if (mine === this.seq) {
        this.data = value;
        this.error = null;
      }
    } catch (e) {
      if (mine === this.seq) this.error = e;
    } finally {
      if (mine === this.seq) {
        this.pending = false;
        this.loading = false;
        if (this.connected) this.arm();
        this.host.requestUpdate();
      }
    }
  }

  private onVisibility = (): void => {
    if (!document.hidden && this.connected) void this.refresh();
  };

  private wanted(): number {
    return typeof this.intervalMs === "function" ? this.intervalMs(this.data) : this.intervalMs;
  }

  // (Re)starts the interval when the wanted period changed since it was armed.
  private arm(): void {
    const ms = this.wanted();
    if (ms === this.armed) return;
    clearInterval(this.timer);
    this.armed = ms;
    this.timer = setInterval(() => {
      if (this.options.pauseWhenHidden && document.hidden) return;
      if (!this.pending) void this.refresh();
    }, ms);
  }
}
