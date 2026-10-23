import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/pages/page-hallazgos";
import type { PageHallazgos } from "../../src/pages/page-hallazgos";
import { ApiError } from "../../src/utils/api-client";
import { catalog, deferred, fakeHallazgosApi } from "../support/hallazgos-fixtures";
import type { HallazgosApi } from "../../src/controllers/hallazgos-controller";
import type { HallazgosResponse } from "../../src/types/api";

const flush = () => vi.advanceTimersByTimeAsync(0);

async function mountPage(api: HallazgosApi) {
  const el = document.createElement("tx-page-hallazgos") as PageHallazgos;
  el.api = api;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const $ = <T extends Element>(root: ParentNode, selector: string) => root.querySelector<T>(selector)!;
const tabla = (el: PageHallazgos) => $(el.shadowRoot!, "tx-hallazgos-tabla") as HTMLElement & { shadowRoot: ShadowRoot; updateComplete: Promise<boolean> };
const codes = (el: PageHallazgos) => [...tabla(el).shadowRoot.querySelectorAll("td.codigo")].map((td) => td.textContent!.trim());

function typeQuery(el: PageHallazgos, value: string) {
  const search = $<HTMLElement>(el.shadowRoot!, "tx-input");
  const inner = search.shadowRoot!.querySelector("input")!;
  inner.value = value;
  inner.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
}

describe("<tx-page-hallazgos>", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("shows the title, a progress bar while loading, then the table with its summary", async () => {
    const pending = deferred<HallazgosResponse>();
    const el = await mountPage(fakeHallazgosApi({ hallazgos: vi.fn(() => pending.promise) }));
    expect($(el.shadowRoot!, "h2").textContent).toBe("Hallazgos");
    expect(el.shadowRoot!.textContent).toContain("Hallazgos que devuelve el portal de Mutual Ser");
    expect($(el.shadowRoot!, "tx-progress")).not.toBeNull();
    expect(el.shadowRoot!.querySelector("tx-hallazgos-tabla")).toBeNull();

    pending.resolve({ hallazgos: catalog, total: 2 });
    await flush();
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("tx-progress")).toBeNull();
    expect($(el.shadowRoot!, ".resumen").textContent).toBe("2 de 2 hallazgos");
    expect(codes(el)).toEqual(["RVC012", "RVC040"]);
    expect(document.title).toBe("Hallazgos — Tessia");
  });

  it("filters by the search box and clears it with the X button", async () => {
    const el = await mountPage(fakeHallazgosApi());
    await flush();
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("tx-button[aria-label='Limpiar búsqueda']")).toBeNull();

    typeQuery(el, "cups");
    await el.updateComplete;
    await tabla(el).updateComplete;
    expect($(el.shadowRoot!, ".resumen").textContent).toBe("1 de 2 hallazgos");
    expect(codes(el)).toEqual(["RVC040"]);

    typeQuery(el, "zzz");
    await el.updateComplete;
    await tabla(el).updateComplete;
    expect(tabla(el).shadowRoot.textContent).toContain("Ningún hallazgo coincide con la búsqueda.");

    $<HTMLElement>(el.shadowRoot!, "tx-button[aria-label='Limpiar búsqueda']").click();
    await el.updateComplete;
    await tabla(el).updateComplete;
    expect(codes(el)).toEqual(["RVC012", "RVC040"]);
    expect($<HTMLElement & { value: string }>(el.shadowRoot!, "tx-input").value).toBe("");
  });

  it("shows the load error with a retry button that reloads", async () => {
    const hallazgos = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(503, "sin base de datos"))
      .mockResolvedValueOnce({ hallazgos: catalog, total: 2 });
    const el = await mountPage(fakeHallazgosApi({ hallazgos }));
    await flush();
    await el.updateComplete;
    const alert = $<HTMLElement & { heading: string; description: string }>(el.shadowRoot!, "tx-alert");
    expect(alert.heading).toBe("No se pudieron cargar los hallazgos");
    expect(alert.description).toBe("sin base de datos");

    $<HTMLElement>(alert, "tx-button").click();
    await flush();
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("tx-alert")).toBeNull();
    expect(codes(el)).toEqual(["RVC012", "RVC040"]);
  });

  it("saves a toggled checkbox end to end and shows Guardado", async () => {
    const updateHallazgo = vi.fn(async (codigo: string, body: object) => ({ ...catalog[0]!, codigo, ...body }));
    const el = await mountPage(fakeHallazgosApi({ updateHallazgo }));
    await flush();
    await el.updateComplete;
    const box = tabla(el).shadowRoot.querySelector("tx-checkbox[aria-label='¿RVC012 genera glosa?']")!;
    const input = box.shadowRoot!.querySelector<HTMLInputElement>("input")!;
    input.checked = true;
    input.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();
    await el.updateComplete;
    await tabla(el).updateComplete;
    expect(updateHallazgo).toHaveBeenCalledWith("RVC012", { genera_glosa: true, devolucion: false, descripcion: "" });
    expect(tabla(el).shadowRoot.querySelector(".estado .ok")!.textContent).toContain("Guardado");
  });

  it("commits a typed description on blur and rolls a failed save back to the typed text", async () => {
    const updateHallazgo = vi.fn().mockRejectedValue(new ApiError(500, "no se pudo guardar"));
    const el = await mountPage(fakeHallazgosApi({ updateHallazgo }));
    await flush();
    await el.updateComplete;
    const area = tabla(el).shadowRoot.querySelector("tx-textarea[aria-label='Descripción de RVC012']")!;
    const inner = area.shadowRoot!.querySelector("textarea")!;
    inner.value = "nota";
    inner.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    area.dispatchEvent(new Event("blur"));
    await flush();
    await el.updateComplete;
    await tabla(el).updateComplete;
    expect(updateHallazgo).toHaveBeenCalledWith("RVC012", { genera_glosa: false, devolucion: false, descripcion: "nota" });
    expect(tabla(el).shadowRoot.querySelector(".estado .fallo")!.textContent).toContain("no se pudo guardar");
    expect(inner.value).toBe("nota");
  });
});
