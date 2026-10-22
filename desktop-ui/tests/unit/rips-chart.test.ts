import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/rips-chart";
import { RipsFiltersController } from "../../src/controllers/rips-filters-controller";
import type { CarguePoint, Summary } from "../../src/types/api";
import { click, key, mountEl, q, qa, settle, text } from "../support/dashboard-dom";

const point = (cargue: string, over: Partial<CarguePoint> = {}): CarguePoint => ({
  cargue, nombre: "", fecha_inicio: "2026-09-01", total: 10, gestionadas: 8, excluidas: 2, revision_manual: 0, radicadas: 4,
  valor_cargado: "1000", valor_radicado: "500", ...over,
});
const summary = (cargues: CarguePoint[]): Summary => ({
  desde: "2026-09-01", hasta: "2026-09-30", cargues,
  kpis: { total: 0, no_radicado: 0, radicadas: 0, rechazadas: 0, radicacion_parcial: 0, nota_ajuste: 0, listas_para_radicar: 0, nota_credito: 0, en_proceso: 0, sin_autorizacion: 0 },
});

afterEach(() => {
  document.body.innerHTML = "";
});

async function mountChart(props: { summary?: Summary | null; error?: string; loading?: boolean } = {}) {
  const filters = new RipsFiltersController();
  const el = await mountEl("rips-chart", { summary: null, error: "", loading: false, ...props, filters });
  await settle(el);
  return { el, filters };
}
const bars = (el: Element) => q(el, "rips-chart-bars")!;
const rows = (el: Element) => (q(el, "rips-chart-bars") ? qa(bars(el), "g.row") : []);

describe("<rips-chart>", () => {
  it("explains the loading, empty and error states", async () => {
    const loading = await mountChart({ loading: true });
    expect(text(loading.el)).toContain("Cargando resumen…");
    expect(q(loading.el, "[role=status]")!.textContent).toContain("Cargando resumen…");
    expect(rows(loading.el)).toHaveLength(0);
    const empty = await mountChart({ summary: summary([]), error: "boom" });
    expect(text(empty.el)).toContain("Todavía no hay cargues con facturas.");
    expect(text(empty.el)).toContain("No se pudo cargar el resumen: boom");
  });

  it("draws one row per cargue with its percentage and numbers", async () => {
    const { el } = await mountChart({ summary: summary([point("c1", { nombre: "Convenio Uno" }), point("c2")]) });
    const r = rows(el);
    expect(r).toHaveLength(2);
    expect(r[0]!.textContent).toContain("Convenio Uno");
    expect(r[0]!.textContent).toContain("50% radicado");
    expect(r[0]!.getAttribute("aria-label")).toContain("Radicadas 4");
    expect(qa(r[0]!, "rect.seg").map((s) => s.getAttribute("class"))).toEqual(["seg seg-filed", "seg seg-managed", "seg seg-excluded"]);
  });

  it("a click or Enter/Space on a row toggles the cargue filter and marks it pressed", async () => {
    const { el, filters } = await mountChart({ summary: summary([point("c1")]) });
    click(rows(el)[0]!);
    await settle(el);
    expect(filters.activeCount).toBe(1);
    expect(rows(el)[0]!.getAttribute("aria-pressed")).toBe("true");
    click(rows(el)[0]!);
    await settle(el);
    expect(filters.activeCount).toBe(0);
    key(rows(el)[0]!, "Enter");
    expect(filters.activeCount).toBe(1);
    key(rows(el)[0]!, " ");
    expect(filters.activeCount).toBe(0);
  });

  it("a flat cargue without a name cannot be filtered", async () => {
    const { el, filters } = await mountChart({ summary: summary([point(".")]) });
    click(rows(el)[0]!);
    expect(filters.activeCount).toBe(0);
    expect(rows(el)[0]!.getAttribute("tabindex")).toBe("-1");
  });

  it("shows 12 cargues and toggles Ver todos / Ver menos", async () => {
    const all = Array.from({ length: 15 }, (_, i) => point("c" + i));
    const { el } = await mountChart({ summary: summary(all) });
    expect(rows(el)).toHaveLength(12);
    const toggle = qa(el, "tx-button").find((b) => b.textContent!.trim() === "Ver todos (15)")!;
    click(toggle);
    await settle(el);
    expect(rows(el)).toHaveLength(15);
    expect(text(el)).toContain("Ver menos");
  });

  it("lists the four series in the legend", async () => {
    const { el } = await mountChart({ summary: summary([point("c1")]) });
    expect(qa(el, ".legend span.name").map((s) => s.textContent!.trim())).toEqual(["Radicadas", "Gestionadas (sin radicar)", "Revisión manual", "Excluidas"]);
  });

  it("explains how the cargues are counted in an info note", async () => {
    const { el } = await mountChart({ summary: summary([point("c1")]) });
    const info = q(el, "[role=note]")!;
    expect(info.textContent).toContain("Cada cargue se cuenta completo");
    expect(info.textContent).toContain("los 4 pasos de radicación");
  });

  it("shows the detail tooltip while hovering a row and hides it on leave", async () => {
    const { el } = await mountChart({ summary: summary([point("c1", { nombre: "Convenio Uno" })]) });
    rows(el)[0]!.dispatchEvent(new MouseEvent("mouseenter", { clientX: 10, clientY: 10 }));
    await settle(el);
    expect(q(el, "[data-testid=chart-tip]")!.textContent).toContain("Total de facturas: 10");
    q(el, "[data-testid=chart-box]")!.dispatchEvent(new MouseEvent("mouseleave"));
    await settle(el);
    expect(q(el, "[data-testid=chart-tip]")).toBeNull();
  });

  it("keyboard focus on a row shows its tooltip too", async () => {
    const { el } = await mountChart({ summary: summary([point("c1", { nombre: "Convenio Uno" })]) });
    rows(el)[0]!.dispatchEvent(new FocusEvent("focus"));
    await settle(el);
    expect(q(el, "[data-testid=chart-tip]")!.textContent).toContain("Convenio Uno");
    rows(el)[0]!.dispatchEvent(new FocusEvent("blur"));
    await settle(el);
    expect(q(el, "[data-testid=chart-tip]")).toBeNull();
  });
});
