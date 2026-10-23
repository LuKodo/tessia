import { expect, test } from "vitest";
import { filterItems, categoryLabel, categoryColor, phaseLabel, reportSummaryText } from "../../src/utils/informe-model";

const items = [
  { factura: "EVS1", categoria: "revision_manual" },
  { factura: "EVS2", categoria: "excluida" },
  { factura: "EVS3", categoria: "falla" },
  { factura: "EVS4", categoria: "revision_manual" },
];

test("filterItems shows only manual reviews by default", () => {
  expect(filterItems(items, {}).map((i) => i.factura)).toEqual(["EVS1", "EVS4"]);
  expect(filterItems(items).map((i) => i.factura)).toEqual(["EVS1", "EVS4"]);
  expect(filterItems(items, { includeOthers: false }).map((i) => i.factura)).toEqual(["EVS1", "EVS4"]);
});

test("filterItems with the toggle keeps everything in the original order", () => {
  expect(filterItems(items, { includeOthers: true }).map((i) => i.factura)).toEqual(["EVS1", "EVS2", "EVS3", "EVS4"]);
});

test("filterItems tolerates missing or malformed input and does not mutate", () => {
  expect(filterItems(null, {})).toEqual([]);
  expect(filterItems(undefined, { includeOthers: true })).toEqual([]);
  expect(filterItems("x" as unknown as null, {})).toEqual([]);
  const copy = JSON.parse(JSON.stringify(items));
  filterItems(items, { includeOthers: true });
  expect(items).toEqual(copy);
});

test("categoryLabel is Spanish and falls back to the raw value", () => {
  expect(categoryLabel("revision_manual")).toBe("Revisión manual");
  expect(categoryLabel("excluida")).toBe("Excluida");
  expect(categoryLabel("falla")).toBe("Falla");
  expect(categoryLabel("otra")).toBe("otra");
  expect(categoryLabel(undefined)).toBe("");
});

test("categoryColor maps to the badge colors", () => {
  expect(categoryColor("revision_manual")).toBe("warning");
  expect(categoryColor("excluida")).toBe("neutral");
  expect(categoryColor("falla")).toBe("error");
  expect(categoryColor("otra")).toBe("neutral");
});

test("phaseLabel reuses the PHASES labels and maps the report aliases", () => {
  expect(phaseLabel("convenio")).toBe("Convenio");
  expect(phaseLabel("derechos")).toBe("Derechos");
  expect(phaseLabel("thefactory")).toBe("TheFactory");
  expect(phaseLabel("fev")).toBe("FEV");
  expect(phaseLabel("fev_ajuste")).toBe("FEV");
  expect(phaseLabel("fev_valor")).toBe("FEV");
  expect(phaseLabel("conciliacion")).toBe("Conciliar");
  expect(phaseLabel("conciliar")).toBe("Conciliar");
  expect(phaseLabel("rips")).toBe("RIPS");
  expect(phaseLabel("cuv")).toBe("CUV");
  expect(phaseLabel("soporte")).toBe("Soporte");
  expect(phaseLabel("radicacion")).toBe("Radicación");
});

test("phaseLabel returns unknown phases as they are", () => {
  expect(phaseLabel("algo_raro")).toBe("algo_raro");
  expect(phaseLabel("")).toBe("");
  expect(phaseLabel(undefined)).toBe("");
});

test("reportSummaryText counts manual reviews, adding the others when included", () => {
  const conteos = { revision_manual: 3, excluidas: 88, fallas: 2 };
  expect(reportSummaryText(conteos, false)).toBe("3 en revisión manual");
  expect(reportSummaryText(conteos, true)).toBe("3 en revisión manual · 88 excluidas · 2 fallas");
});

test("reportSummaryText handles singulars, zeros and missing counts", () => {
  expect(reportSummaryText({ revision_manual: 1, excluidas: 1, fallas: 1 }, true)).toBe("1 en revisión manual · 1 excluida · 1 falla");
  expect(reportSummaryText({ revision_manual: 0, excluidas: 0, fallas: 0 }, false)).toBe("0 en revisión manual");
  expect(reportSummaryText({ revision_manual: 2 }, true)).toBe("2 en revisión manual · 0 excluidas · 0 fallas");
  expect(reportSummaryText(null, false)).toBe("0 en revisión manual");
  expect(reportSummaryText(undefined, true)).toBe("0 en revisión manual · 0 excluidas · 0 fallas");
});
