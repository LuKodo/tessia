import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/revision-filtros";
import type { RevisionFiltros } from "../../src/components/revision-filtros";
import { choose, deep, deepAll, toggle, typeInto } from "../support/revision-dom";

async function mountFiltros(props: Partial<RevisionFiltros> = {}) {
  const el = document.createElement("tx-revision-filtros") as RevisionFiltros;
  Object.assign(el, {
    text: "",
    cargue: "",
    fase: "",
    categoria: "",
    includeOthers: false,
    cargueOptions: [{ value: "A", label: "Convenio A" }],
    faseOptions: [{ value: "fev_valor", label: "FEV" }],
    categoriaOptions: [{ value: "revision_manual", label: "Revisión manual" }],
    csvUrl: "",
    csvName: "informe.csv",
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

describe("<tx-revision-filtros>", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("emits the typed search text", async () => {
    const el = await mountFiltros();
    const seen = events(el, "filter-text");
    typeInto(deep(el, "[data-testid=revision-search]"), "EVS1");
    expect(seen).toEqual([{ value: "EVS1" }]);
  });

  it("offers the clear button only with text and clears it", async () => {
    const empty = await mountFiltros();
    expect(deepAll(empty, "[aria-label='Limpiar búsqueda']")).toHaveLength(0);

    const typed = await mountFiltros({ text: "EVS1" });
    const seen = events(typed, "filter-text");
    deep(typed, "[aria-label='Limpiar búsqueda']").click();
    expect(seen).toEqual([{ value: "" }]);
  });

  it("shows the current filters in the controls", async () => {
    const el = await mountFiltros({ cargue: "A", fase: "fev_valor", text: "x" });
    const select = (id: string) => deep<HTMLElement & { value: string }>(el, `[data-testid=${id}]`);
    expect(select("revision-cargue").value).toBe("A");
    expect(select("revision-fase").value).toBe("fev_valor");
    expect(select("revision-categoria").value).toBe("");
    expect(deep<HTMLElement & { value: string }>(el, "[data-testid=revision-search]").value).toBe("x");
  });

  it("lists every option plus a catch-all that maps back to an empty filter", async () => {
    const el = await mountFiltros({ cargue: "A", fase: "fev_valor" });
    const labels = (id: string) => [...deep(el, `[data-testid=${id}]`).shadowRoot!.querySelectorAll("option")].map((o) => o.textContent!.trim());
    expect(labels("revision-cargue")).toEqual(["(todos)", "Convenio A"]);
    expect(labels("revision-fase")).toEqual(["(todas)", "FEV"]);
    expect(labels("revision-categoria")).toEqual(["(todas)", "Revisión manual"]);

    const cargue = events(el, "filter-cargue");
    const fase = events(el, "filter-fase");
    const categoria = events(el, "filter-categoria");
    choose(deep(el, "[data-testid=revision-cargue]"), "A");
    choose(deep(el, "[data-testid=revision-cargue]"), "");
    choose(deep(el, "[data-testid=revision-fase]"), "fev_valor");
    choose(deep(el, "[data-testid=revision-categoria]"), "");
    expect(cargue).toEqual([{ value: "A" }, { value: "" }]);
    expect(fase).toEqual([{ value: "fev_valor" }]);
    expect(categoria).toEqual([{ value: "" }]);
  });

  it("emits the include-others switch", async () => {
    const el = await mountFiltros();
    const seen = events(el, "filter-others");
    toggle(deep(el, "[data-testid=revision-others]"));
    expect(seen).toEqual([{ value: true }]);
  });

  it("links the CSV only when a cargue is selected", async () => {
    const off = await mountFiltros();
    const disabled = deep(off, "[data-testid=informe-csv]");
    expect(disabled.getAttribute("aria-disabled")).toBe("true");
    expect(disabled.hasAttribute("href")).toBe(false);

    const on = await mountFiltros({ csvUrl: "http://api.test/api/cargues/A/informe?formato=csv", csvName: "informe-revision-manual-A.csv" });
    const link = deep(on, "[data-testid=informe-csv]");
    expect(link.getAttribute("href")).toBe("http://api.test/api/cargues/A/informe?formato=csv");
    expect(link.getAttribute("download")).toBe("informe-revision-manual-A.csv");
    expect(link.getAttribute("aria-disabled")).not.toBe("true");
  });
});
