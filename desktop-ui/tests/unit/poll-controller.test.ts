import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LitElement, html } from "lit";
import { PollController, type PollOptions } from "../../src/controllers/poll-controller";
import type { JobSnapshot } from "../../src/types/api";
import { JobPollController } from "../../src/controllers/job-poll-controller";

type Interval<T> = number | ((data: T | null) => number);

class PollHost<T> extends LitElement {
  poll!: PollController<T>;
  renders = 0;
  render() {
    this.renders++;
    return html`${this.poll.loading ? "loading" : JSON.stringify(this.poll.data)}`;
  }
}
customElements.define("tx-poll-host", PollHost);

/** Mounts a host wired to a PollController; the first tick starts on connect. */
async function mountPoll<T>(fetcher: () => Promise<T>, intervalMs: Interval<T>, options?: PollOptions) {
  const el = document.createElement("tx-poll-host") as PollHost<T>;
  el.poll = new PollController(el, fetcher, intervalMs, options);
  document.body.append(el);
  await el.updateComplete;
  return { el, poll: el.poll };
}

const settle = () => vi.advanceTimersByTimeAsync(0);

describe("PollController", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("runs once on connect and then every interval", async () => {
    const fetcher = vi.fn(async () => 1);
    await mountPoll(fetcher, 5000);
    await settle();
    expect(fetcher).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(5000);
    expect(fetcher).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(10000);
    expect(fetcher).toHaveBeenCalledTimes(4);
  });

  it("exposes data and clears loading after the first tick, re-rendering the host", async () => {
    const el = document.createElement("tx-poll-host") as PollHost<{ n: number }>;
    const poll = (el.poll = new PollController(el, async () => ({ n: 1 }), 1000));
    expect(poll.loading).toBe(true);
    expect(poll.data).toBeNull();
    document.body.append(el);
    await settle();
    await el.updateComplete;
    expect(poll.data).toEqual({ n: 1 });
    expect(poll.error).toBeNull();
    expect(poll.loading).toBe(false);
    expect(el.shadowRoot!.textContent).toBe('{"n":1}');
  });

  it("keeps the last data on a failed tick and clears the error on recovery", async () => {
    let call = 0;
    const { poll } = await mountPoll(async () => {
      call++;
      if (call === 2) throw new Error("down");
      return call;
    }, 1000);
    await settle();
    expect(poll.data).toBe(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(poll.data).toBe(1);
    expect((poll.error as Error).message).toBe("down");
    await vi.advanceTimersByTimeAsync(1000);
    expect(poll.data).toBe(3);
    expect(poll.error).toBeNull();
  });

  it("stops the interval and ignores late results after disconnect", async () => {
    let resolveSlow!: (v: number) => void;
    let call = 0;
    const fetcher = vi.fn(() => {
      call++;
      return call === 2 ? new Promise<number>((r) => (resolveSlow = r)) : Promise.resolve(call);
    });
    const { el, poll } = await mountPoll(fetcher, 1000);
    await settle();
    await vi.advanceTimersByTimeAsync(1000); // second tick starts and stays pending
    el.remove();
    resolveSlow(99);
    await settle();
    expect(poll.data).toBe(1);
    await vi.advanceTimersByTimeAsync(10000);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("refresh runs one tick on demand", async () => {
    const fetcher = vi.fn(async () => "x");
    const { poll } = await mountPoll(fetcher, 60000);
    await settle();
    await poll.refresh();
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("re-evaluates a function interval after each tick", async () => {
    const fetcher = vi.fn(async () => (fetcher.mock.calls.length < 3 ? "busy" : "idle"));
    await mountPoll(fetcher, (data) => (data === "busy" ? 5000 : 1000));
    await settle();
    expect(fetcher).toHaveBeenCalledTimes(1); // data is now "busy": next tick in 5 s
    await vi.advanceTimersByTimeAsync(4999);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetcher).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(5000);
    expect(fetcher).toHaveBeenCalledTimes(3); // this tick answers "idle": back to 1 s
    await vi.advanceTimersByTimeAsync(3000);
    expect(fetcher).toHaveBeenCalledTimes(6);
  });

  it("uses the interval of an empty poll (no data yet) for the first wait", async () => {
    const fetcher = vi.fn(async () => 1);
    await mountPoll(fetcher, (data) => (data === null ? 100 : 1000));
    await settle();
    await vi.advanceTimersByTimeAsync(999);
    expect(fetcher).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("stops re-arming after disconnect", async () => {
    const fetcher = vi.fn(async () => 1);
    const { el } = await mountPoll(fetcher, () => 1000);
    await settle();
    el.remove();
    await vi.advanceTimersByTimeAsync(10000);
    expect(fetcher).toHaveBeenCalledTimes(1);
  });

  it("skips interval ticks while a request is still in flight", async () => {
    let resolveFirst!: (v: number) => void;
    const fetcher = vi.fn(() => (fetcher.mock.calls.length === 1 ? new Promise<number>((r) => (resolveFirst = r)) : Promise.resolve(2)));
    const { poll } = await mountPoll(fetcher, 1000);
    await vi.advanceTimersByTimeAsync(5000); // five timer ticks, the first request never answered
    expect(fetcher).toHaveBeenCalledTimes(1);
    resolveFirst(1);
    await settle();
    expect(poll.data).toBe(1);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetcher).toHaveBeenCalledTimes(2);
  });

  it("an explicit refresh supersedes an in-flight request: the latest answer wins", async () => {
    const resolvers: Array<(v: string) => void> = [];
    const fetcher = vi.fn(() => new Promise<string>((r) => resolvers.push(r)));
    const { poll } = await mountPoll(fetcher, 60000);
    const second = poll.refresh();
    expect(fetcher).toHaveBeenCalledTimes(2);
    resolvers[1]!("new");
    await second;
    resolvers[0]!("stale");
    await settle();
    expect(poll.data).toBe("new");
  });

  it("starts again when the host is reconnected", async () => {
    const fetcher = vi.fn(async () => 1);
    const { el } = await mountPoll(fetcher, 1000);
    await settle();
    el.remove();
    document.body.append(el);
    await settle();
    expect(fetcher).toHaveBeenCalledTimes(2);
    await vi.advanceTimersByTimeAsync(1000);
    expect(fetcher).toHaveBeenCalledTimes(3);
  });

  describe("pauseWhenHidden", () => {
    const setHidden = (hidden: boolean) => {
      Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
      document.dispatchEvent(new Event("visibilitychange"));
    };
    beforeEach(() => setHidden(false));
    afterEach(() => setHidden(false));

    it("skips interval ticks while the page is hidden", async () => {
      const fetcher = vi.fn(async () => 1);
      await mountPoll(fetcher, 1000, { pauseWhenHidden: true });
      await settle();
      expect(fetcher).toHaveBeenCalledTimes(1);
      setHidden(true);
      await vi.advanceTimersByTimeAsync(10000);
      expect(fetcher).toHaveBeenCalledTimes(1);
    });

    it("refreshes immediately when the page becomes visible again, then resumes the interval", async () => {
      const fetcher = vi.fn(async () => 1);
      await mountPoll(fetcher, 1000, { pauseWhenHidden: true });
      await settle();
      setHidden(true);
      await vi.advanceTimersByTimeAsync(5000);
      setHidden(false);
      await settle();
      expect(fetcher).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(1000);
      expect(fetcher).toHaveBeenCalledTimes(3);
    });

    it("does not refresh on hiding the page", async () => {
      const fetcher = vi.fn(async () => 1);
      await mountPoll(fetcher, 60000, { pauseWhenHidden: true });
      await settle();
      setHidden(true);
      await settle();
      expect(fetcher).toHaveBeenCalledTimes(1);
    });

    it("keeps ticking while hidden by default", async () => {
      const fetcher = vi.fn(async () => 1);
      await mountPoll(fetcher, 1000);
      await settle();
      setHidden(true);
      await vi.advanceTimersByTimeAsync(3000);
      expect(fetcher).toHaveBeenCalledTimes(4);
      setHidden(false);
      await settle();
      expect(fetcher).toHaveBeenCalledTimes(4);
    });

    it("job polling never pauses: the job alert must fire from a hidden tab", async () => {
      const jobsCurrent = vi.fn(async () => ({ corriendo: false }) as JobSnapshot);
      const el = document.createElement("tx-poll-host") as PollHost<JobSnapshot>;
      el.poll = new JobPollController(el, { jobsCurrent });
      document.body.append(el);
      await settle();
      setHidden(true);
      await vi.advanceTimersByTimeAsync(4000);
      expect(jobsCurrent).toHaveBeenCalledTimes(3);
    });

    it("stops listening to visibility after disconnect", async () => {
      const fetcher = vi.fn(async () => 1);
      const { el } = await mountPoll(fetcher, 60000, { pauseWhenHidden: true });
      await settle();
      el.remove();
      setHidden(true);
      setHidden(false);
      await settle();
      expect(fetcher).toHaveBeenCalledTimes(1);
    });
  });
});
