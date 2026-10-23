import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-separator";
import type { TxSeparator } from "../../src/components/tx-separator";

async function mount(attrs: Record<string, string> = {}, inner = "") {
  const el = document.createElement("tx-separator") as TxSeparator;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.innerHTML = inner;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-separator", () => {
  it("is a horizontal separator by default", async () => {
    const el = await mount();
    expect(el.getAttribute("role")).toBe("separator");
    expect(el.getAttribute("orientation")).toBe("horizontal");
    expect(el.hasAttribute("aria-orientation")).toBe(false);
  });

  it("announces the vertical orientation", async () => {
    const el = await mount({ orientation: "vertical" });
    expect(el.getAttribute("aria-orientation")).toBe("vertical");
  });

  it("follows orientation changes", async () => {
    const el = await mount({ orientation: "vertical" });
    el.orientation = "horizontal";
    await el.updateComplete;
    expect(el.hasAttribute("aria-orientation")).toBe(false);
  });

  it("renders a label between the two lines", async () => {
    const el = await mount({ label: "o subir un CSV manual" });
    expect(el.shadowRoot!.querySelector(".label")!.textContent).toBe("o subir un CSV manual");
    expect(el.shadowRoot!.querySelectorAll(".line")).toHaveLength(2);
  });

  it("renders slotted content in place of the label", async () => {
    const el = await mount({}, `<b>O</b>`);
    const slot = el.shadowRoot!.querySelector<HTMLSlotElement>(".label slot")!;
    expect(slot.assignedNodes()[0]?.textContent).toBe("O");
  });

  it("draws a single line without label or content", async () => {
    const el = await mount();
    expect(el.shadowRoot!.querySelectorAll(".line")).toHaveLength(1);
    expect(el.shadowRoot!.querySelector(".label")).toBeNull();
  });

  it("can be purely decorative", async () => {
    const el = await mount({ decorative: "" });
    expect(el.getAttribute("role")).toBe("none");
  });

  it("keeps a role the consumer set", async () => {
    const el = await mount({ role: "presentation" });
    expect(el.getAttribute("role")).toBe("presentation");
  });
});
