import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/rips/detalle-modal";
import type { InvoiceTracking, TrackingRow } from "../../src/types/api";
import { click, deepText, mountEl, q, qa, settle } from "../support/dashboard-dom";
import { listRow, trackingRow } from "../support/dashboard-fixtures";
import { routedApi } from "../support/dashboard-routed-api";

const holder = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("../../src/api", () => ({ api: new Proxy({}, { get: (_t, k) => holder.api[k as string] }) }));

afterEach(() => {
  document.body.innerHTML = "";
});

const detailPath = (r: Pick<TrackingRow, "factura" | "cargue">) => `GET /api/facturas/${r.factura}/seguimiento?cargue=${r.cargue}`;

/**
 * Mounts the modal on the slim list row of `row`; the invoice detail route answers `detail`
 * (default: the full row), as GET /api/facturas/{f}/seguimiento does.
 */
async function mountModal(row: InvoiceTracking | null, detail: unknown = row) {
  const routed = routedApi(row ? { [detailPath(row)]: detail } : {});
  holder.api = routed.api as unknown as Record<string, unknown>;
  const el = await mountEl("detalle-modal", { row: row && listRow(row) });
  await settle(el);
  const closes: number[] = [];
  el.addEventListener("close", () => closes.push(1));
  return { el, closes, ...routed };
}
const modal = (el: Element) => q(el, "tx-modal") as (HTMLElement & { open: boolean }) | null;
const text = (el: Element) => deepText(el);

