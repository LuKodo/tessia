import { render } from "lit";
import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/rips-dot";
import "../../src/components/rips/rips-steps";
import "../../src/components/rips/rips-estado-badge";
import { dash } from "../../src/components/rips/rips-dash";
import { mountEl, q, qa, text } from "../support/dashboard-dom";
import { trackingRow } from "../support/dashboard-fixtures";

afterEach(() => {
  document.body.innerHTML = "";
});

describe("<rips-dot>", () => {
  it("paints one tone per status and draws a glyph only for ok and error", async () => {
    const seen = [];
    for (const status of ["ok", "error", "en_curso", "pendiente", "unknown"]) {
      const el = await mountEl("rips-dot", { status, label: "ZIP XML/PDF" });
      const dot = q(el, ".dot")!;
      seen.push({ tone: dot.className.match(/tone-(\w+)/)?.[1], glyph: !!dot.querySelector("svg") });
      el.remove();
    }
    expect(seen).toEqual([
      { tone: "success", glyph: true },
      { tone: "error", glyph: true },
      { tone: "warning", glyph: false },
      { tone: "neutral", glyph: false },
      { tone: "neutral", glyph: false },
    ]);
  });

  it("names the artifact and its state for hover and assistive tech", async () => {
    const ok = await mountEl("rips-dot", { status: "ok", label: "JSON RIPS" });
    expect(q(ok, ".dot")!.getAttribute("title")).toBe("JSON RIPS: correcto");
    expect(q(ok, ".dot")!.getAttribute("aria-label")).toBe("JSON RIPS: correcto");
    const odd = await mountEl("rips-dot", { status: "raro", label: "ZIP" });
    expect(q(odd, ".dot")!.getAttribute("title")).toBe("ZIP: raro");
  });
});

describe("<rips-steps>", () => {
  it("numbers the steps and tones them by status, grey ones being neutral", async () => {
    const el = await mountEl("rips-steps", { list: ["ok", "error", "pendiente", "en_curso"] });
    const steps = qa(el, ".step");
    expect(steps.map((s) => s.textContent!.trim())).toEqual(["1", "2", "3", "4"]);
    expect(steps.map((s) => s.className.match(/tone-(\w+)/)?.[1])).toEqual(["success", "error", "neutral", "warning"]);
    expect(steps.map((s) => s.getAttribute("title"))).toEqual([
      "Paso 1: correcto",
      "Paso 2: falta / error",
      "Paso 3: pendiente",
      "Paso 4: en curso",
    ]);
  });

  it("renders nothing for a missing list", async () => {
    const el = await mountEl("rips-steps", { list: null });
    expect(qa(el, ".step")).toHaveLength(0);
  });
});

describe("<rips-estado-badge>", () => {
  const badge = (el: Element) => q(el, "tx-badge")!;

  it("shows the estado in upper case without underscores, tinted with its color", async () => {
    const cases: Array<[string, string]> = [
      ["TERMINADO", "success"],
      ["RECHAZADO", "error"],
      ["REVISION_MANUAL", "warning"],
      ["EN_PROCESO", "primary"],
    ];
    for (const [estado, color] of cases) {
      const el = await mountEl("rips-estado-badge", { row: trackingRow({ estado }) });
      expect(badge(el).textContent!.trim(), estado).toBe(estado.replace(/_/g, " "));
      expect(badge(el).getAttribute("color"), estado).toBe(color);
      el.remove();
    }
  });

  it("falls back to SIN INICIAR for a fresh prepared row", async () => {
    const el = await mountEl("rips-estado-badge", { row: trackingRow({ estado: undefined }) });
    expect(badge(el).textContent!.trim()).toBe("SIN INICIAR");
  });

  it("shows a truncated detail, green when positive and muted otherwise, and hides it on request", async () => {
    const long = "x".repeat(120);
    const el = await mountEl("rips-estado-badge", { row: trackingRow({ estado: "TERMINADO", estado_detalle: long }) });
    const detail = q(el, ".estado-detail")!;
    expect(detail.textContent!.trim()).toBe("x".repeat(90) + "…");
    expect(detail.getAttribute("title")).toBe(long);
    expect(detail.classList.contains("positive")).toBe(false);

    const ok = await mountEl("rips-estado-badge", { row: trackingRow({ estado: "TERMINADO", estado_detalle: "radicado confirmado=true" }) });
    expect(q(ok, ".estado-detail")!.classList.contains("positive")).toBe(true);

    const hidden = await mountEl("rips-estado-badge", { row: trackingRow({ estado: "TERMINADO", estado_detalle: "algo" }), showDetail: false });
    expect(q(hidden, ".estado-detail")).toBeNull();
  });
});

describe("dash()", () => {
  const host = (value: string | number | null | undefined) => {
    const div = document.createElement("div");
    render(dash(value), div);
    return div;
  };

  it("renders the value as is", () => {
    expect(text(host("FVS1"))).toBe("FVS1");
    expect(host("FVS1").querySelector(".dash")).toBeNull();
  });

  it("renders a dimmed dash when the value is empty", () => {
    for (const value of [undefined, null, ""]) expect(host(value).querySelector(".dash")!.textContent).toBe("-");
  });
});
