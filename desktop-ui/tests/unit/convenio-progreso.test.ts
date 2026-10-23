import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/convenio-progreso";
import type { TxConvenioProgreso } from "../../src/components/convenio-progreso";
import type { TxProgress } from "../../src/components/tx-progress";
import type { ConvenioInput } from "../../src/utils/convenio-progress";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(convenio: ConvenioInput | null | undefined) {
  const el = document.createElement("tx-convenio-progreso") as TxConvenioProgreso;
  el.convenio = convenio;
  document.body.append(el);
  await el.updateComplete;
  const progress = el.shadowRoot!.querySelector<TxProgress>("tx-progress");
  await progress?.updateComplete;
  return { el, text: el.shadowRoot!.textContent ?? "", bar: progress?.shadowRoot!.querySelector("[role=progressbar]") ?? null };
}

describe("<tx-convenio-progreso>", () => {
  it("renders nothing without convenio info", async () => {
    const { el, bar } = await mount(null);
    expect(bar).toBeNull();
    expect(el.shadowRoot!.querySelector(".strip")).toBeNull();
    expect((await mount({ estado: "desconocido" })).bar).toBeNull();
  });

  it("exposes a determinate progressbar while evaluating", async () => {
    const { el, bar, text } = await mount({ estado: "preparando", evaluadas: 40, total: 160 });
    expect(bar!.getAttribute("aria-label")).toBe("Preparación del convenio");
    expect(bar!.getAttribute("aria-valuenow")).toBe("25");
    expect(bar!.getAttribute("aria-valuetext")).toBe("Evaluadas 40 de 160 facturas");
    expect(text).toContain("Evaluadas 40 de 160 facturas");
    expect(el.shadowRoot!.querySelector(".strip")!.getAttribute("data-estado")).toBe("preparando");
  });

  it("is indeterminate before the walk starts", async () => {
    const { bar, text } = await mount({ estado: "preparando", total: 0 });
    expect(bar!.hasAttribute("aria-valuenow")).toBe(false);
    expect(text).toContain("Leyendo el CSV y la nota técnica");
  });

  it("shows the error message as an alert", async () => {
    const { el } = await mount({ estado: "error", evaluadas: 1, total: 4, mensaje: "boom" });
    expect(el.shadowRoot!.querySelector("[role=alert]")!.textContent).toBe("boom");
    const progress = el.shadowRoot!.querySelector<TxProgress>("tx-progress")!;
    expect(progress.color).toBe("error");
  });

  it("shows the counters as badges when done", async () => {
    const { el, text } = await mount({ estado: "ok", total: 10, preparadas: 8, excluidas: 1, revision_manual: 1 });
    expect(text).toContain("Convenio preparado");
    expect(text).toContain("Preparadas: 8");
    expect(text).toContain("Excluidas: 1");
    expect(text).toContain("Revisión manual: 1");
    expect(el.shadowRoot!.querySelectorAll("tx-badge")).toHaveLength(3);
    expect(el.shadowRoot!.querySelector("[role=alert]")).toBeNull();
  });

  it("updates when the convenio changes", async () => {
    const { el } = await mount({ estado: "preparando", evaluadas: 1, total: 4 });
    el.convenio = null;
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector(".strip")).toBeNull();
  });
});
