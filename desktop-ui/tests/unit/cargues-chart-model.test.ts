import { describe, expect, it } from "vitest";
import type { CarguePoint } from "../../src/types/api";
import {
  CHART_DEFAULT_LIMIT,
  MIN_SEGMENT_W,
  buildBars,
  cargueFilterValue,
  cargueLabel,
  carguePct,
  packStats,
  segmentsOf,
  statsOf,
  tooltipOf,
  truncateLabel,
  visibleCargues,
} from "../../src/utils/cargues-chart-model";

const c = (cargue: string, over: Partial<CarguePoint> = {}): CarguePoint => ({
  cargue, nombre: "", fecha_inicio: "", total: 10, gestionadas: 10, excluidas: 0, revision_manual: 0, radicadas: 5,
  valor_cargado: "1000", valor_radicado: "500", ...over,
});
// The user's example: 1000 invoices, 700 gestionadas, 300 excluidas, 100 radicadas.
const example = c("ex", { total: 1000, gestionadas: 700, excluidas: 300, radicadas: 100 });
const sum = (segs: Array<{ w: number }>) => segs.reduce((n, s) => n + s.w, 0);
const lastNum = (s: string) => Number(s.split(" ").pop());

describe("cargues chart model", () => {
  it("carguePct is radicadas/gestionadas, rounded, and 0 without gestionadas", () => {
    expect(carguePct(c("a", { gestionadas: 8, radicadas: 3 }))).toBe(38);
    expect(carguePct(c("a", { gestionadas: 3, radicadas: 3 }))).toBe(100);
    expect(carguePct(c("a", { gestionadas: 0, radicadas: 0 }))).toBe(0);
    expect(carguePct(example)).toBe(14);
    expect(carguePct(c("x", { total: 5, gestionadas: 0, excluidas: 5, radicadas: 0 }))).toBe(0);
  });

  it("cargueLabel prefers the name, then the id, and names the flat folder", () => {
    expect(cargueLabel(c("c1", { nombre: "Convenio Uno" }))).toBe("Convenio Uno");
    expect(cargueLabel(c("c1"))).toBe("c1");
    expect(cargueLabel(c("."))).toBe("Cargue sin carpeta");
    expect(cargueLabel(c(".", { nombre: "Plano.csv" }))).toBe("Plano.csv");
  });

  it("truncateLabel keeps short text and ellipsizes long text within the limit", () => {
    expect(truncateLabel("corto", 10)).toBe("corto");
    expect(truncateLabel("exactly-10", 10)).toBe("exactly-10");
    expect(truncateLabel("un nombre demasiado largo", 10)).toBe("un nombre…");
    expect(truncateLabel("abc", 1)).toBe("…");
    expect(truncateLabel(undefined, 5)).toBe("");
  });

  it("visibleCargues shows the 12 most recent unless everything is requested", () => {
    const all = Array.from({ length: 15 }, (_, i) => c("c" + i));
    expect(CHART_DEFAULT_LIMIT).toBe(12);
    let v = visibleCargues(all, false);
    expect(v.items).toHaveLength(12);
    expect(v.items[0]!.cargue).toBe("c0"); // the backend already orders most recent first
    expect(v.hidden).toBe(3);
    expect(v.canToggle).toBe(true);
    v = visibleCargues(all, true);
    expect(v.items).toHaveLength(15);
    expect(v.hidden).toBe(0);
    expect(v.canToggle).toBe(true); // still offers "Ver menos"
    expect(visibleCargues(all.slice(0, 12), false).canToggle).toBe(false);
    expect(visibleCargues(undefined, false)).toEqual({ items: [], hidden: 0, canToggle: false });
  });

  it("cargueFilterValue is the id, or the name for the flat folder", () => {
    expect(cargueFilterValue(c("c1", { nombre: "Convenio Uno" }))).toBe("c1");
    expect(cargueFilterValue(c(".", { nombre: "Plano.csv" }))).toBe("Plano.csv");
    expect(cargueFilterValue(c("."))).toBe("");
  });

  it("segmentsOf splits the bar as radicadas | gestionadas (sin radicar) | revisión manual | excluidas", () => {
    const segs = segmentsOf(example, 1000);
    expect(segs.map((x) => x.key)).toEqual(["radicadas", "gestionadas", "excluidas"]);
    expect(segs.map((x) => x.count)).toEqual([100, 600, 300]);
    expect(segs.map((x) => x.w)).toEqual([100, 600, 300]);
    expect(segs.map((x) => x.x)).toEqual([0, 100, 700]);
    expect(segs.every((x) => x.label && x.cls)).toBe(true);
  });

  it("segmentsOf includes revisión manual between gestionadas and excluidas when present", () => {
    const segs = segmentsOf(c("m", { total: 10, gestionadas: 6, radicadas: 2, revision_manual: 3, excluidas: 1 }), 500);
    expect(segs.map((x) => x.key)).toEqual(["radicadas", "gestionadas", "revision_manual", "excluidas"]);
    expect(segs.map((x) => x.count)).toEqual([2, 4, 3, 1]);
    expect(sum(segs)).toBeCloseTo(500, 6);
  });

  it("segmentsOf widths always add up to the track, tiny segments stay visible", () => {
    const segs = segmentsOf(c("t", { total: 100000, gestionadas: 99998, radicadas: 0, excluidas: 2 }), 400);
    expect(segs.map((x) => x.key)).toEqual(["gestionadas", "excluidas"]);
    expect(segs.every((x) => x.w >= MIN_SEGMENT_W)).toBe(true);
    expect(sum(segs)).toBeCloseTo(400, 6);
    expect(segs[1]!.x).toBeCloseTo(segs[0]!.w, 6); // contiguous
    const odd = segmentsOf(c("o", { total: 7, gestionadas: 3, radicadas: 1, excluidas: 2, revision_manual: 2 }), 331);
    expect(sum(odd)).toBeCloseTo(331, 6);
  });

  it("segmentsOf of an all-excluded cargue is one grey segment; of an empty one, none", () => {
    const segs = segmentsOf(c("e", { total: 4, gestionadas: 0, radicadas: 0, excluidas: 4 }), 300);
    expect(segs.map((x) => [x.key, x.w])).toEqual([["excluidas", 300]]);
    expect(segmentsOf(c("z", { total: 0, gestionadas: 0, radicadas: 0 }), 300)).toEqual([]);
  });

  it("segmentsOf never lets radicadas exceed gestionadas (restantes is clamped at 0)", () => {
    const segs = segmentsOf(c("w", { total: 5, gestionadas: 2, radicadas: 3 }), 100);
    expect(segs.every((x) => x.count >= 0 && x.w >= 0)).toBe(true);
  });

  it("statsOf is additive and matches the bar: Radicadas · Gestionadas (sin radicar) · Excluidas · Total", () => {
    // Gestionadas is shown WITHOUT the radicadas (700 - 100 = 600): the numbers add up to Total.
    expect(statsOf(example)).toEqual(["Radicadas 100", "Gestionadas 600", "Excluidas 300", "Total 1000"]);
    const withManual = statsOf(c("m", { total: 10, gestionadas: 5, excluidas: 2, revision_manual: 3, radicadas: 1 }));
    expect(withManual).toEqual(["Radicadas 1", "Gestionadas 4", "Revisión manual 3", "Excluidas 2", "Total 10"]);
    for (const row of [statsOf(example), withManual]) {
      const n = row.map(lastNum);
      expect(n.slice(0, -1).reduce((a, b) => a + b, 0)).toBe(n[n.length - 1]);
    }
  });

  it("statsOf shows the same counts as the segments", () => {
    const row = c("k", { total: 20, gestionadas: 9, radicadas: 4, excluidas: 5, revision_manual: 6 });
    expect(statsOf(row).slice(0, -1).map(lastNum)).toEqual(segmentsOf(row, 400).map((x) => x.count));
  });

  it("packStats joins the stats with · and wraps greedily into lines that fit", () => {
    const stats = statsOf(example);
    expect(packStats(stats, 1000)).toEqual(["Radicadas 100 · Gestionadas 600 · Excluidas 300 · Total 1000"]);
    const narrow = packStats(stats, 200);
    expect(narrow.length).toBeGreaterThan(1);
    expect(narrow.join(" · ")).toBe(stats.join(" · "));
    expect(narrow.every((l) => l.length * 6.6 <= 200 || !l.includes(" · "))).toBe(true);
    expect(packStats([], 200)).toEqual([]);
  });

  it("buildBars lays out one two-line row per cargue with a full-width stacked bar", () => {
    const g = buildBars([example, c("b", { total: 0, gestionadas: 0, radicadas: 0 }), c("c", { total: 5, gestionadas: 5, radicadas: 5 })], 800);
    const [first, second] = g.rows as [(typeof g.rows)[number], (typeof g.rows)[number]];
    expect(g.rows).toHaveLength(3);
    expect(first.y).toBe(g.pad);
    expect(second.y).toBe(first.y + first.h);
    expect(g.H).toBe(g.pad * 2 + g.rows.reduce((n, r) => n + r.h, 0));
    expect(g.barX + g.barW).toBe(g.W - g.pad);
    expect(sum(first.segments)).toBeCloseTo(g.barW, 6);
    expect(second.segments).toEqual([]);
    expect(first.pct).toBe(14);
    expect(first.pctText).toBe("14% radicado");
    expect(first.lines.join(" ")).toBe("Radicadas 100 · Gestionadas 600 · Excluidas 300 · Total 1000");
    expect(first.barY).toBeGreaterThan(first.labelY);
    expect(first.lines.length).toBeGreaterThanOrEqual(1);
    expect(first.h).toBeGreaterThanOrEqual(first.barY - first.y + first.barH);
  });

  it("buildBars gives every row its own scale (composition, not size)", () => {
    const g = buildBars([example, c("small", { total: 10, gestionadas: 7, excluidas: 3, radicadas: 1 })], 700);
    const frac = (r: (typeof g.rows)[number]) => r.segments.map((s) => s.w / g.barW);
    frac(g.rows[0]!).forEach((f, i) => expect(f).toBeCloseTo(frac(g.rows[1]!)[i]!, 9));
  });

  it("buildBars adds rows to wrap the numbers on a narrow chart", () => {
    const wide = buildBars([example], 900).rows[0]!;
    const narrow = buildBars([example], 320).rows[0]!;
    expect(narrow.lines.length).toBeGreaterThan(wide.lines.length);
    expect(narrow.h).toBeGreaterThan(wide.h);
  });

  it("buildBars truncates long labels but keeps the full one for the tooltip", () => {
    const long = "Convenio_muy_largo_".repeat(5);
    const row = buildBars([c("a", { nombre: long })], 400).rows[0]!;
    expect(row.label.length).toBeLessThan(long.length);
    expect(row.label.endsWith("…")).toBe(true);
    expect(row.full).toBe(long);
  });

  it("buildBars of nothing is an empty frame", () => {
    const g = buildBars([], 600);
    expect(g.rows).toEqual([]);
    expect(g.H).toBe(g.pad * 2);
  });

  it("tooltipOf lists every count of the cargue and its values", () => {
    const t = tooltipOf(
      c("c1", { nombre: "Convenio Uno", fecha_inicio: "2026-09-01", total: 12, gestionadas: 4, radicadas: 3, excluidas: 5, revision_manual: 3, valor_cargado: "1234567", valor_radicado: "10" }),
    );
    expect(t).toMatchObject({
      title: "Convenio Uno", cargue: "c1", inicio: "01/09/2026", total: 12, gestionadas: 4, gestionadasSinRadicar: 1,
      excluidas: 5, revisionManual: 3, radicadas: 3, pct: 75,
    });
    expect(t.valorCargado).toMatch(/^\$\s?1\.234\.567$/);
    expect(t.valorRadicado).toMatch(/^\$\s?10$/);
    expect(tooltipOf(c("c1")).inicio).toBe("");
    expect(tooltipOf(c("c2", { gestionadas: 2, radicadas: 3 })).gestionadasSinRadicar).toBe(0);
  });
});
