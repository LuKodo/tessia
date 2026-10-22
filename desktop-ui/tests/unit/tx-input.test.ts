import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-input";
import type { TxInput } from "../../src/components/tx-input";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(props: Partial<TxInput> = {}) {
  const el = document.createElement("tx-input") as TxInput;
  Object.assign(el, props);
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const inner = (el: TxInput) => el.shadowRoot!.querySelector("input")!;

function type(el: TxInput, value: string) {
  const input = inner(el);
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
}

describe("tx-input", () => {
  it("renders the native input with its attributes", async () => {
    const el = await mount({ type: "number", value: "5", placeholder: "Cantidad", min: "0", max: "100", step: "5", name: "qty" });
    const input = inner(el);
    expect(input.type).toBe("number");
    expect(input.value).toBe("5");
    expect(input.placeholder).toBe("Cantidad");
    expect(input.getAttribute("min")).toBe("0");
    expect(input.getAttribute("max")).toBe("100");
    expect(input.getAttribute("step")).toBe("5");
    expect(input.name).toBe("qty");
  });

  it("supports the date, search and password types", async () => {
    for (const type of ["date", "search", "password"] as const) {
      expect(inner(await mount({ type })).type).toBe(type);
    }
  });

  it("follows value changes made from outside", async () => {
    const el = await mount({ value: "a" });
    el.value = "b";
    await el.updateComplete;
    expect(inner(el).value).toBe("b");
  });

  it("emits a bubbling composed tx-input on every edit and keeps value in sync", async () => {
    const el = await mount();
    const seen: CustomEvent[] = [];
    document.body.addEventListener("tx-input", (e) => seen.push(e as CustomEvent));
    type(el, "ho");
    type(el, "hol");
    expect(el.value).toBe("hol");
    expect(seen.map((e) => e.detail)).toEqual([{ value: "ho" }, { value: "hol" }]);
    expect(seen[0]!.composed).toBe(true);
    expect(seen[0]!.bubbles).toBe(true);
  });

  it("emits tx-change when the edit is committed", async () => {
    const el = await mount({ value: "x" });
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    type(el, "y");
    expect(handler).not.toHaveBeenCalled();
    inner(el).dispatchEvent(new Event("change", { bubbles: true }));
    expect(handler).toHaveBeenCalledTimes(1);
    expect((handler.mock.calls[0]![0] as CustomEvent).detail).toEqual({ value: "y" });
  });

  it("disables the native input", async () => {
    const el = await mount({ disabled: true });
    expect(inner(el).disabled).toBe(true);
  });

  it("flags invalid state through aria-invalid, also with color error", async () => {
    const el = await mount();
    expect(inner(el).hasAttribute("aria-invalid")).toBe(false);
    el.invalid = true;
    await el.updateComplete;
    expect(inner(el).getAttribute("aria-invalid")).toBe("true");
    el.invalid = false;
    el.color = "error";
    await el.updateComplete;
    expect(inner(el).getAttribute("aria-invalid")).toBe("true");
  });

  it("names the input from aria-label, falling back to the field label", async () => {
    const el = await mount({ fieldLabel: "Desde" });
    expect(inner(el).getAttribute("aria-label")).toBe("Desde");
    el.setAttribute("aria-label", "Buscar factura");
    await el.updateComplete;
    expect(inner(el).getAttribute("aria-label")).toBe("Buscar factura");
  });

  it("describes the input with the field description", async () => {
    const el = await mount({ fieldDescription: "Ayuda" });
    const id = inner(el).getAttribute("aria-describedby")!;
    expect(el.shadowRoot!.getElementById(id)!.textContent).toBe("Ayuda");
    el.fieldDescription = "";
    await el.updateComplete;
    expect(inner(el).hasAttribute("aria-describedby")).toBe(false);
  });

  it("exposes leading and trailing slots", async () => {
    const el = await mount();
    expect(el.shadowRoot!.querySelector('slot[name="leading"]')).not.toBeNull();
    expect(el.shadowRoot!.querySelector('slot[name="trailing"]')).not.toBeNull();
  });

  it("reflects size, variant and color with defaults", async () => {
    const el = await mount();
    expect(el.getAttribute("size")).toBe("md");
    expect(el.getAttribute("variant")).toBe("outline");
    expect(el.getAttribute("color")).toBe("primary");
  });

  it("focus() focuses the inner input", async () => {
    const el = await mount();
    el.focus();
    expect(el.shadowRoot!.activeElement).toBe(inner(el));
  });
});
