import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-skeleton";
import type { TxSkeleton } from "../../src/components/tx-skeleton";

async function mount(attrs: Record<string, string> = {}) {
  const el = document.createElement("tx-skeleton") as TxSkeleton;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  document.body.append(el);
  await el.updateComplete;
  return el;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-skeleton", () => {
  it("is a decorative placeholder hidden from assistive tech", async () => {
    const el = await mount();
    expect(el.getAttribute("aria-hidden")).toBe("true");
  });

  it("keeps a consumer-provided aria-hidden untouched", async () => {
    const el = await mount({ "aria-hidden": "false" });
    expect(el.getAttribute("aria-hidden")).toBe("false");
  });

  it("reflects the circle flag for round avatars", async () => {
    const el = await mount({ circle: "" });
    expect(el.circle).toBe(true);
    expect(el.hasAttribute("circle")).toBe(true);
  });

  it("styles itself as a pulsing block that stops under reduced motion", () => {
    const css = String((customElements.get("tx-skeleton") as unknown as { styles: { cssText: string } }).styles.cssText);
    expect(css).toContain("animation");
    expect(css).toMatch(/prefers-reduced-motion:\s*reduce/);
  });
});
