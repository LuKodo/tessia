import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/hallazgos-tabla";
import type { HallazgosTabla } from "../../src/components/hallazgos-tabla";
import type { HallazgoRow } from "../../src/controllers/hallazgos-controller";
import { row } from "../support/hallazgos-fixtures";

// Ported from ui/tests/unit/hallazgos-tabla.test.ts (same scenarios) against <tx-hallazgos-tabla>.
async function mountTabla(rows: HallazgoRow[], total = rows.length) {
  const el = document.createElement("tx-hallazgos-tabla") as HallazgosTabla;
  el.rows = rows;
  el.total = total;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const $ = <T extends Element>(el: HallazgosTabla, selector: string) => el.shadowRoot!.querySelector<T>(selector)!;
const $$ = (el: HallazgosTabla, selector: string) => [...el.shadowRoot!.querySelectorAll(selector)];
const text = (el: HallazgosTabla) => el.shadowRoot!.textContent ?? "";

function toggle(box: Element, checked: boolean) {
  const input = box.shadowRoot!.querySelector<HTMLInputElement>("input[type=checkbox]")!;
  input.checked = checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("<tx-hallazgos-tabla>", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("tells an empty catalog from a search without matches", async () => {
    expect(text(await mountTabla([], 0))).toContain("Aún no hay hallazgos");
    expect(text(await mountTabla([], 5))).toContain("Ningún hallazgo coincide con la búsqueda.");
  });

  it("renders a row per hallazgo and marks glosa and devolucion rows", async () => {
    const el = await mountTabla([row(), row({ codigo: "H02", genera_glosa: true, devolucion: true })]);
    const rows = $$(el, "tr.hallazgo-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]!.classList.contains("hallazgo-row--glosa")).toBe(false);
    expect(rows[1]!.classList.contains("hallazgo-row--glosa")).toBe(true);
    expect(rows[1]!.classList.contains("hallazgo-row--devolucion")).toBe(true);
    expect(rows[1]!.querySelector(".devolucion-icon")).not.toBeNull();
    expect(rows[0]!.querySelector(".devolucion-icon")).toBeNull();
    expect(rows[0]!.querySelector(".detalle")!.textContent).toContain("Detalle uno");
  });

  it("shows the headers and reflects the checkbox state of each row", async () => {
    const el = await mountTabla([row({ genera_glosa: true })]);
    expect($$(el, "th").map((th) => th.textContent!.trim())).toEqual([
      "Código",
      "Detalle",
      "¿Genera glosa?",
      "¿Devuelve la factura?",
      "Descripción",
    ]);
    expect($<HTMLElement & { checked: boolean }>(el, "tx-checkbox[aria-label='¿H01 genera glosa?']").checked).toBe(true);
    expect($<HTMLElement & { checked: boolean }>(el, "tx-checkbox[aria-label='¿H01 devuelve la factura?']").checked).toBe(false);
  });

  it("reports the checkbox toggles with the new value", async () => {
    const el = await mountTabla([row()]);
    const onToggle = vi.fn();
    const onDevolucion = vi.fn();
    el.addEventListener("toggle-glosa", (e) => onToggle((e as CustomEvent).detail));
    el.addEventListener("toggle-devolucion", (e) => onDevolucion((e as CustomEvent).detail));
    toggle($(el, "tx-checkbox[aria-label='¿H01 genera glosa?']"), true);
    toggle($(el, "tx-checkbox[aria-label='¿H01 devuelve la factura?']"), true);
    expect(onToggle.mock.calls).toEqual([[{ codigo: "H01", value: true }]]);
    expect(onDevolucion.mock.calls).toEqual([[{ codigo: "H01", value: true }]]);
  });

  it("reports edits and commits the description on blur and on Ctrl/Cmd+Enter", async () => {
    const el = await mountTabla([row()]);
    const edit = vi.fn();
    const commit = vi.fn();
    el.addEventListener("edit", (e) => edit((e as CustomEvent).detail));
    el.addEventListener("commit", (e) => commit((e as CustomEvent).detail));
    const area = $(el, "tx-textarea[aria-label='Descripción de H01']");
    const inner = area.shadowRoot!.querySelector("textarea")!;
    inner.value = "nota";
    inner.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
    // Real browsers retarget the non-bubbling composed blur to the host; happy-dom does not.
    area.dispatchEvent(new Event("blur"));
    const key = (init: KeyboardEventInit) =>
      inner.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, composed: true, cancelable: true, ...init }));
    key({ ctrlKey: true });
    key({ metaKey: true });
    key({});
    expect(edit.mock.calls).toEqual([[{ codigo: "H01", text: "nota" }]]);
    expect(commit.mock.calls).toEqual([[{ codigo: "H01" }], [{ codigo: "H01" }], [{ codigo: "H01" }]]);
  });

  it("shows the save state of a row and the length error of its description", async () => {
    const tooLong = "x".repeat(2001);
    const el = await mountTabla([
      row({ status: "saved" }),
      row({ codigo: "H02", status: "error", message: "falló el guardado", descripcion: tooLong }),
      row({ codigo: "H03", status: "saving" }),
    ]);
    const rows = $$(el, "tr.hallazgo-row");
    expect(rows[0]!.querySelector(".estado")!.textContent).toContain("Guardado");
    expect(rows[1]!.querySelector(".estado")!.textContent).toContain("falló el guardado");
    expect(rows[1]!.querySelector(".descripcion-error")!.textContent).toContain("2000");
    expect(rows[0]!.querySelector(".descripcion-error")!.textContent!.trim()).toBe("");
    expect(rows[2]!.querySelector(".estado tx-icon")).not.toBeNull();
    expect(rows[1]!.querySelector(".contador")!.textContent).toContain("2001 / 2000");
    expect(rows[1]!.querySelector("tx-textarea")!.getAttribute("color")).toBe("error");
    expect(rows[0]!.querySelector("tx-textarea")!.getAttribute("color")).toBe("primary");
  });

  it("puts a rolled-back checkbox back in sync with the row", async () => {
    const data = row({ genera_glosa: true });
    const el = await mountTabla([data]);
    const box = $<HTMLElement & { checked: boolean }>(el, "tx-checkbox[aria-label='¿H01 genera glosa?']");
    toggle(box, false); // the user unchecks it; the controller rolls the row back to true
    expect(box.checked).toBe(false);
    el.rows = [{ ...data }];
    await el.updateComplete;
    expect(box.checked).toBe(true);
  });
});
