import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-select-menu";
import type { TxSelectMenu } from "../../src/components/tx-select-menu";

afterEach(() => {
  document.body.innerHTML = "";
});

const ITEMS = [
  { value: "20260105", label: "Cargue enero" },
  { value: "20260205", label: "Cargue febrero" },
  { value: "20260305", label: "Cargue marzo", disabled: true },
  { value: "20260405", label: "Cargue abril" },
];

async function mount(props: Partial<TxSelectMenu> = {}) {
  const el = document.createElement("tx-select-menu") as TxSelectMenu;
  Object.assign(el, { items: ITEMS, ...props });
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const input = (el: TxSelectMenu) => el.shadowRoot!.querySelector<HTMLInputElement>('input[role="combobox"]')!;
const listbox = (el: TxSelectMenu) => el.shadowRoot!.querySelector<HTMLElement>('[role="listbox"]')!;
const opts = (el: TxSelectMenu) => [...el.shadowRoot!.querySelectorAll<HTMLElement>('[role="option"]')];
const labels = (el: TxSelectMenu) => opts(el).map((o) => o.textContent!.trim());

async function open(el: TxSelectMenu) {
  input(el).dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));
  await el.updateComplete;
}

async function press(el: TxSelectMenu, key: string) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, composed: true, cancelable: true });
  input(el).dispatchEvent(event);
  await el.updateComplete;
  return event;
}

async function typeText(el: TxSelectMenu, text: string) {
  input(el).value = text;
  input(el).dispatchEvent(new Event("input", { bubbles: true, composed: true }));
  await el.updateComplete;
}

