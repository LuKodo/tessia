import { describe, expect, it } from "vitest";
import type { JobPhaseProgress } from "../../src/types/api";
import { formatPhaseTime, phaseCards, phaseDoneText, phaseName, phasePct, phaseSettled, phaseTotal } from "../../src/utils/phase-progress";

const phase = (over: Partial<JobPhaseProgress> = {}): JobPhaseProgress => ({
  fase: "derechos",
  etiqueta: "derechos validar",
  en_curso: 0,
  terminadas: 0,
  revision_manual: 0,
  excluidas: 0,
  con_errores: 0,
  ...over,
});

describe("phase arithmetic", () => {
  it("phaseTotal sums every bucket", () => {
    expect(phaseTotal(phase({ en_curso: 2, terminadas: 3, revision_manual: 1, excluidas: 1, con_errores: 1 }))).toBe(8);
  });

  it("phaseSettled excludes in-flight invoices", () => {
    expect(phaseSettled(phase({ en_curso: 5, terminadas: 3, con_errores: 1 }))).toBe(4);
  });

  it("phasePct is 0 when nothing has entered the phase yet", () => {
    expect(phasePct(phase())).toBe(0);
  });

  it("phasePct rounds settled/total to a percentage", () => {
    expect(phasePct(phase({ en_curso: 1, terminadas: 3 }))).toBe(75);
    expect(phasePct(phase({ terminadas: 1, con_errores: 1 }))).toBe(100);
    expect(phasePct(phase({ en_curso: 2, terminadas: 1 }))).toBe(33);
  });

  it("phaseDoneText reads settled/total", () => {
    expect(phaseDoneText(phase({ en_curso: 2, terminadas: 4, excluidas: 1 }))).toBe("5/7");
  });

  it("tolerates missing counters", () => {
    expect(phaseTotal({ fase: "x" } as JobPhaseProgress)).toBe(0);
  });
});

describe("phaseName", () => {
  it("maps every pipeline phase key to its friendly Spanish name", () => {
    const want: Record<string, string> = {
      derechos: "Validación de Derechos",
      thefactory: "Descarga de Factura TF",
      fev: "Validación FEV",
      rips: "Generación RIPS (JSON)",
      cuv: "Generación CUV (Ministerio)",
      soporte: "Generación de Soportes (XML, JSON)",
      conciliar: "Conciliación de Factura",
      radicacion: "Radicación",
    };
    for (const [fase, name] of Object.entries(want)) expect(phaseName(phase({ fase, etiqueta: "cli label" }))).toBe(name);
  });

  it("falls back to the CLI label, then the phase key, for unknown phases", () => {
    expect(phaseName(phase({ fase: "convenio", etiqueta: "convenio preparar" }))).toBe("convenio preparar");
    expect(phaseName({ fase: "otra" } as JobPhaseProgress)).toBe("otra");
  });
});

describe("formatPhaseTime", () => {
  it("renders milliseconds as seconds, minutes or hours", () => {
    expect(formatPhaseTime(42_000)).toBe("42 s");
    expect(formatPhaseTime(192_000)).toBe("3 min 12 s");
    expect(formatPhaseTime(3_780_000)).toBe("1 h 3 min");
  });

  it("is empty when the phase never ran", () => {
    expect(formatPhaseTime(undefined)).toBe("");
    expect(formatPhaseTime(0)).toBe("");
    expect(formatPhaseTime(400)).toBe("");
  });
});

describe("phaseCards", () => {
  it("is empty without phases", () => {
    expect(phaseCards(undefined)).toEqual([]);
    expect(phaseCards([])).toEqual([]);
    expect(phaseCards([], 0)).toEqual([]);
  });

  it("builds one card per phase in the reported order", () => {
    const cards = phaseCards([
      phase({ fase: "derechos", terminadas: 3, en_curso: 1, tiempo_ms: 192_000 }),
      phase({ fase: "fev", etiqueta: "fev validar" }),
    ]);
    expect(cards.map((c) => c.key)).toEqual(["derechos", "fev"]);
    expect(cards[0]!).toMatchObject({ name: "Validación de Derechos", pct: 75, text: "3/4", time: "3 min 12 s", queued: false });
    expect(cards[1]!).toMatchObject({ name: "Validación FEV", pct: 0, text: "0/0", time: "", queued: true });
  });

  it("lists only the non-zero buckets as badges with their colors", () => {
    const [c] = phaseCards([phase({ en_curso: 2, terminadas: 5, revision_manual: 1, excluidas: 3, con_errores: 4 })]);
    expect(c!.badges).toEqual([
      { key: "en_curso", label: "2 en curso", color: "primary" },
      { key: "terminadas", label: "5 ok", color: "success" },
      { key: "revision_manual", label: "1 revisión", color: "warning" },
      { key: "excluidas", label: "3 excluidas", color: "neutral" },
      { key: "con_errores", label: "4 con error", color: "error" },
    ]);
    expect(phaseCards([phase()])[0]!.badges).toEqual([]);
  });

  it("colors the bar by outcome: error on failures, success when complete, primary otherwise", () => {
    expect(phaseCards([phase({ terminadas: 2 })])[0]!.color).toBe("success");
    expect(phaseCards([phase({ terminadas: 2, en_curso: 1 })])[0]!.color).toBe("primary");
    expect(phaseCards([phase({ terminadas: 2, con_errores: 1 })])[0]!.color).toBe("error");
    expect(phaseCards([phase()])[0]!.color).toBe("primary");
  });

  it("shows the radicación meter with the confirmed count as the last card", () => {
    const cards = phaseCards([phase({ fase: "radicacion", etiqueta: "radicacion", en_curso: 2, terminadas: 3 })], 3);
    expect(cards).toHaveLength(1);
    expect(cards[0]!).toMatchObject({ key: "radicacion", name: "Radicación", pct: 60, text: "3/5" });
    expect(cards[0]!.badges.find((b) => b.key === "terminadas")?.label).toBe("3 radicadas");
  });

  it("adds a radicación card when only the confirmed counter is present", () => {
    const cards = phaseCards([phase({ fase: "derechos", terminadas: 1 })], 4);
    expect(cards.map((c) => c.key)).toEqual(["derechos", "radicacion"]);
    expect(cards[1]!).toMatchObject({ pct: 100, text: "4/4", color: "success" });
    expect(cards[1]!.badges).toEqual([{ key: "terminadas", label: "4 radicadas", color: "success" }]);
  });

  it("ignores a zero or missing radicadas counter", () => {
    expect(phaseCards([phase()], 0).map((c) => c.key)).toEqual(["derechos"]);
    expect(phaseCards([phase()], undefined).map((c) => c.key)).toEqual(["derechos"]);
  });
});
