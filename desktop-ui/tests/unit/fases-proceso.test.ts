import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/fases-proceso";
import type { TxFasesProceso } from "../../src/components/fases-proceso";
import type { TxProgress } from "../../src/components/tx-progress";
import type { JobPhaseProgress } from "../../src/types/api";

afterEach(() => {
  document.body.innerHTML = "";
});

const phase = (over: Partial<JobPhaseProgress>): JobPhaseProgress => ({
  fase: "derechos",
  etiqueta: "derechos validar",
  en_curso: 0,
  terminadas: 0,
  revision_manual: 0,
  excluidas: 0,
  con_errores: 0,
  ...over,
});

async function mount(fases: JobPhaseProgress[] | undefined, radicadas?: number, pasos?: JobPhaseProgress[]) {
  const el = document.createElement("tx-fases-proceso") as TxFasesProceso;
  el.fases = fases;
  el.radicadas = radicadas;
  el.pasos = pasos;
  document.body.append(el);
  await el.updateComplete;
  const bars = [...el.shadowRoot!.querySelectorAll<TxProgress>("tx-progress")];
  await Promise.all(bars.map((b) => b.updateComplete));
  const cards = [...el.shadowRoot!.querySelectorAll<HTMLElement>("[data-testid=fase-card]")];
  return { el, bars, cards };
}

describe("<tx-fases-proceso>", () => {
  it("renders nothing without phases", async () => {
    for (const fases of [undefined, []]) {
      const { el, cards } = await mount(fases);
      expect(cards).toHaveLength(0);
      expect(el.shadowRoot!.querySelector("tx-card")).toBeNull();
    }
  });

  it("renders one card per phase, in the reported order, with name, counts and percentage", async () => {
    const { cards, bars } = await mount([
      phase({ fase: "derechos", en_curso: 1, terminadas: 3, tiempo_ms: 192_000 }),
      phase({ fase: "thefactory", etiqueta: "thefactory descargar", terminadas: 4 }),
    ]);
    expect(cards.map((c) => c.dataset.fase)).toEqual(["derechos", "thefactory"]);
    const first = cards[0]!.textContent!.replace(/\s+/g, " ");
    expect(first).toContain("Validación de Derechos");
    expect(first).toContain("3/4");
    expect(first).toContain("3 min 12 s");
    expect(first).toContain("1 en curso");
    expect(first).toContain("3 ok");
    expect(bars[0]!.value).toBe(75);
    expect(bars[0]!.color).toBe("primary");
    expect(bars[1]!.value).toBe(100);
    expect(bars[1]!.color).toBe("success");
    const bar = bars[0]!.shadowRoot!.querySelector("[role=progressbar]")!;
    expect(bar.getAttribute("aria-label")).toBe("Validación de Derechos");
    expect(bar.getAttribute("aria-valuetext")).toBe("3 de 4 facturas");
  });

  it("colors each outcome badge", async () => {
    const { cards } = await mount([phase({ en_curso: 2, terminadas: 5, revision_manual: 1, excluidas: 3, con_errores: 4 })]);
    const colors = Object.fromEntries([...cards[0]!.querySelectorAll("tx-badge")].map((b) => [b.textContent!.trim(), b.getAttribute("color")]));
    expect(colors).toEqual({
      "2 en curso": "primary",
      "5 ok": "success",
      "1 revisión": "warning",
      "3 excluidas": "neutral",
      "4 con error": "error",
    });
  });

  it("marks a phase without invoices as queued", async () => {
    const { cards, bars } = await mount([phase({ fase: "fev", etiqueta: "fev validar" })]);
    expect(cards[0]!.dataset.estado).toBe("en-cola");
    expect(cards[0]!.textContent).toContain("en cola");
    expect(cards[0]!.textContent).toContain("0/0");
    expect(bars[0]!.value).toBe(0);
  });

  it("shows the radicación meter as the last card", async () => {
    const { cards } = await mount([phase({ fase: "derechos", terminadas: 2 }), phase({ fase: "radicacion", etiqueta: "radicacion", en_curso: 1, terminadas: 3 })], 3);
    expect(cards.map((c) => c.dataset.fase)).toEqual(["derechos", "radicacion"]);
    const rad = cards[1]!.textContent!.replace(/\s+/g, " ");
    expect(rad).toContain("Radicación");
    expect(rad).toContain("3/4");
    expect(rad).toContain("3 radicadas");
  });

  it("replaces the radicación card with its four step cards, in place", async () => {
    const pasos = [1, 2, 3, 4].map((n) =>
      phase({ fase: `radicacion_${n}`, etiqueta: `Radicación · paso ${n}: X`, en_curso: n === 2 ? 3 : 0, terminadas: n === 1 ? 5 : 0 }),
    );
    const { cards } = await mount([phase({ fase: "derechos", terminadas: 2 }), phase({ fase: "radicacion", etiqueta: "radicacion", en_curso: 8 })], 5, pasos);
    expect(cards.map((c) => c.dataset.fase)).toEqual(["derechos", "radicacion_1", "radicacion_2", "radicacion_3", "radicacion_4"]);
    const first = cards[1]!.textContent!.replace(/\s+/g, " ");
    expect(first).toContain("Radicación · paso 1: X");
    expect(first).toContain("5 confirmadas");
    expect(cards[2]!.textContent).toContain("3 en curso");
  });

  it("adds the radicación card from the confirmed counter alone", async () => {
    const { cards } = await mount([phase({ fase: "derechos", terminadas: 2 })], 2);
    expect(cards.map((c) => c.dataset.fase)).toEqual(["derechos", "radicacion"]);
    expect(cards[1]!.textContent).toContain("2 radicadas");
  });

  it("updates in place when the phases change", async () => {
    const { el } = await mount([phase({ en_curso: 2 })]);
    el.fases = [phase({ terminadas: 2, tiempo_ms: 5000 })];
    await el.updateComplete;
    const card = el.shadowRoot!.querySelector("[data-testid=fase-card]")!;
    expect(card.textContent).toContain("2/2");
    expect(card.textContent).toContain("5 s");
  });
});