describe("tx-select-menu", () => {
  it("is a closed combobox with listbox semantics", async () => {
    const el = await mount();
    const box = input(el);
    expect(box.getAttribute("aria-expanded")).toBe("false");
    expect(box.getAttribute("aria-haspopup")).toBe("listbox");
    expect(box.getAttribute("aria-autocomplete")).toBe("list");
    expect(box.getAttribute("aria-controls")).toBe(listbox(el).id);
    expect(listbox(el).hidden).toBe(true);
  });

  it("shows the label of the selected item or the placeholder", async () => {
    const el = await mount({ value: "20260205", placeholder: "Elige un cargue" });
    expect(input(el).value).toBe("Cargue febrero");
    el.value = undefined;
    await el.updateComplete;
    expect(input(el).value).toBe("");
    expect(input(el).placeholder).toBe("Elige un cargue");
  });

  it("opens on click and lists every item with the selected one marked", async () => {
    const el = await mount({ value: "20260205" });
    await open(el);
    expect(input(el).getAttribute("aria-expanded")).toBe("true");
    expect(listbox(el).hidden).toBe(false);
    expect(labels(el)).toEqual(["Cargue enero", "Cargue febrero", "Cargue marzo", "Cargue abril"]);
    expect(opts(el).map((o) => o.getAttribute("aria-selected"))).toEqual(["false", "true", "false", "false"]);
    expect(opts(el)[2]!.getAttribute("aria-disabled")).toBe("true");
  });

  it("filters as you type, ignoring case and accents", async () => {
    const el = await mount({ items: [{ value: 1, label: "Preparación" }, { value: 2, label: "Descarga" }] });
    await open(el);
    await typeText(el, "PREPARACION");
    expect(labels(el)).toEqual(["Preparación"]);
    await typeText(el, "esc");
    expect(labels(el)).toEqual(["Descarga"]);
  });

  it("opens when typing even while closed", async () => {
    const el = await mount();
    await typeText(el, "abr");
    expect(input(el).getAttribute("aria-expanded")).toBe("true");
    expect(labels(el)).toEqual(["Cargue abril"]);
  });

  it("says so when nothing matches", async () => {
    const el = await mount();
    await open(el);
    await typeText(el, "zzz");
    expect(opts(el)).toHaveLength(0);
    expect(listbox(el).textContent).toContain("Sin resultados");
  });

  it("ArrowDown opens the menu, then moves the active option skipping disabled ones", async () => {
    const el = await mount();
    await press(el, "ArrowDown");
    expect(input(el).getAttribute("aria-expanded")).toBe("true");
    const activeId = () => input(el).getAttribute("aria-activedescendant");
    expect(activeId()).toBe(opts(el)[0]!.id);
    await press(el, "ArrowDown");
    expect(activeId()).toBe(opts(el)[1]!.id);
    await press(el, "ArrowDown"); // marzo is disabled
    expect(activeId()).toBe(opts(el)[3]!.id);
    await press(el, "ArrowDown"); // stays on the last option
    expect(activeId()).toBe(opts(el)[3]!.id);
    await press(el, "ArrowUp");
    expect(activeId()).toBe(opts(el)[1]!.id);
    expect(opts(el)[1]!.getAttribute("data-active")).toBe("true");
  });

  it("Enter selects the active option, emits events and closes", async () => {
    const el = await mount();
    const events: Array<[string, unknown]> = [];
    for (const name of ["tx-input", "tx-change"]) {
      el.addEventListener(name, (e) => events.push([name, (e as CustomEvent).detail]));
    }
    await press(el, "ArrowDown");
    await press(el, "ArrowDown");
    const event = await press(el, "Enter");
    expect(event.defaultPrevented).toBe(true);
    expect(el.value).toBe("20260205");
    expect(events).toEqual([
      ["tx-input", { value: "20260205", item: ITEMS[1] }],
      ["tx-change", { value: "20260205", item: ITEMS[1] }],
    ]);
    expect(input(el).getAttribute("aria-expanded")).toBe("false");
    expect(input(el).value).toBe("Cargue febrero");
  });

  it("Enter after filtering picks the first match", async () => {
    const el = await mount();
    await typeText(el, "abril");
    await press(el, "Enter");
    expect(el.value).toBe("20260405");
  });

  it("Enter on a closed menu does nothing and is not swallowed", async () => {
    const el = await mount();
    const event = await press(el, "Enter");
    expect(event.defaultPrevented).toBe(false);
    expect(el.value).toBeUndefined();
  });

  it("Escape closes without selecting, restores the label and is consumed only while open", async () => {
    const el = await mount({ value: "20260105" });
    await open(el);
    await typeText(el, "feb");
    const closing = await press(el, "Escape");
    expect(closing.defaultPrevented).toBe(true);
    expect(input(el).getAttribute("aria-expanded")).toBe("false");
    expect(el.value).toBe("20260105");
    expect(input(el).value).toBe("Cargue enero");
    const idle = await press(el, "Escape");
    expect(idle.defaultPrevented).toBe(false);
  });

  it("Tab closes the menu without selecting", async () => {
    const el = await mount();
    await press(el, "ArrowDown");
    await press(el, "Tab");
    expect(input(el).getAttribute("aria-expanded")).toBe("false");
    expect(el.value).toBeUndefined();
  });

  it("clicking an option selects it, and ignores disabled ones", async () => {
    const el = await mount();
    await open(el);
    opts(el)[2]!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await el.updateComplete;
    expect(el.value).toBeUndefined();
    expect(input(el).getAttribute("aria-expanded")).toBe("true");
    opts(el)[3]!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await el.updateComplete;
    expect(el.value).toBe("20260405");
    expect(input(el).getAttribute("aria-expanded")).toBe("false");
  });

  it("keeps focus on the input when an option is pressed", async () => {
    const el = await mount();
    await open(el);
    const down = new MouseEvent("mousedown", { bubbles: true, cancelable: true });
    opts(el)[0]!.dispatchEvent(down);
    expect(down.defaultPrevented).toBe(true);
  });

  it("closes on an outside press but not on a press inside", async () => {
    const el = await mount();
    await open(el);
    opts(el)[0]!.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, composed: true }));
    await el.updateComplete;
    expect(input(el).getAttribute("aria-expanded")).toBe("true");
    document.body.dispatchEvent(new MouseEvent("pointerdown", { bubbles: true, composed: true }));
    await el.updateComplete;
    expect(input(el).getAttribute("aria-expanded")).toBe("false");
  });

  it("supports custom value-key and label-key", async () => {
    const el = await mount({
      items: [{ id: "a", name: "Alfa" }, { id: "b", name: "Beta" }],
      valueKey: "id",
      labelKey: "name",
      value: "b",
    });
    expect(input(el).value).toBe("Beta");
    await open(el);
    expect(labels(el)).toEqual(["Alfa", "Beta"]);
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    opts(el)[0]!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect((handler.mock.calls[0]![0] as CustomEvent).detail.value).toBe("a");
  });

  it("maps the value-key and label-key attributes", async () => {
    const el = document.createElement("tx-select-menu") as TxSelectMenu;
    el.setAttribute("value-key", "id");
    el.setAttribute("label-key", "name");
    expect(el.valueKey).toBe("id");
    expect(el.labelKey).toBe("name");
  });

  it("accepts plain strings as items", async () => {
    const el = await mount({ items: ["uno", "dos"], value: "dos" });
    expect(input(el).value).toBe("dos");
    await open(el);
    expect(labels(el)).toEqual(["uno", "dos"]);
  });

  it("does not open when disabled", async () => {
    const el = await mount({ disabled: true });
    expect(input(el).disabled).toBe(true);
    await open(el);
    expect(input(el).getAttribute("aria-expanded")).toBe("false");
  });

  it("flags invalid state, label and description", async () => {
    const el = await mount({ invalid: true, fieldLabel: "Cargue", fieldDescription: "Obligatorio" });
    const box = input(el);
    expect(box.getAttribute("aria-invalid")).toBe("true");
    expect(box.getAttribute("aria-label")).toBe("Cargue");
    expect(el.shadowRoot!.getElementById(box.getAttribute("aria-describedby")!)!.textContent).toBe("Obligatorio");
  });

  it("stops listening for outside presses once removed", async () => {
    const el = await mount();
    const remove = vi.spyOn(document, "removeEventListener");
    el.remove();
    expect(remove).toHaveBeenCalledWith("pointerdown", expect.any(Function), true);
  });
});
