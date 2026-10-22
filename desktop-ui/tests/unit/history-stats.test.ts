import { expect, test } from "vitest";
import { statChips } from "../../src/utils/history-stats";
import type { StatChip } from "../../src/utils/history-stats";

const full = {
  limite: 100, evaluadas: 188, esperadas: 10, gestionadas: 10, terminadas: 6, con_errores: 1, excluidas: 2, revision_manual: 1,
};
// A run from before the preparation counts existed: none of the new keys.
const legacy = { esperadas: 10, gestionadas: 10, terminadas: 6, con_errores: 1, excluidas: 2, revision_manual: 1 };
const byKey = (chips: StatChip[], key: string) => chips.find((c) => c.key === key);

test("no stats yields no chips", () => {
  expect(statChips(null)).toEqual([]);
  expect(statChips(undefined)).toEqual([]);
  expect(statChips({})).toEqual([]);
  expect(statChips("x")).toEqual([]);
});

test("returns the chips in order with Spanish labels and values", () => {
  const chips = statChips(full);
  expect(chips.map((c) => [c.key, c.label, c.value])).toEqual([
      ["limite", "Límite", 100],
      ["evaluadas", "Evaluadas", 188],
      ["esperadas", "Esperadas", 10],
      ["gestionadas", "Gestionadas", 10],
      ["terminadas", "Terminadas", 6],
      ["con_errores", "Con errores", 1],
      ["excluidas", "Excluidas", 2],
      ["revision_manual", "Revisión manual", 1],
    ]);
});

test("colors follow the outcome", () => {
  const chips = statChips(full);
  expect(byKey(chips, "limite")!.color).toBe("neutral");
  expect(byKey(chips, "evaluadas")!.color).toBe("neutral");
  expect(byKey(chips, "esperadas")!.color).toBe("neutral");
  expect(byKey(chips, "gestionadas")!.color).toBe("primary");
  expect(byKey(chips, "terminadas")!.color).toBe("success");
  expect(byKey(chips, "con_errores")!.color).toBe("error");
  expect(byKey(chips, "excluidas")!.color).toBe("neutral");
  expect(byKey(chips, "revision_manual")!.color).toBe("warning");
});

test("zero errors is neutral so a clean run is not red", () => {
  const chips = statChips({ ...full, con_errores: 0 });
  expect(byKey(chips, "con_errores")!.color).toBe("neutral");
  expect(byKey(chips, "con_errores")!.value).toBe(0);
});

test("excluidas and revision manual only appear above zero", () => {
  const chips = statChips({ ...full, excluidas: 0, revision_manual: 0 });
  expect(chips.map((c) => c.key)).toEqual(["limite", "evaluadas", "esperadas", "gestionadas", "terminadas", "con_errores"]);
  const some = statChips({ ...full, excluidas: 0, revision_manual: 3 });
  expect(some.map((c) => c.key)).toEqual([
    "limite", "evaluadas", "esperadas", "gestionadas", "terminadas", "con_errores", "revision_manual",
  ]);
});

test("limite and evaluadas only appear above zero", () => {
  expect(statChips(legacy).map((c) => c.key)).toEqual([
    "esperadas", "gestionadas", "terminadas", "con_errores", "excluidas", "revision_manual",
  ]);
  // No limit given: evaluated is still shown, the limit is not.
  const noLimit = statChips({ ...full, limite: 0 });
  expect(noLimit.map((c) => c.key).slice(0, 3)).toEqual(["evaluadas", "esperadas", "gestionadas"]);
  const noEvaluated = statChips({ ...full, evaluadas: 0 });
  expect(noEvaluated.map((c) => c.key).slice(0, 2)).toEqual(["limite", "esperadas"]);
});

test("the 188 evaluated / limit 100 / 88 excluded run reads clearly", () => {
  const chips = statChips({
    limite: 100, evaluadas: 188, esperadas: 100, gestionadas: 100, terminadas: 100, con_errores: 0, excluidas: 88, revision_manual: 0,
  });
  expect(chips.map((c) => `${c.label}: ${c.value}`)).toEqual([
    "Límite: 100", "Evaluadas: 188", "Esperadas: 100", "Gestionadas: 100", "Terminadas: 100", "Con errores: 0", "Excluidas: 88",
  ]);
});

