import { describe, expect, it, vi } from "vitest";
import { createApiClient } from "../../src/utils/api-client";
import { createTessiaApi } from "../../src/utils/tessia-api";

function setup() {
  const fetchMock = vi.fn(async () => new Response("{}", { status: 200 }));
  const api = createTessiaApi(createApiClient({ baseUrl: "http://api.test", fetch: fetchMock as unknown as typeof fetch }));
  const last = () => {
    const [url, init] = fetchMock.mock.calls.at(-1) as unknown as [string, RequestInit | undefined];
    return { url, init: init ?? {} };
  };
  return { api, last };
}

describe("tessia api endpoints", () => {
  it("encodes path parameters", async () => {
    const { api, last } = setup();
    await api.informe("Convenio 01/09");
    expect(last().url).toBe("http://api.test/api/cargues/Convenio%2001%2F09/informe");
    await api.portal("FVS-1");
    expect(last().url).toBe("http://api.test/api/facturas/FVS-1/portal");
  });

  it("reads one invoice's full tracking row, with the cargue only when given", async () => {
    const { api, last } = setup();
    await api.seguimientoFactura("FVS-1", "Convenio 01");
    expect(last().url).toBe("http://api.test/api/facturas/FVS-1/seguimiento?cargue=Convenio+01");
    await api.seguimientoFactura("FVS1");
    expect(last().url).toBe("http://api.test/api/facturas/FVS1/seguimiento");
  });

  it("asks the consulta route only for the params given, as one URL-encoded query", async () => {
    const { api, last } = setup();
    await api.seguimientoConsulta({});
    expect(last().url).toBe("http://api.test/api/seguimiento/consulta");
    await api.seguimientoConsulta({
      factura: "FVS 1&2",
      cargue: "Convenio 01/09",
      estado: "EN_PROCESO",
      pipeline: "radicada",
      alerta: ["falta_zip", "nota_ajuste"],
      desde: "2026-10-01",
      hasta: "2026-10-31",
      sort: "fecha_factura",
      dir: "desc",
      limit: 300,
      cursor: "c1",
      agregados: false,
    });
    const url = new URL(last().url);
    expect(url.pathname).toBe("/api/seguimiento/consulta");
    expect(Object.fromEntries(url.searchParams)).toEqual({
      factura: "FVS 1&2",
      cargue: "Convenio 01/09",
      estado: "EN_PROCESO",
      pipeline: "radicada",
      alerta: "falta_zip,nota_ajuste",
      desde: "2026-10-01",
      hasta: "2026-10-31",
      sort: "fecha_factura",
      dir: "desc",
      limit: "300",
      cursor: "c1",
      agregados: "no",
    });
    expect(last().url).toContain("factura=FVS+1%262");
  });

  it("keeps limit=0 (aggregates only), drops empty alerts and leaves agregados out unless it is false", async () => {
    const { api, last } = setup();
    await api.seguimientoConsulta({ limit: 0, alerta: [], agregados: true });
    expect(last().url).toBe("http://api.test/api/seguimiento/consulta?limit=0");
  });

  it("sends the capitalized request keys of /api/run and /api/download", async () => {
    const { api, last } = setup();
    await api.runJob({ Cargue: "A", From: "derechos", To: "", Limit: 0 });
    expect(last().url).toBe("http://api.test/api/run");
    expect(JSON.parse(last().init.body as string)).toEqual({ Cargue: "A", From: "derechos", To: "", Limit: 0 });
    await api.download({ Desde: "2026-09-01", Hasta: "2026-09-02" });
    expect(last().url).toBe("http://api.test/api/download");
    expect(JSON.parse(last().init.body as string)).toEqual({ Desde: "2026-09-01", Hasta: "2026-09-02" });
  });

  it("uses lowercase keys for retry, parameters and hallazgos", async () => {
    const { api, last } = setup();
    await api.retryInvoice("A", "FVS1");
    expect(last().url).toBe("http://api.test/api/cargues/A/reintentar");
    expect(JSON.parse(last().init.body as string)).toEqual({ factura: "FVS1" });
    await api.updateHallazgo("C 1", { genera_glosa: true, descripcion: "x" });
    expect(last().url).toBe("http://api.test/api/hallazgos/C%201");
    expect(last().init.method).toBe("PUT");
  });

  it("passes the range of resumen as query parameters", async () => {
    const { api, last } = setup();
    await api.resumen({ desde: "2026-09-01", hasta: "2026-09-30" });
    expect(last().url).toBe("http://api.test/api/resumen?desde=2026-09-01&hasta=2026-09-30");
  });

  it("builds the CSV download URL of the informe without fetching", () => {
    const { api } = setup();
    expect(api.informeCsvUrl("A")).toBe("http://api.test/api/cargues/A/informe?formato=csv");
  });

  it("queries the filed invoices report, repeating cargue and skipping empty filters", async () => {
    const { api, last } = setup();
    await api.radicadas();
    expect(last().url).toBe("http://api.test/api/reportes/radicadas");
    await api.radicadas({ cargue: ["A 1", "B"], desde: "2026-09-01", hasta: "", q: "fvs 1", limite: 100 });
    expect(last().url).toBe("http://api.test/api/reportes/radicadas?cargue=A+1&cargue=B&desde=2026-09-01&q=fvs+1&limite=100");
  });

  it("builds the CSV URL of the filed invoices report without fetching and without a limit", () => {
    const { api } = setup();
    expect(api.radicadasCsvUrl()).toBe("http://api.test/api/reportes/radicadas?formato=csv");
    expect(api.radicadasCsvUrl({ cargue: ["A", "B"], desde: "2026-09-01", limite: 10 })).toBe(
      "http://api.test/api/reportes/radicadas?cargue=A&cargue=B&desde=2026-09-01&formato=csv",
    );
  });

  it("uploads the convenio CSV as multipart without date fields", async () => {
    const { api, last } = setup();
    await api.uploadConvenio({ csv: new Blob(["a"]) });
    const form = new TextDecoder().decode(last().init.body as Uint8Array);
    expect(form).not.toContain('name="desde"');
    expect(form).not.toContain('name="hasta"');
    expect(form).toContain('name="csv"; filename="blob"');
  });

  it("reads the nota técnica with a plain GET and exposes no upload for it (read-only from Redis)", async () => {
    const { api, last } = setup();
    await api.notaTecnica();
    expect(last().url).toContain("/api/nota-tecnica");
    expect(last().init.method ?? "GET").toBe("GET");
    expect("uploadNotaTecnica" in api).toBe(false);
  });

  it("portal reads use requestJSON, so a 200 with error is data", async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ factura: "F1", error: "portal caido" }), { status: 200 }));
    const api = createTessiaApi(createApiClient({ baseUrl: "", fetch: fetchMock as unknown as typeof fetch }));
    await expect(api.portal("F1")).resolves.toMatchObject({ error: "portal caido" });
    await expect(api.refreshPortal("F1")).resolves.toMatchObject({ error: "portal caido" });
  });
  it("job snapshots with an error are data: a failed or cancelled job must reach the console and the alert", async () => {
    const snapshot = { id: "j1", tipo: "proceso run", corriendo: false, error: "cancelado por el operador", salida: "" };
    const fetchMock = vi.fn(async () => new Response(JSON.stringify(snapshot), { status: 200 }));
    const api = createTessiaApi(createApiClient({ baseUrl: "", fetch: fetchMock as unknown as typeof fetch }));
    await expect(api.jobsCurrent()).resolves.toMatchObject({ error: "cancelado por el operador" });
    await expect(api.retryInvoice("A", "FVS1")).resolves.toMatchObject({ error: "cancelado por el operador" });
  });

  it("a job conflict arrives as HTTP 500 {error} and is thrown with its message", async () => {
    const fetchMock = vi.fn(async () => new Response(JSON.stringify({ error: "ya hay un proceso en curso" }), { status: 500 }));
    const api = createTessiaApi(createApiClient({ baseUrl: "", fetch: fetchMock as unknown as typeof fetch }));
    await expect(api.runJob({ Cargue: "A", From: "", To: "", Limit: 0 })).rejects.toMatchObject({
      status: 500,
      message: "ya hay un proceso en curso",
    });
  });

  it("jobsHistory returns null as is when no job finished yet", async () => {
    const fetchMock = vi.fn(async () => new Response("null", { status: 200 }));
    const api = createTessiaApi(createApiClient({ baseUrl: "", fetch: fetchMock as unknown as typeof fetch }));
    await expect(api.jobsHistory()).resolves.toBeNull();
  });
});
