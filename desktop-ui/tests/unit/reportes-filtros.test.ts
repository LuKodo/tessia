import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/reportes-filtros";
import type { ReportesFiltros } from "../../src/components/reportes-filtros";
import { NO_FILTERS } from "../../src/components/reportes-model";
import { deep, deepAll, toggle, typeInto } from "../support/revision-dom";

async function mountFiltros(props: Partial<ReportesFiltros> = {}) {
  const el = document.createElement("tx-reportes-filtros") as ReportesFiltros;
  Object.assign(el, {
    filters: NO_FILTERS,
    cargueOptions: [
      { value: "A", label: "Convenio A" },
      { value: "B", label: "Convenio B" },
    ],
    csvUrl: "http://api.test/api/reportes/radicadas?formato=csv",
    rangeError: "",
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

const checkboxes = (el: Element) => deepAll(el, "tx-checkbox-group tx-checkbox");
const csv = (el: Element) => deep<HTMLAnchorElement>(el, "[data-testid=reportes-csv]");

describe("<tx-reportes-filtros>", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("emits the typed invoice search", async () => {
    const el = await mountFiltros();
    const seen = events(el, "filter-text");
    typeInto(deep(el, "[data-testid=reportes-search]"), "FVS1");
    expect(seen).toEqual([{ value: "FVS1" }]);
  });

  it("emits the date range", async () => {
    const el = await mountFiltros();
    const desde = events(el, "filter-desde");
    const hasta = events(el, "filter-hasta");
    typeInto(deep(el, "[data-testid=reportes-desde]"), "2026-09-01");
    typeInto(deep(el, "[data-testid=reportes-hasta]"), "2026-09-30");
    expect(desde).toEqual([{ value: "2026-09-01" }]);
    expect(hasta).toEqual([{ value: "2026-09-30" }]);
  });

  it("lists every cargue as a checkbox and reports the checked ones as a new array", async () => {
    const el = await mountFiltros({ filters: { ...NO_FILTERS, cargues: ["A"] } });
    expect(checkboxes(el)).toHaveLength(2);
    const seen = events(el, "filter-cargues");
    toggle(checkboxes(el)[1]!);
    expect(seen).toEqual([{ value: ["A", "B"] }]);
    toggle(checkboxes(el)[0]!);
    expect(seen).toEqual([{ value: ["A", "B"] }, { value: ["B"] }]);
  });

  it("does not leak the inner tx-change events of its controls", async () => {
    const el = await mountFiltros();
    let leaked = 0;
    el.addEventListener("tx-change", () => leaked++);
    toggle(checkboxes(el)[0]!);
    typeInto(deep(el, "[data-testid=reportes-search]"), "x");
    expect(leaked).toBe(0);
  });

  it("shows the current filters in the controls", async () => {
    const el = await mountFiltros({ filters: { cargues: ["B"], desde: "2026-09-01", hasta: "2026-09-30", q: "FVS" } });
    const value = (id: string) => deep<HTMLElement & { value: string }>(el, `[data-testid=${id}]`).value;
    expect(value("reportes-search")).toBe("FVS");
    expect(value("reportes-desde")).toBe("2026-09-01");
    expect(value("reportes-hasta")).toBe("2026-09-30");
    expect(deep<HTMLElement & { value: string[] }>(el, "tx-checkbox-group").value).toEqual(["B"]);
  });

  it("links the CSV download and disables it without a URL", async () => {
    const el = await mountFiltros();
    expect(csv(el).getAttribute("href")).toBe("http://api.test/api/reportes/radicadas?formato=csv");
    expect(csv(el).getAttribute("aria-disabled")).toBe("false");
    expect(csv(el).textContent).toContain("Descargar CSV");

    const off = await mountFiltros({ csvUrl: "" });
    const link = deepAll<HTMLAnchorElement>(off, "[data-testid=reportes-csv]").at(-1)!;
    expect(link.hasAttribute("href")).toBe(false);
    expect(link.getAttribute("aria-disabled")).toBe("true");
  });

  it("shows the range error next to the dates", async () => {
    const ok = await mountFiltros();
    expect(deepAll(ok, "[data-testid=range-error]")).toHaveLength(0);
    const bad = await mountFiltros({ rangeError: "La fecha «desde» no puede ser posterior a «hasta»." });
    expect(deepAll(bad, "[data-testid=range-error]").at(-1)!.textContent).toContain("no puede ser posterior");
  });

  it("offers to clear the filters only when some are active", async () => {
    const idle = await mountFiltros();
    expect(deepAll(idle, "[data-testid=reportes-clear]")).toHaveLength(0);

    const active = await mountFiltros({ filters: { ...NO_FILTERS, q: "FVS" } });
    const seen = events(active, "filter-clear");
    deepAll(active, "[data-testid=reportes-clear]").at(-1)!.click();
    expect(seen).toEqual([{}]);
  });
});
