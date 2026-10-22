import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/rips-kpis";
import { RipsFiltersController } from "../../src/controllers/rips-filters-controller";
import type { Kpis } from "../../src/types/api";
import { click, key, mountEl, q, qa, settle, text } from "../support/dashboard-dom";

const counts: Kpis = {
  total: 20,
  no_radicado: 2,
  radicadas: 10,
  rechazadas: 1,
  radicacion_parcial: 3,
  nota_ajuste: 4,
  listas_para_radicar: 5,
  nota_credito: 6,
  en_proceso: 7,
  sin_autorizacion: 8,
};

afterEach(() => {
  document.body.innerHTML = "";
});

async function mountKpis(props: { counts?: Kpis | null; loading?: boolean } = {}) {
  const filters = new RipsFiltersController();
  const el = await mountEl("rips-kpis", { counts, loading: false, ...props, filters });
  await settle(el);
  return { el, filters };
}

const card = (el: Element, label: string) => qa(el, ".kpi").find((c) => c.textContent!.includes(label))!;
const bar = (c: Element) => q(q(c, "tx-progress")!, "[role=progressbar]")!;

describe("<rips-kpis>", () => {
  it("renders the ten cards with the counts of the summary", async () => {
    const { el } = await mountKpis();
    expect(qa(el, ".kpi")).toHaveLength(10);
    expect(card(el, "Total facturas").textContent).toContain("20");
    expect(card(el, "Radicadas").textContent).toContain("10");
    expect(card(el, "Requieren nota ajuste").textContent).toContain("4");
    expect(card(el, "Listas para radicar").textContent).toContain("(4/4 completas)");
    expect(card(el, "Sin autorización").textContent).toContain("(riesgo de glosa)");
  });

  it("shows the share of each card over the total in its progress bar", async () => {
    const { el } = await mountKpis();
    const b = bar(card(el, "Radicadas"));
    expect(b.getAttribute("aria-valuenow")).toBe("50");
    expect(b.getAttribute("aria-label")).toBe("Radicadas");
    expect(b.getAttribute("aria-valuetext")).toBe("50% del total");
  });

  it("shows a spinner instead of numbers while the first summary loads", async () => {
    const { el } = await mountKpis({ counts: null, loading: true });
    const c = card(el, "Radicadas");
    expect(q(c, "[data-testid=kpi-spinner]")).not.toBeNull();
    expect(q(c, "[data-testid=kpi-count]")).toBeNull();
    expect(bar(c).getAttribute("aria-valuenow")).toBeNull();
    expect(bar(c).getAttribute("aria-valuetext")).toBe("Cargando");
  });

  it("shows zeros when there are no counts and nothing is loading", async () => {
    const { el } = await mountKpis({ counts: null, loading: false });
    expect(q(card(el, "Radicadas"), "[data-testid=kpi-count]")!.textContent).toBe("0");
  });

  it("a clickable card toggles its filter and shows the active ring", async () => {
    const { el, filters } = await mountKpis();
    click(card(el, "Rechazadas"));
    await settle(el);
    expect(filters.isKPIActive("rechazadas")).toBe(true);
    expect(filters.draft.pipeline).toBe("rechazada");
    expect(q(card(el, "Rechazadas"), ".kpi__ring")).not.toBeNull();
    click(card(el, "Rechazadas"));
    await settle(el);
    expect(filters.activeCount).toBe(0);
    expect(q(card(el, "Rechazadas"), ".kpi__ring")).toBeNull();
  });

  it("clickable cards are keyboard buttons that expose their pressed state; the total card is not one", async () => {
    const { el, filters } = await mountKpis();
    const rechazadas = card(el, "Rechazadas");
    expect(rechazadas.getAttribute("role")).toBe("button");
    expect(rechazadas.getAttribute("tabindex")).toBe("0");
    expect(rechazadas.getAttribute("aria-pressed")).toBe("false");
    key(rechazadas, "Enter");
    await settle(el);
    expect(filters.isKPIActive("rechazadas")).toBe(true);
    expect(card(el, "Rechazadas").getAttribute("aria-pressed")).toBe("true");
    key(card(el, "Rechazadas"), " ");
    await settle(el);
    expect(filters.isKPIActive("rechazadas")).toBe(false);
    expect(card(el, "Total facturas").getAttribute("role")).toBeNull();
  });

  it("the nota ajuste card filters by alert, and the total card is not clickable", async () => {
    const { el, filters } = await mountKpis();
    click(card(el, "Requieren nota ajuste"));
    expect(filters.draft.alerts.has("nota_ajuste")).toBe(true);
    click(card(el, "Total facturas"));
    expect(filters.activeCount).toBe(1);
    expect(card(el, "Total facturas").classList.contains("clickable")).toBe(false);
  });

  it("follows filters changed from outside (the chart, the filter card)", async () => {
    const { el, filters } = await mountKpis();
    filters.clickKPI("radicadas");
    await settle(el);
    expect(q(card(el, "Radicadas"), ".kpi__ring")).not.toBeNull();
    expect(text(el)).toContain("Radicadas");
  });
});
