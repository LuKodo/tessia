import { describe, expect, it, vi } from "vitest";
import { ApiError, buildUrl, createApiClient } from "../../src/utils/api-client";

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "Content-Type": "application/json" },
    ...init,
  });
}

/** Awaits a promise that must reject with an ApiError. */
async function rejection(p: Promise<unknown>): Promise<ApiError> {
  try {
    await p;
  } catch (e) {
    return e as ApiError;
  }
  throw new Error("expected the promise to reject");
}

function clientWith(response: Response | (() => Response), baseUrl = "http://api.test") {
  const fetchMock = vi.fn(async () => (typeof response === "function" ? response() : response));
  return { client: createApiClient({ baseUrl, fetch: fetchMock as unknown as typeof fetch }), fetchMock };
}

describe("buildUrl", () => {
  it("joins the base and the path with exactly one slash", () => {
    expect(buildUrl("http://localhost:8090", "/api/health")).toBe("http://localhost:8090/api/health");
    expect(buildUrl("http://localhost:8090/", "/api/health")).toBe("http://localhost:8090/api/health");
    expect(buildUrl("http://localhost:8090//", "api/health")).toBe("http://localhost:8090/api/health");
  });

  it("falls back to a same-origin path when the base is empty", () => {
    expect(buildUrl("", "/api/health")).toBe("/api/health");
  });

  it("appends a query string and skips empty values", () => {
    const url = buildUrl("http://h", "/api/resumen", { desde: "2026-09-01", hasta: "", limit: 5, x: undefined });
    expect(url).toBe("http://h/api/resumen?desde=2026-09-01&limit=5");
  });

  it("omits the question mark when no query value remains", () => {
    expect(buildUrl("http://h", "/api/log", { limit: undefined })).toBe("http://h/api/log");
  });
});

describe("ApiError", () => {
  it("keeps status, message and body and is an Error", () => {
    const err = new ApiError(409, "ya existe", { error: "ya existe" });
    expect(err).toBeInstanceOf(Error);
    expect(err.name).toBe("ApiError");
    expect(err.status).toBe(409);
    expect(err.message).toBe("ya existe");
    expect(err.body).toEqual({ error: "ya existe" });
  });
});

describe("error mapping", () => {
  it("uses the {error} message of a JSON error body", async () => {
    const { client } = clientWith(jsonResponse({ error: "no se pudo leer el log" }, { status: 500 }));
    const err = await rejection(client.getJSON("/api/run"));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(500);
    expect(err.message).toBe("no se pudo leer el log");
  });

  it.each([
    [400, "cargue invalido \"../x\""],
    [404, "dashboard: cargue not found: \"nope\""],
    [405, "método no permitido"],
    [409, "ya hay un proceso en curso (proceso run, iniciado 10:00:00): espera a que termine"],
    [503, "job runner is shutting down"],
  ])("keeps the {error} message and the status of a %i answer (the API never uses 500 for these)", async (status, error) => {
    const { client } = clientWith(jsonResponse({ error }, { status }));
    const err = await rejection(client.sendJSON("POST", "/api/run", {}));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(status);
    expect(err.message).toBe(error);
  });

  it("falls back to the plain-text body", async () => {
    const { client } = clientWith(new Response("method not allowed\n", { status: 405, statusText: "Method Not Allowed" }));
    const err = await rejection(client.requestJSON("/api/nota-tecnica"));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(405);
    expect(err.message).toBe("method not allowed");
  });

  it("falls back to the status text, then to the status code, without a body", async () => {
    const withText = clientWith(new Response("", { status: 502, statusText: "Bad Gateway" }));
    expect((await rejection(withText.client.getJSON("/x"))).message).toBe("Bad Gateway");
    const bare = clientWith(new Response("", { status: 503, statusText: "" }));
    expect((await rejection(bare.client.getJSON("/x"))).message).toBe("HTTP 503");
  });

  it("wraps a network failure as ApiError with status 0", async () => {
    const failing = vi.fn(async () => {
      throw new TypeError("fetch failed");
    });
    const client = createApiClient({ baseUrl: "http://api.test", fetch: failing as unknown as typeof fetch });
    const err = await rejection(client.getJSON("/api/health"));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(0);
    expect(err.message).toContain("http://api.test");
    expect(err.cause).toBeInstanceOf(TypeError);
  });
});

