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
 * A real typed API (client + routes) over a fake fetch, so no test ever reaches a real service.
 * `routes` maps "METHOD /path?query" (or just "METHOD /path") to a JSON body, a Response or a
 * function of the recorded call; unknown routes answer 404.
 */
export function routedApi(routes: Record<string, Reply>) {
  const calls: RecordedCall[] = [];
  const fetchMock = vi.fn(async (input: RequestInfo | URL, init: RequestInit = {}) => {
    const url = new URL(String(input));
    const path = url.pathname + url.search;
    const method = init.method ?? "GET";
    let body: unknown = init.body;
    if (typeof init.body === "string") body = JSON.parse(init.body);
    calls.push({ method, path, body, init });
    const reply = routes[`${method} ${path}`] ?? routes[`${method} ${url.pathname}`];
    if (reply === undefined) return new Response(JSON.stringify({ error: `sin ruta ${method} ${path}` }), { status: 404 });
    const value = typeof reply === "function" ? (reply as (c: RecordedCall) => unknown)(calls[calls.length - 1]!) : reply;
    if (value instanceof Response) return value;
    return new Response(JSON.stringify(value), { status: 200 });
  });
  const api = createTessiaApi(createApiClient({ baseUrl: API_BASE, fetch: fetchMock as unknown as typeof fetch }));
  const callsTo = (method: string, path: string) => calls.filter((c) => c.method === method && c.path === path);
  return { api, calls, callsTo, fetchMock };
}
