import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-checkbox-group";
import type { TxCheckboxGroup } from "../../src/components/tx-checkbox-group";
import type { TxCheckbox } from "../../src/components/tx-checkbox";

afterEach(() => {
  document.body.innerHTML = "";
});

const ITEMS = [
  { value: "evento pos", label: "EVENTO POS" },
  { value: "evento no pbs", label: "EVENTO NO PBS", description: "No financiado" },
  { value: "transferencia", label: "TRANSFERENCIA", disabled: true },
];

async function mount(props: Partial<TxCheckboxGroup> = {}) {
  const el = document.createElement("tx-checkbox-group") as TxCheckboxGroup;
  Object.assign(el, { items: ITEMS, value: [], ...props });
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const boxes = (el: TxCheckboxGroup) => [...el.shadowRoot!.querySelectorAll<TxCheckbox>("tx-checkbox")];

async function flip(el: TxCheckboxGroup, index: number, checked: boolean) {
  const cb = boxes(el)[index]!;
  await cb.updateComplete;
  const input = cb.shadowRoot!.querySelector("input")!;
  input.checked = checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

describe("tx-checkbox-group", () => {
  it("renders one checkbox per item with its label, description and checked state", async () => {
    const el = await mount({ value: ["evento no pbs"] });
    const list = boxes(el);
    expect(list.map((c) => c.label)).toEqual(["EVENTO POS", "EVENTO NO PBS", "TRANSFERENCIA"]);
    expect(list.map((c) => c.checked)).toEqual([false, true, false]);
    expect(list[1]!.description).toBe("No financiado");
    expect(list.map((c) => c.disabled)).toEqual([false, false, true]);
  });

  it("accepts plain strings as items", async () => {
    const el = await mount({ items: ["a", "b"], value: ["b"] });
    expect(boxes(el).map((c) => c.label)).toEqual(["a", "b"]);
    expect(boxes(el).map((c) => c.checked)).toEqual([false, true]);
  });

  it("adds a value when checked and removes it when unchecked", async () => {
    const el = await mount({ value: ["evento pos"] });
    const changes: string[][] = [];
    el.addEventListener("tx-change", (e) => changes.push((e as CustomEvent).detail.value));
    await flip(el, 1, true);
    expect(el.value).toEqual(["evento pos", "evento no pbs"]);
    await flip(el, 0, false);
    expect(el.value).toEqual(["evento no pbs"]);
    expect(changes).toEqual([["evento pos", "evento no pbs"], ["evento no pbs"]]);
  });

  it("emits exactly one tx-input and one tx-change per toggle and hides the child events", async () => {
    const el = await mount();
    const seen: string[] = [];
    document.body.addEventListener("tx-input", () => seen.push("input"));
    document.body.addEventListener("tx-change", () => seen.push("change"));
    await flip(el, 0, true);
    expect(seen).toEqual(["input", "change"]);
  });

  it("does not mutate the array it was given", async () => {
    const original = ["evento pos"];
    const el = await mount({ value: original });
    await flip(el, 1, true);
    expect(original).toEqual(["evento pos"]);
  });

  it("disables every checkbox when the group is disabled", async () => {
    const el = await mount({ disabled: true });
    expect(boxes(el).every((c) => c.disabled)).toBe(true);
  });

  it("passes color and size down", async () => {
    const el = await mount({ color: "warning", size: "lg" });
    expect(boxes(el).every((c) => c.color === "warning" && c.size === "lg")).toBe(true);
  });

  it("groups the checkboxes with an accessible name", async () => {
    const el = await mount({ legend: "Subplanes" });
    const set = el.shadowRoot!.querySelector("fieldset")!;
    expect(set.querySelector("legend")!.textContent).toBe("Subplanes");
    const named = await mount({ fieldLabel: "Subplanes" });
    expect(named.shadowRoot!.querySelector("fieldset")!.getAttribute("aria-label")).toBe("Subplanes");
  });

  it("lays out horizontally on request", async () => {
    const el = await mount({ orientation: "horizontal" });
    expect(el.shadowRoot!.querySelector("fieldset")!.classList.contains("horizontal")).toBe(true);
  });

  it("describes the group with the field description", async () => {
    const el = await mount({ fieldDescription: "Elige al menos uno" });
    const set = el.shadowRoot!.querySelector("fieldset")!;
    expect(el.shadowRoot!.getElementById(set.getAttribute("aria-describedby")!)!.textContent).toBe("Elige al menos uno");
  });

  it("ignores an event from a disabled item", async () => {
    const el = await mount();
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    await flip(el, 2, true);
    expect(handler).not.toHaveBeenCalled();
    expect(el.value).toEqual([]);
  });
});
