import { vi } from "vitest";
import { createApiClient } from "../../src/utils/api-client";
import { createTessiaApi, type TessiaApi } from "../../src/utils/tessia-api";

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
    if (reply === undefined) return new Response(JSON.stringify({ error: `sin ruta ${method} ${path}` }), { status: 404 });
    const value = typeof reply === "function" ? (reply as (c: RecordedCall) => unknown)(call) : reply;
    if (value instanceof Response) return value;
    return new Response(JSON.stringify(value), { status: 200 });
  });
  const api = createTessiaApi(createApiClient({ baseUrl: API_BASE, fetch: fetchMock as unknown as typeof fetch }));
  const callsTo = (method: string, path: string) => calls.filter((c) => c.method === method && c.path === path);
  return { api, calls, callsTo, fetchMock };
}

/** A TessiaApi whose methods are vitest mocks; override the ones a test cares about. */
export function fakeApi(overrides: Partial<Record<keyof TessiaApi, unknown>> = {}) {
  const base = {
    jobsCurrent: vi.fn(async () => ({ id: "", tipo: "", corriendo: false, cancelable: false, iniciado_en: "0001-01-01T00:00:00Z", salida: "" })),
    jobsHistory: vi.fn(async () => null),
    cargues: vi.fn(async () => []),
    cancelJob: vi.fn(async () => ({ ok: true })),
  };
  return { ...base, ...overrides } as unknown as TessiaApi & typeof base;
}

// The seam of the elements: they import `api` from `src/api`. A test replaces it with
//   vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
// and calls `useApi(...)` before mounting.
let current: unknown = null;
export function useApi(api: unknown): void {
  current = api;
}
export const apiModule = {
  api: new Proxy({}, { get: (_target, key) => (current as Record<string | symbol, unknown>)?.[key] }) as TessiaApi,
};
