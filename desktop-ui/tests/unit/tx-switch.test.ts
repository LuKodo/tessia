import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-switch";
import type { TxSwitch } from "../../src/components/tx-switch";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(props: Partial<TxSwitch> = {}) {
  const el = document.createElement("tx-switch") as TxSwitch;
  Object.assign(el, props);
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const control = (el: TxSwitch) => el.shadowRoot!.querySelector<HTMLInputElement>("input")!;

function flip(el: TxSwitch, checked: boolean) {
  const input = control(el);
  input.checked = checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("tx-switch", () => {
  it("renders an accessible switch reflecting checked", async () => {
    const el = await mount({ checked: true });
    const input = control(el);
    expect(input.getAttribute("role")).toBe("switch");
    expect(input.checked).toBe(true);
    el.checked = false;
    await el.updateComplete;
    expect(input.checked).toBe(false);
  });

  it("renders the label and a description", async () => {
    const el = await mount({ label: "Incluir excluidas y fallas", description: "Amplía la lista" });
    expect(el.shadowRoot!.querySelector(".label")!.textContent).toContain("Incluir excluidas y fallas");
    expect(el.shadowRoot!.querySelector(".description")!.textContent).toContain("Amplía la lista");
  });

  it("emits tx-input and tx-change with the new state", async () => {
    const el = await mount();
    const events: Array<[string, unknown]> = [];
    for (const name of ["tx-input", "tx-change"]) {
      el.addEventListener(name, (e) => events.push([name, (e as CustomEvent).detail]));
    }
    flip(el, true);
    expect(el.checked).toBe(true);
    expect(events).toEqual([
      ["tx-input", { value: true, checked: true }],
      ["tx-change", { value: true, checked: true }],
    ]);
  });

  it("disables the control", async () => {
    expect(control(await mount({ disabled: true })).disabled).toBe(true);
  });

  it("names the switch from aria-label or the field label when there is no visible label", async () => {
    const el = await mount({ fieldLabel: "Modo" });
    expect(control(el).getAttribute("aria-label")).toBe("Modo");
    el.label = "Visible";
    await el.updateComplete;
    expect(control(el).hasAttribute("aria-label")).toBe(false);
  });

  it("reflects color and size with defaults", async () => {
    const el = await mount({ color: "success" });
    expect(el.getAttribute("color")).toBe("success");
    expect(el.getAttribute("size")).toBe("md");
  });

  it("focus() focuses the native control", async () => {
    const el = await mount();
    el.focus();
    expect(el.shadowRoot!.activeElement).toBe(control(el));
  });
});
