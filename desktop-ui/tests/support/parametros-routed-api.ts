import { vi } from "vitest";
import { createApiClient } from "../../src/utils/api-client";
import { createTessiaApi } from "../../src/utils/tessia-api";

export interface RecordedCall {
  method: string;
  /** Path with the query string, without the API base. */
  path: string;
  body: unknown;
  init: RequestInit;
}

type Reply = unknown | Response | ((call: RecordedCall) => unknown | Response);

export const API_BASE = "http://api.test";

/**
 * A real typed API (client + routes) over a fake fetch. `routes` maps "METHOD /path?query" to a
 * JSON body, a Response, or a function of the recorded call; unknown routes answer 404.
 */
export function routedApi(routes: Record<string, Reply>) {
  const calls: RecordedCall[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = new URL(String(input));
    const path = url.pathname + url.search;
    const method = init.method ?? "GET";
    let body: unknown = init.body;
    if (typeof init.body === "string") body = JSON.parse(init.body);
    const call: RecordedCall = { method, path, body, init };
    calls.push(call);
    const reply = routes[`${method} ${path}`] ?? routes[`${method} ${url.pathname}`];
    if (reply === undefined) return new Response(JSON.stringify({ error: `no route ${method} ${path}` }), { status: 404 });
    const value = typeof reply === "function" ? (reply as (c: RecordedCall) => unknown)(call) : reply;
    if (value instanceof Response) return value;
    return new Response(JSON.stringify(value), { status: 200 });
  });
  const api = createTessiaApi(createApiClient({ baseUrl: API_BASE, fetch: fetchMock as unknown as typeof fetch }));
  const callsTo = (method: string, path: string) => calls.filter((c) => c.method === method && c.path === path);
  return { api, calls, callsTo, fetchMock };
}

type Api = ReturnType<typeof createTessiaApi>;

/** The API the page under test talks to; tests assign `apiSlot.api = routedApi(...).api`. */
export const apiSlot: { api: Api | null } = { api: null };

/**
 * Stand-in for the shared `api` export of `src/api.ts`: every method forwards to `apiSlot.api` at call time,
 * so a test can swap the fake API after the page module has been imported.
 * Use it as `vi.mock("../../src/api", async () => ({ api: (await import("../support/parametros-routed-api")).forwardingApi }))`.
 */
export const forwardingApi = new Proxy({} as Api, {
  get: (_target, method: string) => (...args: unknown[]) => {
    if (!apiSlot.api) throw new Error("apiSlot.api is not set");
    return (apiSlot.api as unknown as Record<string, (...a: unknown[]) => unknown>)[method]!(...args);
  },
});

/** Awaits pending promises and Lit updates by yielding to the macrotask queue a few times. */
export async function settle(...els: Array<{ updateComplete: Promise<unknown> }>) {
  for (let i = 0; i < 3; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await Promise.all(els.map((el) => el.updateComplete));
  }
}
