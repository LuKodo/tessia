import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-accordion";
import type { TxAccordion } from "../../src/components/tx-accordion";

const ITEMS = [
  { value: "a", label: "Primero" },
  { value: "b", label: "Segundo" },
  { value: "c", label: "Tercero", disabled: true },
  { value: "d", label: "Cuarto" },
];

async function mount(props: Partial<TxAccordion> = {}, light = ""): Promise<TxAccordion> {
  const el = document.createElement("tx-accordion") as TxAccordion;
  el.items = ITEMS;
  Object.assign(el, props);
  el.innerHTML = light || `<p slot="a">cuerpo a</p><p slot="b">cuerpo b</p>`;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const triggers = (el: TxAccordion) => [...el.shadowRoot!.querySelectorAll<HTMLButtonElement>("button.trigger")];
const regions = (el: TxAccordion) => [...el.shadowRoot!.querySelectorAll<HTMLElement>("[role=region]")];
const expanded = (el: TxAccordion) => triggers(el).map((t) => t.getAttribute("aria-expanded"));
const press = (target: HTMLElement, key: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-accordion", () => {
  it("renders one collapsed trigger per item with labelled regions", async () => {
    const el = await mount();
    expect(triggers(el).map((t) => t.textContent!.trim())).toEqual(["Primero", "Segundo", "Tercero", "Cuarto"]);
    expect(expanded(el)).toEqual(["false", "false", "false", "false"]);
    expect(regions(el).every((r) => r.hidden)).toBe(true);
    triggers(el).forEach((t, i) => {
      expect(t.getAttribute("aria-controls")).toBe(regions(el)[i]!.id);
      expect(regions(el)[i]!.getAttribute("aria-labelledby")).toBe(t.id);
    });
  });

  it("opens an item on click, exposing aria-expanded, and projects its body slot", async () => {
    const el = await mount();
    triggers(el)[0]!.click();
    await el.updateComplete;
    expect(expanded(el)).toEqual(["true", "false", "false", "false"]);
    expect(regions(el)[0]!.hidden).toBe(false);
    expect(regions(el)[0]!.querySelector("slot")!.getAttribute("name")).toBe("a");
    expect(el.value).toEqual(["a"]);
  });

  it("single mode closes the previous item and collapses the open one when collapsible", async () => {
    const el = await mount();
    triggers(el)[0]!.click();
    triggers(el)[1]!.click();
    await el.updateComplete;
    expect(el.value).toEqual(["b"]);
    triggers(el)[1]!.click();
    await el.updateComplete;
    expect(el.value).toEqual([]);
  });

  it("single mode with collapsible=false keeps the open item open", async () => {
    const el = await mount({ collapsible: false, value: ["a"] });
    triggers(el)[0]!.click();
    await el.updateComplete;
    expect(el.value).toEqual(["a"]);
  });

  it("multiple mode keeps several items open", async () => {
    const el = await mount({ type: "multiple" });
    triggers(el)[0]!.click();
    triggers(el)[1]!.click();
    await el.updateComplete;
    expect(expanded(el)).toEqual(["true", "true", "false", "false"]);
    expect(el.value).toEqual(["a", "b"]);
    triggers(el)[0]!.click();
    await el.updateComplete;
    expect(el.value).toEqual(["b"]);
  });

  it("fires tx-change with the open values", async () => {
    const el = await mount({ type: "multiple" });
    const seen: string[][] = [];
    el.addEventListener("tx-change", (e) => seen.push((e as CustomEvent).detail.value));
    triggers(el)[0]!.click();
    triggers(el)[1]!.click();
    expect(seen).toEqual([["a"], ["a", "b"]]);
  });

  it("starts with the items listed in `value` open and does not mutate the array it was given", async () => {
    const initial = ["b"];
    const el = await mount({ value: initial });
    expect(expanded(el)).toEqual(["false", "true", "false", "false"]);
    triggers(el)[0]!.click();
    await el.updateComplete;
    expect(initial).toEqual(["b"]);
  });

  it("does not open a disabled item", async () => {
    const el = await mount();
    expect(triggers(el)[2]!.disabled).toBe(true);
    triggers(el)[2]!.click();
    await el.updateComplete;
    expect(el.value).toEqual([]);
  });

  it("arrow keys, Home and End move focus between enabled triggers", async () => {
    const el = await mount();
    triggers(el)[0]!.focus();
    press(triggers(el)[0]!, "ArrowDown");
    expect(el.shadowRoot!.activeElement).toBe(triggers(el)[1]);
    press(triggers(el)[1]!, "ArrowDown"); // skips the disabled one
    expect(el.shadowRoot!.activeElement).toBe(triggers(el)[3]);
    press(triggers(el)[3]!, "ArrowDown"); // wraps
    expect(el.shadowRoot!.activeElement).toBe(triggers(el)[0]);
    press(triggers(el)[0]!, "ArrowUp");
    expect(el.shadowRoot!.activeElement).toBe(triggers(el)[3]);
    press(triggers(el)[3]!, "Home");
    expect(el.shadowRoot!.activeElement).toBe(triggers(el)[0]);
    press(triggers(el)[0]!, "End");
    expect(el.shadowRoot!.activeElement).toBe(triggers(el)[3]);
  });

  it("lets a `label-<value>` slot replace the label text", async () => {
    const el = await mount({}, `<span slot="label-a">Etiqueta rica</span>`);
    expect(triggers(el)[0]!.querySelector("slot[name=label-a]")).not.toBeNull();
    expect(triggers(el)[0]!.querySelector("slot[name=label-a]")!.textContent).toContain("Primero");
  });
});
