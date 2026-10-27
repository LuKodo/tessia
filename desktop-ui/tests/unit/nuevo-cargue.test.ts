import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { TxNuevoCargue } from "../../src/components/nuevo-cargue";
import type { TxFileUpload } from "../../src/components/tx-file-upload";
import { dialogs } from "../../src/utils/dialogs";
import { routedApi, useApi } from "../support/ejecuciones-api";

vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);
await import("../../src/components/nuevo-cargue");

const snapshot = { id: "d1", tipo: "convenio descargar", corriendo: true, cancelable: true, iniciado_en: "2026-10-01T10:00:00Z", salida: "" };

async function flush(el: TxNuevoCargue) {
  await vi.advanceTimersByTimeAsync(0);
  await el.updateComplete;
}

async function mountCard(routes: Parameters<typeof routedApi>[0] = {}) {
  const routed = routedApi({ "POST /api/download": snapshot, ...routes });
  useApi(routed.api);
  const el = document.createElement("tx-nuevo-cargue") as TxNuevoCargue;
  document.body.append(el);
  await el.updateComplete;
  return { ...routed, el };
}

const q = <T extends HTMLElement>(el: TxNuevoCargue, sel: string) => el.shadowRoot!.querySelector<T>(sel)!;
const click = (e: Element) => e.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));

async function typeDate(el: TxNuevoCargue, testid: string, value: string) {
  const input = q(el, `[data-testid=${testid}]`).shadowRoot!.querySelector("input")!;
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  input.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
  await el.updateComplete;
}

const setDates = async (el: TxNuevoCargue, desde: string, hasta: string) => {
  await typeDate(el, "desde", desde);
  await typeDate(el, "hasta", hasta);
};

function pickFile(el: TxNuevoCargue, file: File) {
  const upload = q<TxFileUpload>(el, "[data-testid=csv-file]");
  const input = upload.shadowRoot!.querySelector<HTMLInputElement>('input[type="file"]')!;
  Object.defineProperty(input, "files", { configurable: true, value: [file] });
  input.dispatchEvent(new Event("change", { bubbles: true }));
  return upload;
}

