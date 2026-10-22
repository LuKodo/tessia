import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/rips/portal-modal";
import type { PortalView } from "../../src/types/api";
import { click, deepText, mountEl, q, qa, settle } from "../support/dashboard-dom";
import { portalView } from "../support/dashboard-fixtures";
import { routedApi } from "../support/dashboard-routed-api";

const holder = vi.hoisted(() => ({ api: {} as Record<string, unknown> }));
vi.mock("../../src/api", () => ({ api: new Proxy({}, { get: (_t, k) => holder.api[k as string] }) }));

type Routes = Parameters<typeof routedApi>[0];
type Modal = HTMLElement & { open: boolean; factura: string; updateComplete: Promise<boolean> };

afterEach(() => {
  document.body.innerHTML = "";
});

async function mountModal(routes: Routes, props: { open?: boolean; factura?: string } = {}) {
  const routed = routedApi(routes);
  holder.api = routed.api as unknown as Record<string, unknown>;
  return { ...routed, ...(await mountWith(props)) };
}

async function mountWith(props: { open?: boolean; factura?: string } = {}) {
  const el = await mountEl<Modal>("portal-modal", { open: true, factura: "FVS1001", ...props });
  const events = { loaded: 0, close: 0 };
  el.addEventListener("loaded", () => events.loaded++);
  el.addEventListener("close", () => events.close++);
  await settle(el);
  return { el, events };
}

const text = (el: Element) => deepText(el);
const button = (el: Element, label: string) => qa<HTMLElement & { disabled: boolean }>(el, "tx-button").find((b) => b.textContent!.trim() === label)!;

const full = portalView({
  estado: "VALIDADO_CON_ADVERTENCIA",
  advertencias: 1,
  errores: 1,
  remision: "535895",
  usuario: "u@x.co",
  consultas: 3,
  consultado_en: "2026-10-05T12:30:00Z",
  origen: "portal",
  documento_soporte: { nombre_archivo: "sop.pdf", estado: "ACTIVE" },
  hallazgos: [
    { codigo: "W1", tipo: "ADVERTENCIA", campo: "campo.a", detalle: "Detalle de la advertencia" },
    { codigo: "E1", tipo: "ERROR", campo: "", detalle: "Detalle del error" },
  ],
});

