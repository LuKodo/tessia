import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/revision-tabla";
import type { RevisionTabla } from "../../src/components/revision-tabla";
import { rowKey } from "../../src/utils/revision-batch-model";
import type { RevisionRow } from "../../src/utils/revision-batch-model";
import { deep, deepAll, toggle } from "../support/revision-dom";

const row = (over: Partial<RevisionRow> = {}): RevisionRow => ({
  cargue: "A",
  factura: "EVS1",
  fase: "fev_valor",
  categoria: "revision_manual",
  codigo: "VALOR_DIFERENTE",
  motivo: "VALOR_DIFERENTE XML=1 convenio=2",
  actualizado_en: "",
  ...over,
});

const retryableRow = row();
const otherRow = row({ factura: "EVS2", fase: "derechos", codigo: "", motivo: "sin derechos" });

async function mountTabla(props: Partial<RevisionTabla> = {}) {
  const el = document.createElement("tx-revision-tabla") as RevisionTabla;
  Object.assign(el, {
    rows: [retryableRow, otherRow],
    retryable: new Set([rowKey(retryableRow)]),
    selected: new Set<string>(),
    allState: "none",
    ...props,
  });
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const events = (el: Element, name: string) => {
  const seen: unknown[] = [];
  el.addEventListener(name, (e) => seen.push((e as CustomEvent).detail));
  return seen;
};

describe("<tx-revision-tabla>", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("shows an empty state without rows", async () => {
    const el = await mountTabla({ rows: [] });
    expect(deep(el, "[data-testid=revision-empty]").textContent!.trim()).toBe("Sin facturas por revisar");
    expect(deepAll(el, "tr.revision-row")).toHaveLength(0);
  });

  it("renders a row per item with its cargue, phase and category", async () => {
    const el = await mountTabla();
    const rows = deepAll(el, "tr.revision-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]?.textContent).toContain("A");
    expect(rows[0]?.textContent).toContain("EVS1");
    expect(rows[0]?.textContent).toContain("FEV");
    expect(rows[0]?.textContent).toContain("Revisión manual");
    expect(rows[1]?.textContent).toContain("Derechos");
    expect(rows[1]?.textContent).toContain("sin derechos");
  });

  it("offers a checkbox and Reintentar only on the retryable rows", async () => {
    const el = await mountTabla();
    expect(deepAll(el, "[data-testid=row-select]")).toHaveLength(1);
    expect(deepAll(el, "[data-testid=retry-invoice]")).toHaveLength(1);
    expect(deepAll(deepAll(el, "tr.revision-row")[0]!, "[data-testid=retry-invoice]")).toHaveLength(1);
  });

  it("emits the row key when a checkbox is toggled and the row when Reintentar is clicked", async () => {
    const el = await mountTabla();
    const selects = events(el, "row-select");
    const retries = events(el, "row-retry");
    toggle(deep(el, "[data-testid=row-select]"));
    expect(selects).toEqual([{ key: rowKey(retryableRow), checked: true }]);
    deep(el, "[data-testid=retry-invoice]").click();
    expect(retries).toEqual([{ row: retryableRow }]);
  });

  it("reflects the selection and emits select-all from the header checkbox", async () => {
    const el = await mountTabla({ selected: new Set([rowKey(retryableRow)]), allState: "all" });
    const selectAll = events(el, "row-select-all");
    expect(deep<HTMLElement & { checked: boolean }>(el, "[data-testid=row-select]").checked).toBe(true);
    const header = deep<HTMLElement & { checked: boolean; indeterminate: boolean }>(el, "[data-testid=select-all]");
    expect(header.checked).toBe(true);
    toggle(header);
    expect(selectAll).toEqual([{ checked: false }]);
  });

  it("shows the indeterminate state when some rows are selected", async () => {
    const el = await mountTabla({ allState: "some" });
    const header = deep<HTMLElement & { checked: boolean; indeterminate: boolean }>(el, "[data-testid=select-all]");
    expect(header.indeterminate).toBe(true);
    expect(header.checked).toBe(false);
  });

  it("disables the checkboxes and buttons while a retry runs", async () => {
    const el = await mountTabla({ retrying: true });
    expect(deep(el, "[data-testid=retry-invoice]").hasAttribute("disabled")).toBe(true);
    expect(deep(el, "[data-testid=row-select]").hasAttribute("disabled")).toBe(true);
    expect(deep(el, "[data-testid=select-all]").hasAttribute("disabled")).toBe(true);
  });

  it("disables select-all when no row is retryable", async () => {
    const el = await mountTabla({ retryable: new Set<string>() });
    expect(deep(el, "[data-testid=select-all]").hasAttribute("disabled")).toBe(true);
  });
});
