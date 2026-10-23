import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/reportes-tabla";
import type { ReportesTabla } from "../../src/components/reportes-tabla";
import type { RadicadaRow } from "../../src/types/api";
import { deep, deepAll } from "../support/revision-dom";

const row = (over: Partial<RadicadaRow> = {}): RadicadaRow => ({
  cargue: "A",
  cargue_nombre: "Convenio A",
  factura: "FVS1",
  fecha_radicacion: "2026-09-12T18:30:00Z",
  estado: "TERMINADO",
  fecha_factura: "2026-09-02",
  ssc: "11",
  autorizacion: "AUT1",
  valor: "100",
  observaciones: "RIPS regenerado | radicada",
  ...over,
});

async function mountTabla(rows: RadicadaRow[]) {
  const el = document.createElement("tx-reportes-tabla") as ReportesTabla;
  el.rows = rows;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

describe("<tx-reportes-tabla>", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("renders one row per filed invoice with its cargue, date, estado and notes", async () => {
    const el = await mountTabla([row(), row({ cargue: "B", cargue_nombre: "", factura: "EVS2", observaciones: "" })]);
    const headers = deepAll(el, "th").map((h) => h.textContent!.trim());
    expect(headers).toEqual(["Cargue", "Factura", "Fecha de radicación", "Estado", "Observaciones"]);
    const cells = (r: Element) => [...r.querySelectorAll("td")].map((td) => td.textContent!.trim());
    const rows = deepAll(el, "tr.reportes-row").map(cells);
    expect(rows[0]).toEqual(["Convenio A", "FVS1", expect.stringMatching(/2026|12\/9|9\/12/), "TERMINADO", "RIPS regenerado | radicada"]);
    expect(rows[1]![0]).toBe("B"); // no name: the id
    expect(rows[1]![4]).toBe("—"); // no notes
  });

  it("shows a dash for a filing date the store did not record", async () => {
    const el = await mountTabla([row({ fecha_radicacion: "" })]);
    expect(deepAll(el, "tr.reportes-row td")[2]!.textContent!.trim()).toBe("—");
  });

  it("colors the estado badge", async () => {
    const el = await mountTabla([row()]);
    expect(deep(el, "tx-badge").getAttribute("color")).toBe("success");
  });

  it("says so when there is nothing to show", async () => {
    const el = await mountTabla([]);
    expect(deep(el, "[data-testid=reportes-empty]").textContent).toContain("Sin facturas radicadas");
    expect(deepAll(el, "table")).toHaveLength(0);
  });
});
