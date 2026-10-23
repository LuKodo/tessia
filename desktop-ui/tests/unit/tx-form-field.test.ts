import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-form-field";
import "../../src/components/tx-input";
import type { TxFormField } from "../../src/components/tx-form-field";
import type { TxInput } from "../../src/components/tx-input";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(attrs: Record<string, string> = {}, child = "<tx-input></tx-input>") {
  const el = document.createElement("tx-form-field") as TxFormField;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.innerHTML = child;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const q = (el: Element, sel: string) => el.shadowRoot!.querySelector(sel);
const text = (el: Element, sel: string) => q(el, sel)?.textContent?.trim();

describe("tx-form-field", () => {
  it("renders the label, description and hint", async () => {
    const el = await mount({ label: "Desde", description: "Fecha inicial", hint: "opcional" });
    expect(text(el, "label")).toBe("Desde");
    expect(text(el, ".description")).toBe("Fecha inicial");
    expect(text(el, ".hint")).toBe("opcional");
  });

  it("marks required fields with an asterisk hidden from assistive tech", async () => {
    const el = await mount({ label: "Monto", required: "" });
    const star = q(el, ".required")!;
    expect(star.textContent).toBe("*");
    expect(star.getAttribute("aria-hidden")).toBe("true");
  });

  it("shows the help text, and the error replaces it as an alert", async () => {
    const el = await mount({ label: "Monto", help: "Máximo 100" });
    expect(text(el, ".help")).toBe("Máximo 100");
    expect(q(el, ".error")).toBeNull();
    el.error = "Valor inválido";
    await el.updateComplete;
    expect(q(el, ".help")).toBeNull();
    expect(text(el, ".error")).toBe("Valor inválido");
    expect(q(el, ".error")!.getAttribute("role")).toBe("alert");
  });

  it("gives a slotted tx control its label, description, invalid and required state", async () => {
    const el = await mount({ label: "Monto", help: "Máximo 100", required: "" });
    const input = el.querySelector("tx-input") as TxInput;
    await input.updateComplete;
    const inner = input.shadowRoot!.querySelector("input")!;
    expect(inner.getAttribute("aria-label")).toBe("Monto");
    const describedBy = inner.getAttribute("aria-describedby")!;
    expect(input.shadowRoot!.getElementById(describedBy)!.textContent).toContain("Máximo 100");
    expect(inner.required).toBe(true);
    expect(inner.hasAttribute("aria-invalid")).toBe(false);

    el.error = "Obligatorio";
    await el.updateComplete;
    await input.updateComplete;
    expect(inner.getAttribute("aria-invalid")).toBe("true");
    expect(input.shadowRoot!.getElementById(inner.getAttribute("aria-describedby")!)!.textContent).toContain("Obligatorio");

    el.error = undefined;
    await el.updateComplete;
    await input.updateComplete;
    expect(inner.hasAttribute("aria-invalid")).toBe(false);
  });

  it("labels a native control in the light DOM", async () => {
    const el = await mount({ label: "Cargue", error: "x" }, "<input />");
    const input = el.querySelector("input")!;
    expect(input.getAttribute("aria-label")).toBe("Cargue");
    expect(input.getAttribute("aria-invalid")).toBe("true");
  });

  it("does not overwrite an aria-label the control already has", async () => {
    const el = await mount({ label: "Cargue" }, '<input aria-label="Mío" />');
    expect(el.querySelector("input")!.getAttribute("aria-label")).toBe("Mío");
  });

  it("clicking the label focuses the control", async () => {
    const el = await mount({ label: "Desde" }, "<input />");
    const input = el.querySelector("input")!;
    q(el, "label")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(document.activeElement).toBe(input);
  });

  it("clicking the label focuses a tx control through its focus()", async () => {
    const el = await mount({ label: "Desde" });
    const input = el.querySelector("tx-input") as TxInput;
    const spy = vi.spyOn(input, "focus");
    q(el, "label")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(spy).toHaveBeenCalled();
  });
});
