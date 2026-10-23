import { describe, expect, it } from "vitest";
import { DOT_COLOR, DOT_TONE, KPIS, KPI_TONE, TEXT_FILTERS, estadoClass } from "../../src/utils/rips-constants";
import { RIPS_ICONS } from "../../src/utils/rips-icons";

// Ported from webui-src/src/rips/constants.test.js (same cases and expectations).
describe("rips constants", () => {
  it("estadoClass maps estados to badge colors", () => {
    expect(estadoClass("TERMINADO")).toBe("success");
    expect(estadoClass("terminado")).toBe("success");
    for (const e of ["RECHAZADO", "VENCIDO", "NOTA_AJUSTE", "EXCLUIDA"]) expect(estadoClass(e), e).toBe("error");
    expect(estadoClass("REVISION_MANUAL")).toBe("warning");
    for (const e of ["EN_PROCESO", "SIN INICIAR", "", null, undefined]) expect(estadoClass(e), String(e)).toBe("primary");
  });

  it("DOT_COLOR uses Color names (neutral instead of grey)", () => {
    expect(DOT_COLOR).toEqual({ ok: "success", error: "error", en_curso: "warning", pendiente: "neutral" });
  });

  it("DOT_TONE spells out one literal background class per dot color", () => {
    expect(Object.keys(DOT_TONE).sort()).toEqual(["error", "neutral", "success", "warning"]);
    for (const tone of Object.values(DOT_TONE)) expect(tone).toMatch(/^bg-[a-z0-9-]+/);
  });

  it("KPI colors are Tailwind hues (neutral instead of grey)", () => {
    const colors = Object.fromEntries(KPIS.map(([key, , , , color]) => [key, color]));
    expect(colors).toEqual({
      total: "cyan", no_radicado: "neutral", radicadas: "green", rechazadas: "red", radicacion_parcial: "blue",
      nota_ajuste: "pink", listas_para_radicar: "orange", nota_credito: "purple", en_proceso: "violet",
      sin_autorizacion: "red",
    });
  });

  it("every KPI hue has literal tint, icon, bar and ring classes", () => {
    for (const [key, , , , color] of KPIS) {
      const tone = KPI_TONE[color];
      expect(tone.card, key).toMatch(/^bg-/);
      expect(tone.icon, key).toMatch(/^text-/);
      expect(tone.bar, key).toMatch(/^bg-/);
      expect(tone.ring, key).toMatch(/^text-/);
    }
  });

  it("every KPI and text filter icon key resolves to a literal iconoir class", () => {
    for (const [key, , , icon] of KPIS) expect(RIPS_ICONS[icon] || "", `KPI ${key}: ${icon}`).toMatch(/^i-iconoir-[a-z0-9-]+$/);
    for (const [key, , , icon] of TEXT_FILTERS) expect(RIPS_ICONS[icon] || "", `filter ${key}: ${icon}`).toMatch(/^i-iconoir-[a-z0-9-]+$/);
  });
});
