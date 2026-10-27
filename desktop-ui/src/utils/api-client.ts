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

const quoteHeaderValue = (value: string) => value.replace(/[\r\n"]/g, (c) => (c === '"' ? "%22" : c === "\r" ? "%0D" : "%0A"));

/**
 * Serializes a form into one in-memory multipart body. The desktop webview hands requests to the app through a
 * custom protocol and does not carry file-backed bodies (a `File` inside `FormData`) across it: the server
 * received an empty file. Bytes held in memory travel as-is.
 */
export async function encodeMultipart(form: FormData): Promise<{ body: Uint8Array<ArrayBuffer>; contentType: string }> {
  const boundary = `----tessia${Date.now().toString(16)}${Math.random().toString(16).slice(2)}`;
  const text = new TextEncoder();
  const chunks: Uint8Array[] = [];
  for (const [name, value] of form.entries()) {
    const disposition = `Content-Disposition: form-data; name="${quoteHeaderValue(name)}"`;
    if (typeof value === "string") {
      chunks.push(text.encode(`--${boundary}\r\n${disposition}\r\n\r\n${value}\r\n`));
      continue;
    }
    const type = value.type || "application/octet-stream";
    chunks.push(text.encode(`--${boundary}\r\n${disposition}; filename="${quoteHeaderValue(value.name)}"\r\nContent-Type: ${type}\r\n\r\n`));
    chunks.push(new Uint8Array(await value.arrayBuffer()));
    chunks.push(text.encode("\r\n"));
  }
  chunks.push(text.encode(`--${boundary}--\r\n`));
  const body = new Uint8Array(chunks.reduce((sum, chunk) => sum + chunk.length, 0));
  let offset = 0;
  for (const chunk of chunks) {
    body.set(chunk, offset);
    offset += chunk.length;
  }
  return { body, contentType: `multipart/form-data; boundary=${boundary}` };
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
    upload: async <T>(path: string, form: FormData) => {
      const { body, contentType } = await encodeMultipart(form);
      return (await call(path, { method: "POST", headers: { "Content-Type": contentType }, body }, undefined, true)) as T;
    },
  };
}
