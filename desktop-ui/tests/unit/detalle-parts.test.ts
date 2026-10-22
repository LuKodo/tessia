import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/detalle-card";
import "../../src/components/rips/detalle-convenio";
import "../../src/components/rips/detalle-etapas";
import "../../src/components/rips/detalle-field";
import { click, deepText, mountEl, q, qa, text } from "../support/dashboard-dom";
import { trackingRow } from "../support/dashboard-fixtures";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("<detalle-card>", () => {
  it("shows its optional title above the slot", async () => {
    const el = await mountEl("detalle-card", { title: "NOTAS" });
    el.append("contenido");
    expect(q(el, "[data-testid=card-title]")!.textContent).toBe("NOTAS");
    expect(el.textContent).toBe("contenido");
    expect(q(el, "slot")).not.toBeNull();
  });

  it("omits the title block without a title", async () => {
    const el = await mountEl("detalle-card");
    expect(q(el, "[data-testid=card-title]")).toBeNull();
  });
});

describe("<detalle-field>", () => {
  it("pairs a muted label with its value slot", async () => {
    const el = await mountEl("detalle-field", { label: "SSC" });
    el.append("5618214");
    expect(q(el, ".label")!.textContent).toBe("SSC");
    expect(q(el, ".value slot")).not.toBeNull();
    expect(el.textContent).toBe("5618214");
  });
});

describe("<detalle-convenio>", () => {
  const mountConvenio = (over: Parameters<typeof trackingRow>[0]) => mountEl("detalle-convenio", { row: trackingRow(over) });
  const diffBadge = (el: Element) => qa(el, "tx-badge").find((b) => b.textContent!.trim().startsWith("$"))!;

  it("shows the crossed values and the value difference tinted by whether it matches", async () => {
    const off = await mountConvenio({ ssc: "5618214", subplan: "EVENTO POS", valor_xml: "150", valor_convenio: "100" });
    expect(deepText(off)).toContain("5618214");
    expect(deepText(off)).toContain("EVENTO POS");
    expect(deepText(off)).toContain("Diferencia XML − convenio");
    expect(diffBadge(off).textContent!.trim()).toBe("$ 50");
    expect(diffBadge(off).getAttribute("color")).toBe("error");
    const same = await mountConvenio({ ssc: "5618214", valor_xml: "100", valor_convenio: "100" });
    expect(diffBadge(same).textContent!.trim()).toBe("$ 0");
    expect(diffBadge(same).getAttribute("color")).toBe("success");
  });

  it("flags a filed invoice without authorization with a glosa badge", async () => {
    const el = await mountConvenio({ ssc: "5618214", radicacion: ["ok", "ok", "ok", "ok"] });
    const glosa = qa(el, "tx-badge").find((b) => b.textContent!.includes("Riesgo de GLOSA (radicada sin autorización)"))!;
    expect(glosa.getAttribute("color")).toBe("error");
    expect(glosa.getAttribute("title")).toContain("paso 4 completo");
  });

  it("lists the convenio lines in a plain table and dashes the empty cells", async () => {
    const el = await mountConvenio({
      ssc: "5618214",
      lineas_convenio: [{ ssc: "5618214", cum: "CUM-1", valor_unitario: "10", valor_total: "30", autorizacion: "", id_mipres: "M-1" }],
    });
    const table = q(el, "table.detail-table")!;
    expect(qa(table, "th").map((th) => th.textContent!.trim())).toEqual(["SSC", "CUM", "Vlr. Unitario", "Vlr. Total", "Autorización", "Id Mipres"]);
    const cells = qa(table, "tbody td").map((td) => td.textContent!.trim());
    expect(cells[1]).toBe("CUM-1");
    expect(cells[4]).toBe("-");
    expect(cells[5]).toBe("M-1");
  });

  it("shows skeleton lines instead of the lines table or the empty notice while the detail loads", async () => {
    const el = await mountEl("detalle-convenio", { row: trackingRow({ ssc: "5618214" }), loading: true });
    expect(qa(el, "tx-skeleton").length).toBeGreaterThan(0);
    expect(q(el, "table")).toBeNull();
    expect(q(el, "[aria-busy=true]")).not.toBeNull();
    const none = await mountEl("detalle-convenio", { row: trackingRow(), loading: true });
    expect(deepText(none)).not.toContain("Sin datos de convenio cruzados");
  });

  it("reports a failed detail load with a retry that does not claim there is no convenio data", async () => {
    const el = await mountEl("detalle-convenio", { row: trackingRow(), error: "base caída" });
    const retries: number[] = [];
    el.addEventListener("retry", () => retries.push(1));
    expect(deepText(el)).toContain("No se pudo cargar el detalle de la factura: base caída");
    expect(deepText(el)).not.toContain("Sin datos de convenio cruzados");
    click(qa(el, "tx-button").find((b) => b.textContent!.trim() === "Reintentar") ?? null);
    expect(retries).toHaveLength(1);
  });

  it("explains when there is no convenio data", async () => {
    const el = await mountConvenio({});
    expect(deepText(el)).toContain("Sin datos de convenio cruzados para esta factura todavía.");
    expect(q(el, "table")).toBeNull();
  });
});

describe("<detalle-etapas>", () => {
  const mountEtapas = (over: Parameters<typeof trackingRow>[0]) => mountEl("detalle-etapas", { row: trackingRow(over) });
  const dots = (el: Element) => qa(q(el, "tx-timeline")!, ".dot").map((d) => d.getAttribute("data-color"));

  it("draws one stage card per artifact, edged by the stage outcome", async () => {
    const el = await mountEtapas({ artefactos: ["ok", "error", "ok", "ok"] });
    const cards = qa(el, "[data-testid=stage-card]");
    expect(cards).toHaveLength(4);
    expect(cards[0]!.classList.contains("ok")).toBe(true);
    expect(cards[1]!.classList.contains("error")).toBe(true);
    expect(cards[1]!.textContent!.trim()).toBe("JSON");
    expect(cards[0]!.textContent).not.toContain("Sin observaciones");
  });

  it("renders the filing timeline, one item per step", async () => {
    const el = await mountEtapas({ radicacion: ["ok", "ok", "pendiente", "pendiente"] });
    const timeline = q(el, "tx-timeline")!;
    await (timeline as unknown as { updateComplete: Promise<boolean> }).updateComplete;
    expect(qa(timeline, "li")).toHaveLength(4);
    const t = deepText(timeline);
    expect(t).toContain("Cargue ZIP XML/PDF a Mutualser (completado)");
    expect(t).toContain("(pendiente)");
    expect(t).not.toContain("Sin respuesta");
    expect(dots(el)).toEqual(["success", "success", "neutral", "neutral"]);
  });

  it("a nota de ajuste turns steps 3 and 4 into a warning reprocess", async () => {
    const el = await mountEtapas({ radicacion: ["ok", "ok", "ok", "ok"], requiere_nota_ajuste: true });
    expect(deepText(el).match(/\(pendiente por reproceso - Nota de Ajuste\)/g)).toHaveLength(2);
    expect(dots(el)).toEqual(["success", "success", "warning", "warning"]);
    expect(text(el)).toContain("DETALLE POR ETAPA");
  });
});
