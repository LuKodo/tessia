import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/nota-tecnica";
import type { TxNotaTecnica } from "../../src/components/nota-tecnica";
import type { TxFileUpload } from "../../src/components/tx-file-upload";
import { dialogs } from "../../src/utils/dialogs";
import { apiSlot, routedApi, settle } from "../support/parametros-routed-api";

vi.mock("../../src/api", async () => ({ api: (await import("../support/parametros-routed-api")).forwardingApi }));

type Routes = Parameters<typeof routedApi>[0];

async function mountCard(routes: Routes) {
  const routed = routedApi(routes);
  apiSlot.api = routed.api;
  const el = document.createElement("tx-nota-tecnica") as TxNotaTecnica;
  document.body.append(el);
  await settle(el);
  return { ...routed, el };
}
type Card = Awaited<ReturnType<typeof mountCard>>["el"];

const q = <T extends Element>(el: Card, selector: string) => el.shadowRoot!.querySelector<T>(selector)!;
const status = (el: Card) => q(el, "[data-testid=nota-status]").textContent!.trim();
const message = (el: Card) => q(el, "[data-testid=nota-msg]");
const button = (el: Card) => q<HTMLElement & { disabled: boolean }>(el, "[data-testid=nota-reemplazar]");
const upload = (el: Card) => q<TxFileUpload>(el, "tx-file-upload");
const input = (el: Card) => upload(el).shadowRoot!.querySelector<HTMLInputElement>('input[type="file"]')!;

async function pickFile(el: Card, name = "Nota tecnica.xlsx") {
  Object.defineProperty(input(el), "files", { configurable: true, value: [new File(["xlsx"], name)] });
  input(el).dispatchEvent(new Event("change", { bubbles: true }));
  await settle(el, upload(el));
}

afterEach(() => {
  dialogs.answerConfirm(false);
  document.body.innerHTML = "";
});

describe("<tx-nota-tecnica>", () => {
  it("shows the current file with its size in MiB", async () => {
    const { el } = await mountCard({
      "GET /api/nota-tecnica": { exists: true, file_name: "Nota tecnica.xlsx", size_bytes: 3 * 1024 * 1024 },
    });
    expect(status(el)).toBe("Nota tecnica.xlsx · 3.0 MiB");
    expect(message(el).textContent).toContain("tiene prioridad sobre las copias junto a cada cargue");
  });

  it("says it is verifying before the first answer", async () => {
    apiSlot.api = { notaTecnica: () => new Promise(() => {}) } as never;
    const el = document.createElement("tx-nota-tecnica") as TxNotaTecnica;
    document.body.append(el);
    await el.updateComplete;
    expect(status(el)).toBe("verificando…");
    expect(message(el).textContent).toContain("Se prioriza Nota tecnica.xlsx en paths.automatic");
  });

  it("says when no shared file is loaded", async () => {
    const { el } = await mountCard({ "GET /api/nota-tecnica": { exists: false, file_name: "Nota tecnica.xlsx" } });
    expect(status(el)).toBe("No cargada");
    expect(message(el).textContent).toContain("No hay un archivo compartido");
  });

  it("shows the error when the status cannot be read", async () => {
    const { el } = await mountCard({ "GET /api/nota-tecnica": new Response(JSON.stringify({ error: "sin carpeta" }), { status: 500 }) });
    expect(status(el)).toBe("No disponible");
    expect(message(el).textContent!.trim()).toBe("sin carpeta");
    expect(message(el).classList.contains("error")).toBe(true);
  });

  it("only accepts XLSX workbooks in the real file input", async () => {
    const { el } = await mountCard({ "GET /api/nota-tecnica": { exists: false, file_name: "Nota tecnica.xlsx" } });
    expect(input(el).getAttribute("accept")).toContain(".xlsx");
  });

  it("keeps the replace button disabled until a file is chosen", async () => {
    const { el } = await mountCard({ "GET /api/nota-tecnica": { exists: false, file_name: "Nota tecnica.xlsx" } });
    expect(button(el).disabled).toBe(true);
    await pickFile(el);
    expect(button(el).disabled).toBe(false);
  });

  it("uploads the workbook as multipart field `workbook` only after confirmation, then refreshes the status", async () => {
    let size = 1024 * 1024;
    const { el, callsTo } = await mountCard({
      "GET /api/nota-tecnica": () => ({ exists: true, file_name: "Nota tecnica.xlsx", size_bytes: size }),
      "POST /api/nota-tecnica": () => {
        size = 2 * 1024 * 1024;
        return { ok: true, file_name: "Nota tecnica.xlsx", version_id: 7, created: true, already_current: false };
      },
    });
    await pickFile(el, "nueva.xlsx");
    button(el).click();
    await settle(el);
    expect(dialogs.confirm?.text).toBe(
      '¿Reemplazar la nota técnica compartida con "nueva.xlsx"? El archivo anterior se conserva si el nuevo no es válido.',
    );

    dialogs.answerConfirm(false);
    await settle(el);
    expect(callsTo("POST", "/api/nota-tecnica")).toHaveLength(0);

    button(el).click();
    await settle(el);
    dialogs.answerConfirm(true);
    await settle(el);
    const sent = callsTo("POST", "/api/nota-tecnica");
    expect(sent).toHaveLength(1);
    const workbook = (sent[0]!.body as FormData).get("workbook") as File;
    expect(workbook).toBeInstanceOf(File);
    expect(workbook.name).toBe("nueva.xlsx");
    expect(callsTo("GET", "/api/nota-tecnica")).toHaveLength(2);
    expect(status(el)).toBe("Nota tecnica.xlsx · 2.0 MiB");
    expect(message(el).textContent!.trim()).toBe("Nota técnica guardada (versión 7).");
    expect(message(el).classList.contains("ok")).toBe(true);
    expect(upload(el).files).toEqual([]);
    expect(button(el).disabled).toBe(true);
  });

  it("shows the server's rejection and keeps the current file", async () => {
    const { el } = await mountCard({
      "GET /api/nota-tecnica": { exists: true, file_name: "Nota tecnica.xlsx", size_bytes: 1024 * 1024 },
      "POST /api/nota-tecnica": new Response(JSON.stringify({ error: "libro inválido" }), { status: 400 }),
    });
    await pickFile(el);
    button(el).click();
    await settle(el);
    dialogs.answerConfirm(true);
    await settle(el);
    expect(message(el).textContent!.trim()).toBe("libro inválido");
    expect(message(el).classList.contains("error")).toBe(true);
    expect(status(el)).toBe("Nota tecnica.xlsx · 1.0 MiB");
    expect(button(el).disabled).toBe(false);
  });

  it.each([
    ["a new version", { version_id: 9, created: true, already_current: false }, "Nota técnica guardada (versión 9)."],
    ["a version that was already stored", { version_id: 4, created: false, already_current: false }, "Esta nota técnica ya estaba cargada (versión 4)."],
    ["the file that is already current", { version_id: 4, created: false, already_current: true }, "Esta nota técnica ya es la vigente (versión 4)."],
  ])("tells the person the upload stored %s", async (_name, outcome, text) => {
    const { el } = await mountCard({
      "GET /api/nota-tecnica": { exists: true, file_name: "Nota tecnica.xlsx", size_bytes: 1024 * 1024 },
      "POST /api/nota-tecnica": { ok: true, file_name: "Nota tecnica.xlsx", ...outcome },
    });
    await pickFile(el);
    button(el).click();
    await settle(el);
    dialogs.answerConfirm(true);
    await settle(el);
    expect(message(el).textContent!.trim()).toBe(text);
    expect(message(el).classList.contains("ok")).toBe(true);
  });
});