describe("<detalle-modal>", () => {
  it("renders nothing and stays closed while no row is selected", async () => {
    const { el } = await mountModal(null);
    expect(modal(el)!.open).toBe(false);
    expect(text(el)).not.toContain("Factura");
  });

  it("opens as a dialog titled with the invoice number", async () => {
    const { el } = await mountModal(trackingRow({ factura: "FVS8" }));
    expect(modal(el)!.open).toBe(true);
    expect(text(el)).toContain("Factura");
    expect(text(el)).toContain("FVS8");
    expect(q(modal(el)!, "dialog")!.getAttribute("aria-labelledby")).toBeTruthy();
  });

  it("asks to close on Escape and from the header close button, without closing itself", async () => {
    const { el, closes } = await mountModal(trackingRow());
    q(modal(el)!, "dialog")!.dispatchEvent(new Event("cancel", { cancelable: true }));
    expect(closes).toHaveLength(1);
    click(q(modal(el)!, "button[aria-label=Cerrar]"));
    expect(closes).toHaveLength(2);
    expect(modal(el)!.open).toBe(true); // the owner closes it by clearing `row`
  });

  it("emits close from the Cerrar footer button", async () => {
    const { el, closes } = await mountModal(trackingRow());
    click(qa(el, "tx-button").find((b) => b.textContent!.trim() === "Cerrar") ?? null);
    expect(closes).toHaveLength(1);
  });

  it("closes when the row is cleared but keeps the last content during the close", async () => {
    const { el } = await mountModal(trackingRow({ factura: "FVS3" }));
    Object.assign(el, { row: null });
    await settle(el);
    expect(modal(el)!.open).toBe(false);
    expect(text(el)).toContain("FVS3");
  });

  it("tints the plazo badge: solid error when expired, solid success when filed, subtle neutral when pending", async () => {
    const badgeOf = async (row: TrackingRow, label: string) => {
      const { el } = await mountModal(row);
      const badge = qa(el, "tx-badge").find((b) => b.textContent!.trim() === label)!;
      const out = `${badge.getAttribute("variant")}/${badge.getAttribute("color")}`;
      el.remove();
      return out;
    };
    expect(await badgeOf(trackingRow({ vencida_rffe022: true }), "NOTA CRÉDITO · vencido (RFFE022)")).toBe("solid/error");
    expect(await badgeOf(trackingRow({ radicacion: ["ok", "ok", "ok", "ok"] }), "Radicación completa")).toBe("solid/success");
    expect(await badgeOf(trackingRow(), "Falta Paso 1")).toBe("subtle/neutral");
  });

  it("marks each pipeline step done or missing", async () => {
    const { el } = await mountModal(trackingRow({ artefactos: ["ok", "error", "ok", "ok"], radicacion: ["ok", "ok", "pendiente", "pendiente"] }));
    const dots = qa(el, "[data-testid=stepper-dot]").map((d) => d.classList.contains("ok"));
    expect(dots).toEqual([true, false, true, true, true, true, false, false]);
  });

  it("shows the invoice, cargue and estado of the selected row", async () => {
    const { el } = await mountModal(
      trackingRow({ factura: "FVS5", cargue_nombre: "Convenio Cinco", fecha_inicio_cargue: "2026-09-01", fecha_fin_cargue: "2026-09-30", estado: "TERMINADO" }),
    );
    expect(text(el)).toContain("FVS5");
    expect(text(el)).toContain("Convenio Cinco");
    expect(text(el)).toContain("2026-09-01 → 2026-09-30");
    expect(text(el)).toContain("TERMINADO");
  });

  it("an RFFE022 invoice is expired and needs a nota crédito, never computed from dates", async () => {
    const { el } = await mountModal(trackingRow({ vencida_rffe022: true, fecha_factura: "2020-01-01" }));
    expect(text(el)).toContain("NOTA CRÉDITO · vencido (RFFE022)");
    expect(text(el)).toContain("Nota Crédito");
    expect(text(el)).not.toContain("Fecha de factura:");
  });

  it("names the first missing filing step and the rejection code of the estado detail", async () => {
    const { el } = await mountModal(trackingRow({ radicacion: ["ok", "ok", "error", "pendiente"], estado_detalle: "rechazo RF-FE-99" }));
    expect(text(el)).toContain("Paso 3 rechazado (RF-FE-99)");
    expect(text(el)).toContain("Fecha de factura: 2026-10-05");
  });

  it("a complete filing says so", async () => {
    const { el } = await mountModal(trackingRow({ radicacion: ["ok", "ok", "ok", "ok"], autorizacion: "A-1" }));
    expect(text(el)).toContain("Radicación completa");
    expect(text(el)).not.toContain("Riesgo de GLOSA");
  });

  it("flags a filed invoice without authorization as a glosa risk and shows the value difference", async () => {
    const { el } = await mountModal(trackingRow({ radicacion: ["ok", "ok", "ok", "ok"], ssc: "5618214", valor_xml: "150", valor_convenio: "100" }));
    expect(text(el)).toContain("Riesgo de GLOSA (radicada sin autorización)");
    expect(text(el)).toContain("Diferencia XML − convenio");
    expect(text(el)).toContain("$ 50");
  });

  it("a nota de ajuste turns steps 3 and 4 into a reprocess in the timeline", async () => {
    const { el } = await mountModal(trackingRow({ radicacion: ["ok", "ok", "ok", "ok"], requiere_nota_ajuste: true }));
    expect(text(el)).toContain("Nota Ajuste");
    expect(text(el).match(/\(pendiente por reproceso - Nota de Ajuste\)/g)).toHaveLength(2);
  });

  it("shows one card per artifact stage", async () => {
    const { el } = await mountModal(trackingRow({ artefactos: ["ok", "error", "ok", "ok"] }));
    const etapas = q(q(el, "detalle-etapas")!, "[data-testid=stage-card]");
    expect(etapas).not.toBeNull();
    expect(qa(q(el, "detalle-etapas")!, "[data-testid=stage-card]")).toHaveLength(4);
    expect(text(el)).not.toContain("Sin observaciones registradas.");
  });

  it("lists the convenio lines when the invoice has them, or explains there is no convenio data", async () => {
    const withLines = await mountModal(
      trackingRow({
        ssc: "5618214",
        lineas_convenio: [{ ssc: "5618214", cum: "CUM-1", valor_unitario: "10", valor_total: "30", autorizacion: "A-1", id_mipres: "M-1" }],
      }),
    );
    expect(text(withLines.el)).toContain("CUM-1");
    const without = await mountModal(trackingRow());
    expect(text(without.el)).toContain("Sin datos de convenio cruzados para esta factura todavía.");
  });

  it("lists the alerts and no longer shows the legacy upload attempts or delivery support", async () => {
    const { el } = await mountModal(trackingRow({ alertas: ["falta_zip", "vencida_22_dias"], ssc: "5618214" }));
    expect(text(el)).toContain("falta zip");
    expect(text(el)).not.toContain("INTENTOS TÉCNICOS DE CARGUE");
    expect(text(el)).not.toContain("Soporte de entrega");
  });

  describe("on-demand detail", () => {
    const withDetail = trackingRow({
      factura: "FVS7",
      radicacion: ["ok", "ok", "error", "pendiente"],
      estado_detalle: "rechazo RF-FE-99",
      lineas_convenio: [{ ssc: "5618214", cum: "CUM-77", valor_unitario: "10", valor_total: "30", autorizacion: "", id_mipres: "" }],
    });

    it("fetches the invoice's full row once when it opens, with its cargue", async () => {
      const { el, callsTo } = await mountModal(withDetail);
      expect(callsTo("GET", "/api/facturas/FVS7/seguimiento?cargue=c1")).toHaveLength(1);
      expect(text(el)).toContain("CUM-77");
      expect(text(el)).toContain("Paso 3 rechazado (RF-FE-99)");
    });

    it("does not fetch while no row is selected", async () => {
      const { calls } = await mountModal(null);
      expect(calls).toHaveLength(0);
    });

    it("shows a skeleton while the detail loads and the table fields meanwhile", async () => {
      let release!: (v: unknown) => void;
      const pending = new Promise((r) => (release = r));
      const routed = routedApi({ [detailPath(withDetail)]: () => pending as never });
      holder.api = routed.api as unknown as Record<string, unknown>;
      const el = await mountEl("detalle-modal", { row: listRow(withDetail) });
      await settle(el);
      expect(text(el)).toContain("FVS7");
      expect(qa(q(el, "detalle-convenio")!, "tx-skeleton").length).toBeGreaterThan(0);
      expect((q(el, "detalle-convenio") as unknown as { loading: boolean }).loading).toBe(true);
      expect(text(el)).not.toContain("CUM-77");
      expect(text(el)).not.toContain("Sin datos de convenio cruzados");
      release(new Response(JSON.stringify(withDetail), { status: 200 }));
      await settle(el);
      expect(text(el)).toContain("CUM-77");
      expect(qa(q(el, "detalle-convenio")!, "tx-skeleton")).toHaveLength(0);
    });

    it("uses the whole estado detail of the invoice detail over the short one of the list", async () => {
      const whole = trackingRow({ ...withDetail, estado_detalle: "x".repeat(90) + "… rechazo RF-FE-42" });
      const shortRow = { ...listRow(whole), estado_detalle: "x".repeat(90) + "…" };
      const routed = routedApi({ [detailPath(whole)]: whole });
      holder.api = routed.api as unknown as Record<string, unknown>;
      const el = await mountEl("detalle-modal", { row: shortRow });
      await settle(el);
      expect(text(el)).toContain("Paso 3 rechazado (RF-FE-42)");
    });

    it("shows the failure and retries on request", async () => {
      let fail = true;
      const routed = routedApi({
        [detailPath(withDetail)]: () => (fail ? new Response(JSON.stringify({ error: "base caída" }), { status: 500 }) : withDetail),
      });
      holder.api = routed.api as unknown as Record<string, unknown>;
      const el = await mountEl("detalle-modal", { row: listRow(withDetail) });
      await settle(el);
      expect(text(el)).toContain("No se pudo cargar el detalle de la factura");
      expect(text(el)).toContain("base caída");
      fail = false;
      click(qa(q(el, "detalle-convenio")!, "tx-button").find((b) => b.textContent!.trim() === "Reintentar") ?? null);
      await settle(el);
      expect(text(el)).toContain("CUM-77");
      expect(text(el)).not.toContain("No se pudo cargar el detalle");
      expect(routed.callsTo("GET", "/api/facturas/FVS7/seguimiento?cargue=c1")).toHaveLength(2);
    });

    it("loads again for another invoice and ignores a late answer for the previous one", async () => {
      let releaseFirst!: (v: unknown) => void;
      const first = new Promise((r) => (releaseFirst = r));
      const other = trackingRow({ factura: "FVS8", lineas_convenio: [{ ssc: "1", cum: "CUM-88", valor_unitario: "", valor_total: "", autorizacion: "", id_mipres: "" }] });
      const routed = routedApi({ [detailPath(withDetail)]: () => first as never, [detailPath(other)]: other });
      holder.api = routed.api as unknown as Record<string, unknown>;
      const el = await mountEl("detalle-modal", { row: listRow(withDetail) });
      await settle(el);
      Object.assign(el, { row: listRow(other) });
      await settle(el);
      expect(text(el)).toContain("CUM-88");
      releaseFirst(new Response(JSON.stringify(withDetail), { status: 200 }));
      await settle(el);
      expect(text(el)).toContain("CUM-88");
      expect(text(el)).not.toContain("CUM-77");
    });

    it("refreshes the detail when the live row's estado changes, without blanking it", async () => {
      let answer = trackingRow({ ...withDetail, estado: "EN_PROCESO", estado_detalle: "paso 2" });
      const routed = routedApi({ [detailPath(withDetail)]: () => answer });
      holder.api = routed.api as unknown as Record<string, unknown>;
      const el = await mountEl("detalle-modal", { row: listRow(answer) });
      await settle(el);
      expect(text(el)).toContain("CUM-77");
      answer = trackingRow({ ...withDetail, estado: "RECHAZADO", estado_detalle: "rechazo RF-FE-12" });
      Object.assign(el, { row: listRow(answer) });
      await settle(el);
      expect(routed.callsTo("GET", "/api/facturas/FVS7/seguimiento?cargue=c1")).toHaveLength(2);
      expect(text(el)).toContain("CUM-77");
      expect(text(el)).toContain("Paso 3 rechazado (RF-FE-12)");
    });
  });
});
