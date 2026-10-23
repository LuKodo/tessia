// Typed fetch helpers for the `tessia serve` API. Same semantics as the old
// webui-src/src/api.js, plus a typed ApiError:
//  - getJSON throws on a non-2xx AND on a 2xx body carrying `error`.
//  - requestJSON throws only on a non-2xx (the portal endpoints answer 200 with
//    a readable `error` next to usable data).

export type QueryValue = string | number | boolean | undefined | null;
export type Query = Record<string, QueryValue>;

export class ApiError extends Error {
  readonly status: number;
  readonly body: unknown;

  constructor(status: number, message: string, body?: unknown, options?: { cause?: unknown }) {
    super(message, options);
    this.name = "ApiError";
    this.status = status;
    this.body = body;
  }
}

/** Joins the API base and a path; an empty base keeps the URL same-origin relative. */
export function buildUrl(baseUrl: string, path: string, query?: Query): string {
  const base = baseUrl.replace(/\/+$/, "");
  const url = `${base}/${path.replace(/^\/+/, "")}`;
  const params = new URLSearchParams();
  for (const [key, value] of Object.entries(query ?? {})) {
    if (value === undefined || value === null || value === "") continue;
    params.set(key, String(value));
  }
  const qs = params.toString();
  return qs ? `${url}?${qs}` : url;
}

export interface ApiClientOptions {
  baseUrl: string;
  /** Defaults to the global fetch, resolved at call time. */
  fetch?: typeof fetch;
}

export interface ApiClient {
  url(path: string, query?: Query): string;
  getJSON<T>(path: string, init?: RequestInit, query?: Query): Promise<T>;
  requestJSON<T>(path: string, init?: RequestInit, query?: Query): Promise<T>;
  sendJSON<T>(method: "POST" | "PUT", path: string, body?: unknown, options?: { lenient?: boolean }): Promise<T>;
  upload<T>(path: string, form: FormData): Promise<T>;
}

function errorMessageOf(body: unknown): string {
  if (body && typeof body === "object" && "error" in body) {
    const e = (body as { error: unknown }).error;
    if (typeof e === "string" && e) return e;
  }
  return "";
}

export function createApiClient(options: ApiClientOptions): ApiClient {
  const doFetch = (url: string, init?: RequestInit) => (options.fetch ?? globalThis.fetch)(url, init);

  async function call(path: string, init: RequestInit | undefined, query: Query | undefined, strict: boolean): Promise<unknown> {
    const url = buildUrl(options.baseUrl, path, query);
    let response: Response;
    try {
      response = await doFetch(url, init);
    } catch (cause) {
      throw new ApiError(0, `No se pudo conectar con el servicio (${url})`, undefined, { cause });
    }
    const text = await response.text().catch(() => "");
    let body: unknown;
    let isJSON = false;
    try {
      body = text ? JSON.parse(text) : {};
      isJSON = true;
    } catch {
      body = text;
    }
    const bodyError = isJSON ? errorMessageOf(body) : "";
    if (!response.ok) {
      const plain = !isJSON ? text.trim() : "";
      throw new ApiError(response.status, bodyError || plain || response.statusText || `HTTP ${response.status}`, body);
    }
    if (strict && bodyError) throw new ApiError(response.status, bodyError, body);
    return isJSON ? body : {};
  }

  return {
    url: (path, query) => buildUrl(options.baseUrl, path, query),
    getJSON: <T>(path: string, init?: RequestInit, query?: Query) => call(path, init, query, true) as Promise<T>,
    requestJSON: <T>(path: string, init?: RequestInit, query?: Query) => call(path, init, query, false) as Promise<T>,
    sendJSON: <T>(method: "POST" | "PUT", path: string, body?: unknown, opts?: { lenient?: boolean }) =>
      call(
        path,
        {
          method,
          headers: { "Content-Type": "application/json" },
          body: body === undefined ? undefined : JSON.stringify(body),
        },
        undefined,
        !opts?.lenient,
      ) as Promise<T>,
    // No Content-Type: the browser sets the multipart boundary itself.
    upload: <T>(path: string, form: FormData) => call(path, { method: "POST", body: form }, undefined, true) as Promise<T>,
  };
}
