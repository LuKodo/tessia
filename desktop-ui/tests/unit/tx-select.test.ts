import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-select";
import type { TxSelect } from "../../src/components/tx-select";

afterEach(() => {
  document.body.innerHTML = "";
});

const ITEMS = [
  { value: "all", label: "(todas)" },
  { value: "PREP", label: "Preparación" },
  { value: "DOWN", label: "Descarga", disabled: true },
];

async function mount(props: Partial<TxSelect> = {}) {
  const el = document.createElement("tx-select") as TxSelect;
  Object.assign(el, { items: ITEMS, ...props });
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const native = (el: TxSelect) => el.shadowRoot!.querySelector("select")!;
const options = (el: TxSelect) => [...el.shadowRoot!.querySelectorAll("option")];

function choose(el: TxSelect, value: string) {
  const select = native(el);
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("tx-select", () => {
  it("renders one option per item and selects the current value", async () => {
    const el = await mount({ value: "PREP" });
    expect(options(el).map((o) => o.textContent!.trim())).toEqual(["(todas)", "Preparación", "Descarga"]);
    expect(native(el).value).toBe("PREP");
  });

  it("marks disabled items", async () => {
    const el = await mount();
    expect(options(el).map((o) => o.disabled)).toEqual([false, false, true]);
  });

  it("accepts plain strings as items", async () => {
    const el = await mount({ items: ["a", "b"], value: "b" });
    expect(options(el).map((o) => o.textContent!.trim())).toEqual(["a", "b"]);
    expect(native(el).value).toBe("b");
  });

  it("shows a disabled placeholder option while nothing is selected", async () => {
    const el = await mount({ placeholder: "Elige uno", value: "" });
    const first = options(el)[0]!;
    expect(first.textContent!.trim()).toBe("Elige uno");
    expect(first.disabled).toBe(true);
    expect(native(el).value).toBe("");
    el.value = "PREP";
    await el.updateComplete;
    expect(native(el).value).toBe("PREP");
  });

  it("emits tx-input and tx-change with the chosen value", async () => {
    const el = await mount({ value: "all" });
    const events: Array<[string, unknown]> = [];
    for (const name of ["tx-input", "tx-change"]) {
      el.addEventListener(name, (e) => events.push([name, (e as CustomEvent).detail]));
    }
    choose(el, "PREP");
    expect(el.value).toBe("PREP");
    expect(events).toEqual([
      ["tx-input", { value: "PREP" }],
      ["tx-change", { value: "PREP" }],
    ]);
  });

  it("keeps numeric item values numeric", async () => {
    const el = await mount({ items: [{ value: 1, label: "Uno" }, { value: 2, label: "Dos" }], value: 1 });
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    choose(el, "2");
    expect((handler.mock.calls[0]![0] as CustomEvent).detail).toEqual({ value: 2 });
    expect(el.value).toBe(2);
  });

  it("disables the native select", async () => {
    expect(native(await mount({ disabled: true })).disabled).toBe(true);
  });

  it("flags invalid state, label and description", async () => {
    const el = await mount({ invalid: true, fieldLabel: "Fase", fieldDescription: "Elige una" });
    const select = native(el);
    expect(select.getAttribute("aria-invalid")).toBe("true");
    expect(select.getAttribute("aria-label")).toBe("Fase");
    expect(el.shadowRoot!.getElementById(select.getAttribute("aria-describedby")!)!.textContent).toBe("Elige una");
  });

  it("draws a chevron hidden from assistive tech", async () => {
    const el = await mount();
    expect(el.shadowRoot!.querySelector("svg")!.getAttribute("aria-hidden")).toBe("true");
  });

  it("focus() focuses the native select", async () => {
    const el = await mount();
    el.focus();
    expect(el.shadowRoot!.activeElement).toBe(native(el));
  });
});