describe("<portal-modal>", () => {
  it("fetches the portal result when it opens and renders its sections", async () => {
    const { el, events, callsTo } = await mountModal({ "GET /api/facturas/FVS1001/portal": full });
    expect(callsTo("GET", "/api/facturas/FVS1001/portal")).toHaveLength(1);
    const t = text(el);
    expect(t).toContain("Resultado del portal de Mutual Ser");
    expect(t).toContain("FVS1001");
    expect(t).toContain("VALIDADO_CON_ADVERTENCIA");
    expect(t).toContain("Con errores en el portal");
    expect(t).toContain("Datos del portal");
    expect(t).toContain("535895");
    expect(t).toContain("sop.pdf (ACTIVE)");
    expect(t).toContain("3 consultas");
    // errors are listed before warnings
    expect(t.indexOf("Detalle del error")).toBeLessThan(t.indexOf("Detalle de la advertencia"));
    expect(t).toContain("campo.a");
    expect(events.loaded).toBe(1);
  });

  it("does not fetch while closed and fetches once it opens", async () => {
    const { el, callsTo } = await mountModal({ "GET /api/facturas/FVS1001/portal": full }, { open: false });
    expect(callsTo("GET", "/api/facturas/FVS1001/portal")).toHaveLength(0);
    Object.assign(el, { open: true });
    await settle(el);
    expect(callsTo("GET", "/api/facturas/FVS1001/portal")).toHaveLength(1);
  });

  it("a 200 carrying `error` still shows the stored data plus the warning", async () => {
    const withError = portalView({ ...full, error: "El portal no respondió; se muestra la última consulta guardada" });
    const { el, events } = await mountModal({ "GET /api/facturas/FVS1001/portal": withError });
    expect(q(q(el, "portal-contenido")!, ".portal-error")!.getAttribute("description")).toContain("El portal no respondió");
    expect(text(el)).toContain("535895");
    expect(text(el)).not.toContain("No se pudo consultar el portal");
    expect(events.loaded).toBe(1);
  });

  it("an invoice the portal never saw shows its message", async () => {
    const unseen = portalView({ consultado: false, mensaje: "El portal aún no tiene registro de esta factura" });
    const { el } = await mountModal({ "GET /api/facturas/FVS1001/portal": unseen });
    expect(q(q(el, "portal-contenido")!, ".portal-empty")!.getAttribute("description")).toContain("El portal aún no tiene registro");
    expect(text(el)).not.toContain("Datos del portal");
  });

  it("a clean validated result says there are no findings", async () => {
    const { el } = await mountModal({ "GET /api/facturas/FVS1001/portal": portalView({ origen: "base de datos", consultas: 1 }) });
    expect(q(q(el, "portal-contenido")!, ".portal-clean")).not.toBeNull();
    expect(text(el)).toContain("1 consulta ·");
    expect(text(el)).toContain("base de datos");
  });

  it("a failed request shows the error in an alert and no content", async () => {
    const { el, events } = await mountModal({
      "GET /api/facturas/FVS1001/portal": new Response(JSON.stringify({ error: "almacén caído" }), { status: 500 }),
    });
    const alert = q(el, ".request-error")!;
    expect(alert.getAttribute("description")).toBe("No se pudo consultar el portal: almacén caído");
    expect(alert.getAttribute("role")).toBe("alert");
    expect(text(el)).not.toContain("Datos del portal");
    expect(events.loaded).toBe(0);
  });

  it("encodes the invoice number in the path", async () => {
    const { calls } = await mountModal({ "GET /api/facturas/FV%2F1/portal": full }, { factura: "FV/1" });
    expect(calls[0]?.path).toBe("/api/facturas/FV%2F1/portal");
  });

  it("refreshing POSTs to /actualizar, replaces the result and announces it", async () => {
    const refreshed = portalView({ remision: "999000", estado: "VALIDADO" });
    const { el, events, callsTo } = await mountModal({
      "GET /api/facturas/FVS1001/portal": full,
      "POST /api/facturas/FVS1001/portal/actualizar": refreshed,
    });
    click(button(el, "Actualizar desde el portal"));
    await settle(el);
    expect(callsTo("POST", "/api/facturas/FVS1001/portal/actualizar")).toHaveLength(1);
    expect(text(el)).toContain("999000");
    expect(text(el)).not.toContain("535895");
    expect(events.loaded).toBe(2);
  });

  it("a refresh answered 200 with `error` keeps the data and shows the warning", async () => {
    const refreshed = portalView({ ...full, remision: "777", error: "Mutual Ser no contestó" });
    const { el } = await mountModal({
      "GET /api/facturas/FVS1001/portal": full,
      "POST /api/facturas/FVS1001/portal/actualizar": refreshed,
    });
    click(button(el, "Actualizar desde el portal"));
    await settle(el);
    expect(q(q(el, "portal-contenido")!, ".portal-error")!.getAttribute("description")).toContain("Mutual Ser no contestó");
    expect(text(el)).toContain("777");
  });

  it("a failed refresh shows the error", async () => {
    const { el } = await mountModal({
      "GET /api/facturas/FVS1001/portal": full,
      "POST /api/facturas/FVS1001/portal/actualizar": new Response("boom", { status: 502 }),
    });
    click(button(el, "Actualizar desde el portal"));
    await settle(el);
    expect(q(el, ".request-error")!.getAttribute("description")).toBe("No se pudo consultar el portal: boom");
  });

  it("drops a response that was overtaken by a newer request", async () => {
    const pending: Array<(v: PortalView) => void> = [];
    const portal = vi.fn(() => new Promise<PortalView>((resolve) => pending.push(resolve)));
    holder.api = { portal, refreshPortal: vi.fn() };
    const { el, events } = await mountWith();
    Object.assign(el, { factura: "FVS2002" });
    await settle(el);
    expect(portal).toHaveBeenCalledTimes(2);
    pending[1]!(portalView({ factura: "FVS2002", remision: "SECOND" }));
    await settle(el);
    pending[0]!(portalView({ factura: "FVS1001", remision: "FIRST" }));
    await settle(el);
    expect(text(el)).toContain("SECOND");
    expect(text(el)).not.toContain("FIRST");
    expect(events.loaded).toBe(1);
  });

  it("closes from the Cerrar button and on Escape without closing itself", async () => {
    const { el, events } = await mountModal({ "GET /api/facturas/FVS1001/portal": full });
    click(button(el, "Cerrar"));
    expect(events.close).toBe(1);
    q(q(el, "tx-modal")!, "dialog")!.dispatchEvent(new Event("cancel", { cancelable: true }));
    expect(events.close).toBe(2);
    expect(el.open).toBe(true);
  });

  it("is a dialog titled for the portal result and the invoice", async () => {
    const { el } = await mountModal({ "GET /api/facturas/FVS1001/portal": full });
    const modal = q(el, "tx-modal") as HTMLElement & { title: string; description: string };
    expect(modal.title).toBe("Resultado del portal de Mutual Ser");
    expect(modal.description).toBe("FVS1001");
  });

  it("shows a spinner while consulting and keeps the refresh button disabled", async () => {
    const pending: Array<(v: PortalView) => void> = [];
    holder.api = { portal: vi.fn(() => new Promise<PortalView>((resolve) => pending.push(resolve))), refreshPortal: vi.fn() };
    const { el } = await mountWith();
    expect(text(el)).toContain("Consultando el portal de Mutual Ser…");
    expect(q(el, ".spin")).not.toBeNull();
    expect(button(el, "Actualizar desde el portal").disabled).toBe(true);
    pending[0]!(portalView());
    await settle(el);
    expect(text(el)).not.toContain("Consultando el portal");
    expect(button(el, "Actualizar desde el portal").disabled).toBe(false);
  });
});