test("new hints explain the limit and the evaluated invoices", () => {
  const chips = statChips(full);
  expect(byKey(chips, "limite")!.hint).toBe("Facturas pendientes que se pidió preparar (--limit)");
  expect(byKey(chips, "evaluadas")!.hint).toBe("Facturas pendientes que se revisaron contra las reglas de convenio hasta completar el límite");
  expect(byKey(chips, "excluidas")!.hint).toMatch(/convenio/);
  expect(byKey(chips, "revision_manual")!.hint).toMatch(/convenio/);
});

test("missing or non numeric fields are coerced to 0 and the four core chips stay", () => {
  const chips = statChips({ esperadas: 5 });
  expect(chips.map((c) => [c.key, c.value])).toEqual([
    ["esperadas", 5], ["gestionadas", 0], ["terminadas", 0], ["con_errores", 0],
  ]);
  const junk = statChips({ esperadas: "7", gestionadas: null, terminadas: "abc", con_errores: NaN });
  expect(junk.map((c) => c.value)).toEqual([7, 0, 0, 0]);
  const junkNew = statChips({ esperadas: 1, limite: "abc", evaluadas: null });
  expect(junkNew.map((c) => c.key)).toEqual(["esperadas", "gestionadas", "terminadas", "con_errores"]);
});

test("every chip carries a non empty Spanish hint", () => {
  for (const c of statChips(full)) {
    expect(typeof c.hint).toBe("string");
    expect(c.hint.length > 10, c.key).toBeTruthy();
  }
});

// A run where two invoices stopped and were replaced: 5 asked, 7 entered.
const replaced = {
  limite: 5, evaluadas: 15, esperadas: 5, procesadas: 7, reemplazadas: 2,
  gestionadas: 7, terminadas: 5, con_errores: 0, excluidas: 8, revision_manual: 2,
};

test("procesadas and reemplazadas sit right after esperadas when something was replaced", () => {
  expect(statChips(replaced).map((c) => `${c.label}: ${c.value}`)).toEqual([
    "Límite: 5", "Evaluadas: 15", "Esperadas: 5", "Procesadas: 7", "Reemplazadas: 2",
    "Gestionadas: 7", "Terminadas: 5", "Con errores: 0", "Excluidas: 8", "Revisión manual: 2",
  ]);
});

test("procesadas and reemplazadas are hidden without replacement", () => {
  // New server, no replacement: procesadas equals esperadas, reemplazadas is 0.
  const none = statChips({ ...full, procesadas: 10, reemplazadas: 0 });
  expect(byKey(none, "procesadas")).toBe(undefined);
  expect(byKey(none, "reemplazadas")).toBe(undefined);
  // Old entries have neither key.
  expect(byKey(statChips(legacy), "procesadas")).toBe(undefined);
  expect(byKey(statChips(legacy), "reemplazadas")).toBe(undefined);
  // procesadas below esperadas (a cancelled run) is not a replacement either.
  expect(byKey(statChips({ ...full, procesadas: 4 }), "procesadas")).toBe(undefined);
});

test("reemplazadas shows on its own above zero, procesadas only above esperadas", () => {
  const only = statChips({ ...legacy, reemplazadas: 1 });
  expect(only.map((c) => c.key).slice(0, 3)).toEqual(["esperadas", "reemplazadas", "gestionadas"]);
});

test("replacement chips colors and Spanish hints", () => {
  const chips = statChips(replaced);
  expect(byKey(chips, "procesadas")!.color).toBe("neutral");
  expect(byKey(chips, "reemplazadas")!.color).toBe("info");
  expect(byKey(chips, "procesadas")!.hint).toBe("Facturas que entraron al proceso, incluyendo las tomadas para reemplazar a las que se detuvieron");
  expect(byKey(chips, "reemplazadas")!.hint).toBe("Facturas adicionales que se tomaron porque otras se detuvieron antes de la radicación");
  expect(byKey(chips, "revision_manual")!.hint).toMatch(/Revísalas en el informe de revisión manual del Dashboard/);
});

test("a stats object with only the replacement keys still yields chips", () => {
  expect(statChips({ procesadas: 3 }).length > 0).toBeTruthy();
  expect(statChips({ reemplazadas: 1 }).length > 0).toBeTruthy();
});
