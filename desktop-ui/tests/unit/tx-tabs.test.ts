import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-tabs";
import type { TxTabs } from "../../src/components/tx-tabs";

const ITEMS = [
  { label: "Operar", value: "operate" },
  { label: "Analizar", value: "analyze" },
  { label: "Archivo", value: "archive", disabled: true },
  { label: "Ajustes", value: "settings" },
];

async function mount(props: Partial<TxTabs> = {}, light = ""): Promise<TxTabs> {
  const el = document.createElement("tx-tabs") as TxTabs;
  el.items = ITEMS;
  Object.assign(el, props);
  el.innerHTML = light || `<div slot="operate">panel operar</div><div slot="analyze">panel analizar</div><div slot="settings">ajustes</div>`;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const tabs = (el: TxTabs) => [...el.shadowRoot!.querySelectorAll<HTMLButtonElement>("[role=tab]")];
const panels = (el: TxTabs) => [...el.shadowRoot!.querySelectorAll<HTMLElement>("[role=tabpanel]")];
const press = (target: HTMLElement, key: string) => target.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-tabs", () => {
  it("renders an ARIA tablist with one tab per item and selects the first by default", async () => {
    const el = await mount();
    expect(el.shadowRoot!.querySelector("[role=tablist]")).not.toBeNull();
    expect(tabs(el).map((t) => t.textContent!.trim())).toEqual(["Operar", "Analizar", "Archivo", "Ajustes"]);
    expect(tabs(el).map((t) => t.getAttribute("aria-selected"))).toEqual(["true", "false", "false", "false"]);
    expect(el.value).toBe("operate");
  });

  it("honors the initial value and uses a roving tabindex", async () => {
    const el = await mount({ value: "analyze" });
    expect(tabs(el).map((t) => t.getAttribute("aria-selected"))).toEqual(["false", "true", "false", "false"]);
    expect(tabs(el).map((t) => t.tabIndex)).toEqual([-1, 0, -1, -1]);
  });

  it("connects each tab to its panel (aria-controls / aria-labelledby) and shows only the active one", async () => {
    const el = await mount();
    const ps = panels(el);
    expect(ps).toHaveLength(4);
    tabs(el).forEach((tab, i) => {
      expect(tab.getAttribute("aria-controls")).toBe(ps[i]!.id);
      expect(ps[i]!.getAttribute("aria-labelledby")).toBe(tab.id);
    });
    expect(ps.map((p) => p.hidden)).toEqual([false, true, true, true]);
  });

  it("projects each panel from the slot named after the item value", async () => {
    const el = await mount();
    expect(panels(el)[0]!.querySelector("slot")!.getAttribute("name")).toBe("operate");
    expect(panels(el)[1]!.querySelector("slot")!.getAttribute("name")).toBe("analyze");
  });

  it("clicking a tab selects it and fires tx-change with the value", async () => {
    const el = await mount();
    const seen: string[] = [];
    el.addEventListener("tx-change", (e) => seen.push((e as CustomEvent).detail.value));
    tabs(el)[1]!.click();
    await el.updateComplete;
    expect(el.value).toBe("analyze");
    expect(seen).toEqual(["analyze"]);
    expect(panels(el).map((p) => p.hidden)).toEqual([true, false, true, true]);
    tabs(el)[1]!.click(); // already selected: no event
    expect(seen).toEqual(["analyze"]);
  });

  it("does not select a disabled tab", async () => {
    const el = await mount();
    expect(tabs(el)[2]!.disabled).toBe(true);
    tabs(el)[2]!.click();
    await el.updateComplete;
    expect(el.value).toBe("operate");
  });

  it("arrow keys move the selection and focus, wrap around and skip disabled tabs", async () => {
    const el = await mount();
    const seen: string[] = [];
    el.addEventListener("tx-change", (e) => seen.push((e as CustomEvent).detail.value));
    press(tabs(el)[0]!, "ArrowRight");
    await el.updateComplete;
    expect(el.value).toBe("analyze");
    expect(el.shadowRoot!.activeElement).toBe(tabs(el)[1]);
    press(tabs(el)[1]!, "ArrowRight"); // skips "archive" (disabled)
    await el.updateComplete;
    expect(el.value).toBe("settings");
    press(tabs(el)[3]!, "ArrowRight"); // wraps
    await el.updateComplete;
    expect(el.value).toBe("operate");
    press(tabs(el)[0]!, "ArrowLeft"); // wraps backwards
    await el.updateComplete;
    expect(el.value).toBe("settings");
    expect(seen).toEqual(["analyze", "settings", "operate", "settings"]);
  });

  it("Home and End jump to the first and last enabled tab", async () => {
    const el = await mount({ value: "analyze" });
    press(tabs(el)[1]!, "End");
    await el.updateComplete;
    expect(el.value).toBe("settings");
    press(tabs(el)[3]!, "Home");
    await el.updateComplete;
    expect(el.value).toBe("operate");
  });

  it("falls back to the first enabled tab when the value matches no item", async () => {
    const el = await mount({ value: "nope" });
    expect(el.value).toBe("operate");
  });

  it("renders an item icon by name before the label", async () => {
    const el = await mount({ items: [{ label: "Uno", value: "a", icon: "history" }, { label: "Dos", value: "b" }] });
    expect(tabs(el)[0]!.querySelector("svg")).not.toBeNull();
    expect(tabs(el)[1]!.querySelector("svg")).toBeNull();
  });

  it("reflects variant on the tablist", async () => {
    const el = await mount({ variant: "pill" });
    expect(el.shadowRoot!.querySelector("[role=tablist]")!.getAttribute("data-variant")).toBe("pill");
  });
});
