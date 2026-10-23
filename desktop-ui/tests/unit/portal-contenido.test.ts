import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/portal-contenido";
import { deepText, mountEl, q, qa } from "../support/dashboard-dom";
import { portalView } from "../support/dashboard-fixtures";

afterEach(() => {
  document.body.innerHTML = "";
});

const mountContenido = (over: Parameters<typeof portalView>[0] = {}) => mountEl("portal-contenido", { payload: portalView(over) });

describe("<portal-contenido>", () => {
  it("tints the status icon and the state badge with the tone of the result", async () => {
    const el = await mountContenido({ estado: "VALIDADO_CON_ADVERTENCIA", advertencias: 1 });
    expect(q(el, "[data-testid=portal-status-icon]")!.getAttribute("data-tone")).toBe("warning");
    const badge = qa(el, "tx-badge").find((b) => b.textContent!.trim() === "VALIDADO_CON_ADVERTENCIA")!;
    expect(badge.getAttribute("color")).toBe("warning");
    expect(badge.getAttribute("variant")).toBe("solid");
    expect(deepText(el)).toContain("Con advertencias");
  });

  it("shows the portal error as a warning alert above the stored data", async () => {
    const el = await mountContenido({ error: "El portal no respondió" });
    const alert = q(el, ".portal-error")!;
    expect(alert.getAttribute("description")).toBe("El portal no respondió");
    expect(alert.getAttribute("color")).toBe("warning");
    expect(alert.getAttribute("role")).toBe("alert");
  });

  it("an invoice the portal never saw shows its message as an info note", async () => {
    const el = await mountContenido({ consultado: false, mensaje: "" });
    const note = q(el, ".portal-empty")!;
    expect(note.getAttribute("description")).toBe("El portal aún no tiene registro de esta factura");
    expect(note.getAttribute("role")).toBe("note");
    expect(deepText(el)).not.toContain("Datos del portal");
  });

  it("a clean result says there are no findings, in a success note", async () => {
    const el = await mountContenido();
    const clean = q(el, ".portal-clean")!;
    expect(clean.getAttribute("description")).toContain("Sin hallazgos: el portal validó el JSON sin advertencias");
    expect(clean.getAttribute("role")).toBe("note");
    expect(clean.getAttribute("color")).toBe("success");
  });

  it("groups the findings, errors first, with a count and the code, field and detail of each", async () => {
    const el = await mountContenido({
      hallazgos: [
        { codigo: "W1", tipo: "ADVERTENCIA", campo: "campo.a", detalle: "Detalle W" },
        { codigo: "E1", tipo: "ERROR", campo: "", detalle: "Detalle E" },
        { codigo: "X1", tipo: "INFO", campo: "", detalle: "Detalle X" },
      ],
    });
    const findings = qa(el, ".finding");
    expect(findings.map((f) => q(f, ".finding-code")!.textContent)).toEqual(["E1", "W1", "X1"]);
    expect(q(findings[1]!, ".finding-field")!.textContent).toBe("campo.a");
    expect(q(findings[0]!, ".finding-field")).toBeNull();
    const t = deepText(el);
    expect(t).toContain("Errores");
    expect(t).toContain("Advertencias");
    expect(t).toContain("INFO");
  });

  it("lists the stored portal data and the footer with the origin", async () => {
    const el = await mountContenido({ remision: "535895", consultas: 3, origen: "portal", documento_soporte: { nombre_archivo: "sop.pdf", estado: "ACTIVE" } });
    const t = deepText(el);
    expect(t).toContain("Datos del portal");
    expect(t).toContain("535895");
    expect(t).toContain("sop.pdf (ACTIVE)");
    expect(q(el, ".portal-footer")!.textContent).toMatch(/3 consultas · portal$/);
  });
});
