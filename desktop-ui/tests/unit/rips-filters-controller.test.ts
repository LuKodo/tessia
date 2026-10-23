import type { ReactiveControllerHost } from "lit";
import { describe, expect, it, vi } from "vitest";
import { RipsFiltersController } from "../../src/controllers/rips-filters-controller";

function host() {
  const h = { addController: vi.fn(), removeController: vi.fn(), requestUpdate: vi.fn(), updateComplete: Promise.resolve(true) };
  return h as typeof h & ReactiveControllerHost;
}

describe("RipsFiltersController", () => {
  it("registers itself on the host and asks it to re-render on every change", () => {
    const h = host();
    const f = new RipsFiltersController(h);
    expect(h.addController).toHaveBeenCalledWith(f);
    f.setText("factura", "x");
    expect(h.requestUpdate).toHaveBeenCalledTimes(1);
  });

  it("typed text only enters the query once committed", () => {
    const f = new RipsFiltersController(host());
    f.setText("factura", "FVS9");
    expect(f.draft.text.factura).toBe("FVS9");
    expect(f.activeCount).toBe(0);
    expect(f.query).toEqual({});
    f.commit();
    expect(f.activeCount).toBe(1);
    expect(f.query).toEqual({ factura: "fvs9" });
  });

  it("keeps the same query object between commits so nothing is refetched for nothing", () => {
    const f = new RipsFiltersController(host());
    f.setText("factura", "A");
    f.commit();
    const first = f.query;
    f.setText("ssc", "typing, not committed");
    expect(f.query).toBe(first);
    f.commit();
    expect(f.query).not.toBe(first);
  });

  it("calls onQueryChange when the applied filters or the sort change, not while typing", () => {
    const changed = vi.fn();
    const f = new RipsFiltersController(host(), changed);
    f.setText("factura", "A");
    f.setField("estado", "TERMINADO");
    expect(changed).not.toHaveBeenCalled();
    f.commit();
    expect(changed).toHaveBeenCalledTimes(1);
    f.clickKPI("radicadas");
    f.sortBy("fecha_factura");
    f.clear();
    expect(changed).toHaveBeenCalledTimes(4);
  });

  it("sortBy cycles the sort like the table header and puts it in the query", () => {
    const f = new RipsFiltersController(host());
    expect(f.sort).toEqual({ key: null, dir: 1 });
    f.sortBy("fecha_factura");
    expect(f.sort).toEqual({ key: "fecha_factura", dir: -1 });
    expect(f.query).toEqual({ sort: "fecha_factura", dir: "desc" });
    f.sortBy("fecha_factura");
    expect(f.query).toEqual({ sort: "fecha_factura", dir: "asc" });
    f.setText("factura", "A");
    f.commit();
    expect(f.query).toEqual({ factura: "a", sort: "fecha_factura", dir: "asc" });
  });

  it("a KPI click commits at once and a second click clears it", () => {
    const f = new RipsFiltersController(host());
    f.clickKPI("radicadas");
    expect(f.isKPIActive("radicadas")).toBe(true);
    expect(f.activeCount).toBe(1);
    f.clickKPI("radicadas");
    expect(f.isKPIActive("radicadas")).toBe(false);
    expect(f.activeCount).toBe(0);
    f.clickKPI("total");
    expect(f.activeCount).toBe(0);
  });

  it("alert KPIs toggle alerts and alertsMulti reports more than one", () => {
    const f = new RipsFiltersController(host());
    f.clickKPI("nota_ajuste");
    expect(f.alertsMulti).toBe(false);
    f.toggleAlert("falta_zip");
    expect(f.alertsMulti).toBe(true);
  });

  it("toggleText drives cargueActive (chart bar click) and clear resets everything", () => {
    const f = new RipsFiltersController(host());
    f.toggleText("cargue", "c1");
    expect(f.cargueActive).toBe(true);
    expect(f.isTextActive("cargue", "c1")).toBe(true);
    f.clear();
    expect(f.cargueActive).toBe(false);
    expect(f.activeCount).toBe(0);
    expect(f.draft.text.cargue).toBe("");
  });

  it("setField edits the draft without applying it", () => {
    const f = new RipsFiltersController(host());
    f.setField("estado", "TERMINADO");
    expect(f.draft.estado).toBe("TERMINADO");
    expect(f.activeCount).toBe(0);
    f.commit();
    expect(f.activeCount).toBe(1);
  });

  it("notifies subscribers (child elements) and stops after unsubscribe", () => {
    const f = new RipsFiltersController(host());
    const seen = vi.fn();
    const off = f.subscribe(seen);
    f.setText("factura", "x");
    f.clickKPI("radicadas");
    expect(seen).toHaveBeenCalledTimes(2);
    off();
    f.clear();
    expect(seen).toHaveBeenCalledTimes(2);
  });
});
