import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/rips-filters";
import { RipsFiltersController } from "../../src/controllers/rips-filters-controller";
import { click, key, mountEl, q, qa, settle } from "../support/dashboard-dom";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mountFilters(alertCounts: Record<string, number> = {}) {
  const filters = new RipsFiltersController();
  const el = await mountEl("rips-filters", { filters, estadoOptions: ["RECHAZADO", "TERMINADO"], alertCounts });
  await settle(el);
  return { el, filters };
}

const field = (el: Element, label: string) => qa(el, "tx-form-field").find((f) => f.getAttribute("label") === label)!;
const control = (el: Element, label: string) => field(el, label).firstElementChild as HTMLElement & { value: string };
const button = (el: Element, label: string) => qa(el, "tx-button").find((b) => b.textContent!.trim() === label)!;
const emit = (target: Element, name: string, detail: unknown) => target.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }));

describe("<rips-filters>", () => {
  it("typing only edits the draft; Filtrar applies it", async () => {
    const { el, filters } = await mountFilters();
    emit(control(el, "Número de factura"), "tx-input", { value: "FVS9" });
    expect(filters.draft.text.factura).toBe("FVS9");
    expect(filters.activeCount).toBe(0);
    click(button(el, "Filtrar"));
    await settle(el);
    expect(filters.activeCount).toBe(1);
    expect(q(el, ".active-count")!.textContent).toContain("1 filtro(s) activo(s) sobre la tabla de seguimiento");
  });

  it("Enter in a text field applies the draft", async () => {
    const { el, filters } = await mountFilters();
    emit(control(el, "SSC"), "tx-input", { value: "5618214" });
    key(control(el, "SSC"), "Enter");
    expect(filters.activeCount).toBe(1);
  });

  it("the date fields feed the draft", async () => {
    const { el, filters } = await mountFilters();
    expect(control(el, "Fecha desde").getAttribute("type")).toBe("date");
    emit(control(el, "Fecha desde"), "tx-input", { value: "2026-09-01" });
    emit(control(el, "Fecha hasta"), "tx-input", { value: "2026-09-30" });
    expect(filters.draft.desde).toBe("2026-09-01");
    expect(filters.draft.hasta).toBe("2026-09-30");
  });

  it("shows the typed draft back in the inputs", async () => {
    const { el, filters } = await mountFilters();
    filters.setText("autorizacion", "A-1");
    await settle(el);
    expect(control(el, "Autorización").value).toBe("A-1");
  });

  it("an alert checkbox applies at once and the counter lists the alert mode", async () => {
    const { el, filters } = await mountFilters({ falta_zip: 3, falta_json: 1 });
    const boxes = qa(el, "tx-checkbox");
    expect(boxes).toHaveLength(10);
    emit(boxes[0]!, "tx-change", { value: undefined, checked: true });
    await settle(el);
    expect(filters.activeCount).toBe(1);
    expect(q(el, ".active-count")!.textContent).not.toContain("alertas: cualquiera de las marcadas");
    emit(qa(el, "tx-checkbox")[1]!, "tx-change", { value: undefined, checked: true });
    await settle(el);
    expect(filters.activeCount).toBe(2);
    expect(q(el, ".active-count")!.textContent).toContain("alertas: cualquiera de las marcadas");
    expect(qa<HTMLInputElement & { checked: boolean }>(el, "tx-checkbox").map((b) => b.checked).slice(0, 3)).toEqual([true, true, false]);
  });

  it("shows how many rows carry each alert, and marks the dangerous ones", async () => {
    const { el } = await mountFilters({ falta_zip: 3, vencida_22_dias: 2 });
    const labels = qa(el, "[data-testid=alert-label]");
    const of = (start: string) => labels.find((l) => l.textContent!.trim().startsWith(start))!;
    expect(q(of("Falta ZIP XML/PDF"), "[data-testid=alert-count]")!.textContent!.trim()).toBe("3");
    expect(q(of("Falta JSON"), "[data-testid=alert-count]")!.textContent!.trim()).toBe("0");
    expect(q(of("Más de 22 días hábiles"), ".alert-text")!.classList.contains("danger")).toBe(true);
    expect(q(of("Falta ZIP XML/PDF"), ".alert-text")!.classList.contains("danger")).toBe(false);
  });

  it("the select check buttons commit the draft; Limpiar resets everything", async () => {
    const { el, filters } = await mountFilters();
    filters.setField("estado", "TERMINADO");
    expect(filters.activeCount).toBe(0);
    const activate = qa(el, "tx-button[title='Activar este filtro']");
    expect(activate).toHaveLength(2);
    click(activate[0]!);
    expect(filters.activeCount).toBe(1);
    click(button(el, "Limpiar"));
    await settle(el);
    expect(filters.activeCount).toBe(0);
    expect(filters.draft.estado).toBe("");
  });

  it("the estado and pipeline selects offer Todos as a sentinel and map it back to an empty draft", async () => {
    const { el, filters } = await mountFilters();
    const [estado, pipeline] = qa<HTMLElement & { items: Array<{ value: string; label: string }>; value: string }>(el, "tx-select");
    expect(estado!.items.map((i) => i.label)).toEqual(["Todos", "RECHAZADO", "TERMINADO"]);
    expect(pipeline!.items[0]!.label).toBe("Todos");
    expect(pipeline!.items.map((i) => i.label)).toContain("Lista para radicar");
    expect(estado!.value).toBe("todos");
    emit(estado!, "tx-change", { value: "TERMINADO" });
    emit(pipeline!, "tx-change", { value: "radicada" });
    expect(filters.draft.estado).toBe("TERMINADO");
    expect(filters.draft.pipeline).toBe("radicada");
    emit(estado!, "tx-change", { value: "todos" });
    expect(filters.draft.estado).toBe("");
  });
});