describe("<tx-nuevo-cargue>", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
    dialogs.answerConfirm(false);
  });

  it("asks for both dates before downloading", async () => {
    const { el, calls } = await mountCard();
    click(q(el, "[data-testid=descargar]"));
    await el.updateComplete;
    expect(q(el, "[data-testid=download-msg]").textContent).toBe("elige ambas fechas");
    expect(q(el, "[data-testid=download-msg]").classList.contains("text-error")).toBe(true);
    expect(calls).toHaveLength(0);
  });

  it("downloads only after the operator confirms, sending the capitalized keys", async () => {
    const { el, callsTo } = await mountCard();
    await setDates(el, "2026-09-01", "2026-09-15");
    click(q(el, "[data-testid=descargar]"));
    expect(dialogs.confirm?.text).toBe("¿Descargar el convenio de Medicar del 2026-09-01 al 2026-09-15? Esto consulta Medicar de verdad.");

    dialogs.answerConfirm(false);
    await flush(el);
    expect(callsTo("POST", "/api/download")).toHaveLength(0);

    click(q(el, "[data-testid=descargar]"));
    dialogs.answerConfirm(true);
    await flush(el);
    const sent = callsTo("POST", "/api/download");
    expect(sent).toHaveLength(1);
    expect(sent[0]?.body).toEqual({ Desde: "2026-09-01", Hasta: "2026-09-15" });
    expect(q(el, "[data-testid=download-msg]").textContent).toBe("descarga en curso — sigue el progreso justo abajo");
    expect(q(el, "[data-testid=download-msg]").classList.contains("text-success")).toBe(true);
  });

  it("shows the job conflict (HTTP 500 {error}) as the download message", async () => {
    const { el } = await mountCard({ "POST /api/download": new Response(JSON.stringify({ error: "ya hay un proceso en curso" }), { status: 500 }) });
    await setDates(el, "2026-09-01", "2026-09-15");
    click(q(el, "[data-testid=descargar]"));
    dialogs.answerConfirm(true);
    await flush(el);
    expect(q(el, "[data-testid=download-msg]").textContent).toBe("ya hay un proceso en curso");
    expect(q(el, "[data-testid=download-msg]").classList.contains("text-error")).toBe(true);
  });

  it("validates the file before uploading the CSV without asking for dates", async () => {
    const { el, calls } = await mountCard();
    click(q(el, "[data-testid=subir-csv]"));
    await el.updateComplete;
    expect(q(el, "[data-testid=upload-msg]").textContent).toBe("elige un archivo CSV o Excel");
    expect(calls).toHaveLength(0);
  });

  it("also uploads an Excel workbook, under the same csv field", async () => {
    const { el, callsTo } = await mountCard({ "POST /api/convenio/upload": { ok: true, cargue: "Manual_20261016T120000Z_abcd1234", file_name: "Manual_20261016T120000Z_abcd1234.csv" } });
    const upload = pickFile(el, new File(["PK"], "devueltas.xlsx", { type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" }));
    await el.updateComplete;
    expect(upload.getAttribute("accept")).toBe(".csv,.xlsx");
    click(q(el, "[data-testid=subir-csv]"));
    await flush(el);

    const form = callsTo("POST", "/api/convenio/upload")[0]?.body as FormData;
    expect((form.get("csv") as File).name).toBe("devueltas.xlsx");
    expect(q(el, "[data-testid=upload-msg]").classList.contains("text-success")).toBe(true);
  });

  it("uploads only the CSV and reports the generated cargue", async () => {
    const { el, callsTo } = await mountCard({ "POST /api/convenio/upload": { ok: true, cargue: "Manual_20261016T120000Z_abcd1234", file_name: "Manual_20261016T120000Z_abcd1234.csv" } });
    const upload = pickFile(el, new File(["factura;valor\nEVS1;10"], "convenio.csv", { type: "text/csv" }));
    await el.updateComplete;
    expect(upload.files.map((f) => f.name)).toEqual(["convenio.csv"]);
    click(q(el, "[data-testid=subir-csv]"));
    await flush(el);

    const sent = callsTo("POST", "/api/convenio/upload");
    expect(sent).toHaveLength(1);
    const form = sent[0]?.body as FormData;
    expect(form.get("desde")).toBeNull();
    expect(form.get("hasta")).toBeNull();
    expect((form.get("csv") as File).name).toBe("convenio.csv");
    expect(q(el, "[data-testid=upload-msg]").textContent).toBe('Cargue "Manual_20261016T120000Z_abcd1234" creado (Manual_20261016T120000Z_abcd1234.csv). Aparecerá como SIN_PREPARAR.');
    expect(q(el, "[data-testid=upload-msg]").classList.contains("text-success")).toBe(true);
    await upload.updateComplete;
    expect(upload.files).toEqual([]);
  });

  it("only accepts .csv files in the picker", async () => {
    const { el } = await mountCard();
    const upload = q<TxFileUpload>(el, "[data-testid=csv-file]");
    expect(upload.accept).toBe(".csv,.xlsx");
    expect(upload.variant).toBe("button");
  });

  it("shows the 409 of an existing load as the upload message and keeps the file", async () => {
    const { el } = await mountCard({ "POST /api/convenio/upload": new Response(JSON.stringify({ error: "el cargue ya existe" }), { status: 409 }) });
    const upload = pickFile(el, new File(["x"], "c.csv"));
    await el.updateComplete;
    click(q(el, "[data-testid=subir-csv]"));
    await flush(el);
    expect(q(el, "[data-testid=upload-msg]").textContent).toBe("el cargue ya existe");
    expect(upload.files).toHaveLength(1);
  });
});
