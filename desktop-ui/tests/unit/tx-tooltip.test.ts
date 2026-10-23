import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-tooltip";
import type { TxTooltip } from "../../src/components/tx-tooltip";

async function mount(markup: string): Promise<TxTooltip> {
  const host = document.createElement("div");
  host.innerHTML = markup;
  document.body.append(host);
  const el = host.querySelector("tx-tooltip") as TxTooltip;
  await el.updateComplete;
  return el;
}

const bubble = (el: TxTooltip) => el.shadowRoot!.querySelector<HTMLElement>(".bubble")!;
const tip = (el: TxTooltip) => el.querySelector<HTMLElement>("[role=tooltip]")!;
const isOpen = (el: TxTooltip) => bubble(el).hasAttribute("data-open");

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  vi.useRealTimers();
  document.body.innerHTML = "";
});

describe("tx-tooltip", () => {
  it("is registered", () => {
    expect(customElements.get("tx-tooltip")).toBeDefined();
  });

  it("starts hidden and shows the text on hover, hiding again on leave", async () => {
    const el = await mount(`<tx-tooltip text="Paso 1: OK"><span id="t">punto</span></tx-tooltip>`);
    expect(isOpen(el)).toBe(false);
    el.dispatchEvent(new MouseEvent("mouseenter"));
    await el.updateComplete;
    expect(isOpen(el)).toBe(true);
    expect(tip(el).textContent).toBe("Paso 1: OK");
    el.dispatchEvent(new MouseEvent("mouseleave"));
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
  });

  it("shows on keyboard focus of the trigger and hides on blur", async () => {
    const el = await mount(`<tx-tooltip text="ayuda"><button id="b">?</button></tx-tooltip>`);
    el.querySelector<HTMLButtonElement>("#b")!.dispatchEvent(new FocusEvent("focusin", { bubbles: true, composed: true }));
    await el.updateComplete;
    expect(isOpen(el)).toBe(true);
    el.querySelector<HTMLButtonElement>("#b")!.dispatchEvent(new FocusEvent("focusout", { bubbles: true, composed: true }));
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
  });

  it("describes the trigger with the tooltip through aria-describedby", async () => {
    const el = await mount(`<tx-tooltip text="ayuda"><button id="b" aria-describedby="otro">?</button></tx-tooltip>`);
    const trigger = el.querySelector<HTMLButtonElement>("#b")!;
    const t = tip(el);
    expect(t.id).not.toBe("");
    expect(t.getAttribute("slot")).toBe("tip");
    expect(trigger.getAttribute("aria-describedby")!.split(" ")).toEqual(["otro", t.id]);
  });

  it("keeps the description in sync when the text changes or goes empty", async () => {
    const el = await mount(`<tx-tooltip text="uno"><button id="b">?</button></tx-tooltip>`);
    el.text = "dos";
    await el.updateComplete;
    expect(tip(el).textContent).toBe("dos");
    el.text = "";
    await el.updateComplete;
    expect(el.querySelector("#b")!.hasAttribute("aria-describedby")).toBe(false);
    el.dispatchEvent(new MouseEvent("mouseenter"));
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
  });

  it("describes triggers added later", async () => {
    const el = await mount(`<tx-tooltip text="ayuda"></tx-tooltip>`);
    const late = document.createElement("button");
    el.append(late);
    await vi.advanceTimersByTimeAsync(0);
    expect(late.getAttribute("aria-describedby")).toBe(tip(el).id);
  });

  it("waits for `delay` before showing and cancels when the pointer leaves early", async () => {
    const el = await mount(`<tx-tooltip text="x" delay="300"><span>a</span></tx-tooltip>`);
    el.dispatchEvent(new MouseEvent("mouseenter"));
    await vi.advanceTimersByTimeAsync(299);
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await el.updateComplete;
    expect(isOpen(el)).toBe(true);

    el.dispatchEvent(new MouseEvent("mouseleave"));
    await el.updateComplete;
    el.dispatchEvent(new MouseEvent("mouseenter"));
    el.dispatchEvent(new MouseEvent("mouseleave"));
    await vi.advanceTimersByTimeAsync(400);
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
  });

  it("Escape closes it", async () => {
    const el = await mount(`<tx-tooltip text="x"><span>a</span></tx-tooltip>`);
    el.dispatchEvent(new MouseEvent("mouseenter"));
    await el.updateComplete;
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
  });

  it("does not open while disabled", async () => {
    const el = await mount(`<tx-tooltip text="x" disabled><span>a</span></tx-tooltip>`);
    el.dispatchEvent(new MouseEvent("mouseenter"));
    await el.updateComplete;
    expect(isOpen(el)).toBe(false);
  });

  it("reflects the side on the bubble and defaults to top", async () => {
    const el = await mount(`<tx-tooltip text="x"><span>a</span></tx-tooltip>`);
    expect(bubble(el).getAttribute("data-side")).toBe("top");
    el.side = "bottom";
    await el.updateComplete;
    expect(bubble(el).getAttribute("data-side")).toBe("bottom");
  });

  it("positions the bubble from the trigger rectangle when it opens", async () => {
    const el = await mount(`<tx-tooltip text="x"><span>a</span></tx-tooltip>`);
    el.getBoundingClientRect = () => ({ left: 100, top: 50, right: 140, bottom: 70, width: 40, height: 20, x: 100, y: 50, toJSON() {} });
    el.dispatchEvent(new MouseEvent("mouseenter"));
    await el.updateComplete;
    expect(bubble(el).style.getPropertyValue("--tx-tip-x")).toBe("120px");
    expect(bubble(el).style.getPropertyValue("--tx-tip-y")).toBe("50px");
  });

  it("clears pending timers when disconnected", async () => {
    const el = await mount(`<tx-tooltip text="x" delay="100"><span>a</span></tx-tooltip>`);
    el.dispatchEvent(new MouseEvent("mouseenter"));
    el.remove();
    expect(vi.getTimerCount()).toBe(0);
  });
});
