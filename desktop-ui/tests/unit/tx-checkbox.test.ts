import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-checkbox";
import type { TxCheckbox } from "../../src/components/tx-checkbox";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(props: Partial<TxCheckbox> = {}, inner = "") {
  const el = document.createElement("tx-checkbox") as TxCheckbox;
  Object.assign(el, props);
  el.innerHTML = inner;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const box = (el: TxCheckbox) => el.shadowRoot!.querySelector<HTMLInputElement>('input[type="checkbox"]')!;

function toggle(el: TxCheckbox, checked: boolean) {
  const input = box(el);
  input.checked = checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("tx-checkbox", () => {
  it("renders a native checkbox reflecting checked", async () => {
    const el = await mount({ checked: true });
    expect(box(el).checked).toBe(true);
    el.checked = false;
    await el.updateComplete;
    expect(box(el).checked).toBe(false);
  });

  it("renders the label and description from props", async () => {
    const el = await mount({ label: "Incluir", description: "Todas las fases" });
    expect(el.shadowRoot!.querySelector(".label")!.textContent).toContain("Incluir");
    expect(el.shadowRoot!.querySelector(".description")!.textContent).toContain("Todas las fases");
  });

  it("projects a rich label through the label slot", async () => {
    const el = await mount({}, '<span slot="label" id="rich">Alerta <b>3</b></span>');
    const slot = el.shadowRoot!.querySelector<HTMLSlotElement>('slot[name="label"]')!;
    expect(slot.assignedElements().map((n) => n.id)).toEqual(["rich"]);
  });

  it("emits tx-input then tx-change with the new state when toggled", async () => {
    const el = await mount();
    const seen: string[] = [];
    const details: unknown[] = [];
    for (const name of ["tx-input", "tx-change"]) {
      document.body.addEventListener(name, (e) => {
        seen.push(name);
        details.push((e as CustomEvent).detail);
      });
    }
    toggle(el, true);
    expect(el.checked).toBe(true);
    expect(seen).toEqual(["tx-input", "tx-change"]);
    expect(details[0]).toEqual({ value: true, checked: true });
    toggle(el, false);
    expect(el.checked).toBe(false);
    expect(details[2]).toEqual({ value: false, checked: false });
  });

  it("is composed and bubbling so pages can listen on ancestors", async () => {
    const el = await mount();
    const handler = vi.fn();
    document.body.addEventListener("tx-change", handler);
    toggle(el, true);
    const event = handler.mock.calls[0]![0] as CustomEvent;
    expect(event.bubbles).toBe(true);
    expect(event.composed).toBe(true);
  });

  it("shows the indeterminate state and clears it on toggle", async () => {
    const el = await mount({ indeterminate: true });
    expect(box(el).indeterminate).toBe(true);
    toggle(el, true);
    await el.updateComplete;
    expect(el.indeterminate).toBe(false);
    expect(box(el).indeterminate).toBe(false);
  });

  it("disables the native checkbox", async () => {
    const el = await mount({ disabled: true });
    expect(box(el).disabled).toBe(true);
  });

  it("names the checkbox from aria-label when there is no visible label", async () => {
    const el = await mount({ fieldLabel: "Campo" });
    expect(box(el).getAttribute("aria-label")).toBe("Campo");
    el.setAttribute("aria-label", "¿A01 genera glosa?");
    await el.updateComplete;
    expect(box(el).getAttribute("aria-label")).toBe("¿A01 genera glosa?");
  });

  it("does not add an aria-label that would shadow the visible label", async () => {
    const el = await mount({ label: "Incluir", fieldLabel: "Campo" });
    expect(box(el).hasAttribute("aria-label")).toBe(false);
  });

  it("flags invalid state and describes the checkbox", async () => {
    const el = await mount({ invalid: true, fieldDescription: "Obligatorio" });
    expect(box(el).getAttribute("aria-invalid")).toBe("true");
    expect(el.shadowRoot!.getElementById(box(el).getAttribute("aria-describedby")!)!.textContent).toBe("Obligatorio");
  });

  it("reflects color and size with defaults", async () => {
    const el = await mount({ color: "warning" });
    expect(el.getAttribute("color")).toBe("warning");
    expect(el.getAttribute("size")).toBe("md");
  });

  it("focus() focuses the native checkbox", async () => {
    const el = await mount();
    el.focus();
    expect(el.shadowRoot!.activeElement).toBe(box(el));
  });
});