describe("getJSON vs requestJSON", () => {
  const portalBody = { factura: "F1", error: "portal no disponible" };

  it("getJSON throws on a 200 whose body carries an error", async () => {
    const { client } = clientWith(jsonResponse(portalBody));
    const err = await rejection(client.getJSON("/api/facturas/F1/portal"));
    expect(err).toBeInstanceOf(ApiError);
    expect(err.status).toBe(200);
    expect(err.message).toBe("portal no disponible");
  });

  it("requestJSON returns the body of a 200 even when it carries an error", async () => {
    const { client } = clientWith(jsonResponse(portalBody));
    expect(await client.requestJSON("/api/facturas/F1/portal")).toEqual(portalBody);
  });

  it("both return the body of a clean 200 and treat an array body as data", async () => {
    const rows = [{ nombre: "Medicar", disponible: true }];
    expect(await clientWith(jsonResponse(rows)).client.getJSON("/api/health")).toEqual(rows);
    expect(await clientWith(jsonResponse(rows)).client.requestJSON("/api/health")).toEqual(rows);
  });

  it("both throw on a non-2xx", async () => {
    for (const method of ["getJSON", "requestJSON"] as const) {
      const { client } = clientWith(jsonResponse({ error: "boom" }, { status: 409 }));
      await expect(client[method]("/x")).rejects.toMatchObject({ status: 409, message: "boom" });
    }
  });
});

describe("requests", () => {
  it("builds the URL from the base, forwards the init and sends no credentials", async () => {
    const { client, fetchMock } = clientWith(jsonResponse({ ok: true }));
    await client.getJSON("/api/jobs/cancel", { method: "POST" });
    const [url, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe("http://api.test/api/jobs/cancel");
    expect(init.method).toBe("POST");
    expect(init.credentials).toBeUndefined();
  });

  it("sendJSON serializes the body and sets the content type", async () => {
    const { client, fetchMock } = clientWith(jsonResponse({ ok: true }));
    await client.sendJSON("PUT", "/api/parameters", { wave_size: 3 });
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(init.method).toBe("PUT");
    expect(init.body).toBe('{"wave_size":3}');
    expect(new Headers(init.headers).get("Content-Type")).toBe("application/json");
  });

  it("upload posts one in-memory multipart body with its own boundary", async () => {
    const { client, fetchMock } = clientWith(jsonResponse({ ok: true }));
    const form = new FormData();
    form.append("desde", "2026-09-01");
    form.append("csv", new File(["a;b\n1;ñ"], 'con "comillas".csv', { type: "text/csv" }));
    await client.upload("/api/convenio/upload", form);
    const [, init] = fetchMock.mock.calls[0] as unknown as [string, RequestInit];
    expect(init.method).toBe("POST");
    const contentType = new Headers(init.headers).get("Content-Type") ?? "";
    const boundary = /^multipart\/form-data; boundary=(.+)$/.exec(contentType)?.[1];
    expect(boundary).toBeTruthy();
    expect(init.body).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(init.body as Uint8Array)).toBe(
      `--${boundary}\r\nContent-Disposition: form-data; name="desde"\r\n\r\n2026-09-01\r\n` +
        `--${boundary}\r\nContent-Disposition: form-data; name="csv"; filename="con %22comillas%22.csv"\r\nContent-Type: text/csv\r\n\r\na;b\n1;ñ\r\n` +
        `--${boundary}--\r\n`,
    );
  });
});
