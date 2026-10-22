import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-progress";
import type { TxProgress } from "../../src/components/tx-progress";

async function mount(attrs: Record<string, string> = {}) {
  const el = document.createElement("tx-progress") as TxProgress;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  document.body.append(el);
  await el.updateComplete;
  return el;
}
const bar = (el: TxProgress) => el.shadowRoot!.querySelector<HTMLElement>('[role="progressbar"]')!;
const fill = (el: TxProgress) => el.shadowRoot!.querySelector<HTMLElement>(".fill")!;

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-progress", () => {
  it("exposes an accessible progressbar with its value range", async () => {
    const el = await mount({ value: "42" });
    expect(bar(el).getAttribute("aria-valuemin")).toBe("0");
    expect(bar(el).getAttribute("aria-valuemax")).toBe("100");
    expect(bar(el).getAttribute("aria-valuenow")).toBe("42");
    expect(fill(el).style.width).toBe("42%");
  });

  it("scales by max", async () => {
    const el = await mount({ value: "50", max: "200" });
    expect(bar(el).getAttribute("aria-valuemax")).toBe("200");
    expect(fill(el).style.width).toBe("25%");
  });

  it("clamps out-of-range values", async () => {
    const el = await mount({ value: "150" });
    expect(fill(el).style.width).toBe("100%");
    el.value = -5;
    await el.updateComplete;
    expect(fill(el).style.width).toBe("0%");
    expect(bar(el).getAttribute("aria-valuenow")).toBe("0");
  });

  it("is indeterminate without a value and omits aria-valuenow", async () => {
    const el = await mount();
    expect(el.dataset.state).toBe("indeterminate");
    expect(bar(el).hasAttribute("aria-valuenow")).toBe(false);
    expect(fill(el).style.width).toBe("");
  });

  it("is indeterminate when the flag is set even with a value", async () => {
    const el = await mount({ value: "30", indeterminate: "" });
    expect(el.dataset.state).toBe("indeterminate");
    expect(bar(el).hasAttribute("aria-valuenow")).toBe(false);
  });

  it("reports complete at the maximum", async () => {
    const el = await mount({ value: "100" });
    expect(el.dataset.state).toBe("complete");
  });

  it("leaves indeterminate mode once a value arrives", async () => {
    const el = await mount();
    el.value = 10;
    await el.updateComplete;
    expect(el.dataset.state).toBe("loading");
    expect(bar(el).getAttribute("aria-valuenow")).toBe("10");
  });

  it("forwards label and value text to assistive tech", async () => {
    const el = await mount({ value: "3", max: "10", label: "Avance del cargue", "value-text": "3 de 10 pasos" });
    expect(bar(el).getAttribute("aria-label")).toBe("Avance del cargue");
    expect(bar(el).getAttribute("aria-valuetext")).toBe("3 de 10 pasos");
  });

  it("defaults to primary md and reflects color and size", async () => {
    const el = await mount({ value: "1" });
    expect(el.getAttribute("color")).toBe("primary");
    expect(el.getAttribute("size")).toBe("md");
    el.color = "success";
    el.size = "sm";
    await el.updateComplete;
    expect(el.getAttribute("color")).toBe("success");
    expect(el.getAttribute("size")).toBe("sm");
  });

  it("stops the indeterminate animation under reduced motion", () => {
    const css = (customElements.get("tx-progress") as unknown as { styles: { cssText: string } }).styles.cssText;
    expect(css).toMatch(/prefers-reduced-motion:\s*reduce/);
  });
});
