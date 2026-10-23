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

type Reply = unknown | Response | ((call: RecordedCall) => unknown | Response | Promise<unknown | Response>);

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
    if (reply === undefined) return new Response(JSON.stringify({ error: `sin ruta ${method} ${path}` }), { status: 404 });
    const value = typeof reply === "function" ? await (reply as (c: RecordedCall) => unknown)(call) : reply;
    if (value instanceof Response) return value;
    return new Response(JSON.stringify(value), { status: 200 });
  });
  const api = createTessiaApi(createApiClient({ baseUrl: API_BASE, fetch: fetchMock as unknown as typeof fetch }));
  const callsTo = (method: string, path: string) => calls.filter((c) => c.method === method && c.path === path);
  return { api, calls, callsTo, fetchMock };
}
