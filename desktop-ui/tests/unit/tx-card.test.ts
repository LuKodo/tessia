import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-card";
import type { TxCard } from "../../src/components/tx-card";

async function mount(inner: string, attrs: Record<string, string> = {}) {
  const el = document.createElement("tx-card") as TxCard;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.innerHTML = inner;
  document.body.append(el);
  await el.updateComplete;
  return el;
}
const part = (el: TxCard, name: string) => el.shadowRoot!.querySelector(`[part~="${name}"]`);

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-card", () => {
  it("defaults to the subtle variant with medium padding", async () => {
    const el = await mount("<p>Hola</p>");
    expect(el.getAttribute("variant")).toBe("subtle");
    expect(el.getAttribute("padding")).toBe("md");
  });

  it("always renders the body and projects its default slot", async () => {
    const el = await mount("<p>Hola</p>");
    const slot = part(el, "body")!.querySelector("slot")!;
    expect(slot.assignedNodes()[0]?.textContent).toBe("Hola");
  });

  it("omits header and footer when nothing is slotted into them", async () => {
    const el = await mount("<p>Hola</p>");
    expect(part(el, "header")).toBeNull();
    expect(part(el, "footer")).toBeNull();
  });

  it("renders header and footer when slotted", async () => {
    const el = await mount(`<h3 slot="header">Título</h3><p>Hola</p><div slot="footer">Pie</div>`);
    const header = part(el, "header")!.querySelector("slot")!;
    const footer = part(el, "footer")!.querySelector("slot")!;
    expect(header.assignedNodes()[0]?.textContent).toBe("Título");
    expect(footer.assignedNodes()[0]?.textContent).toBe("Pie");
  });

  it("shows a header added after the first render", async () => {
    const el = await mount("<p>Hola</p>");
    const h = document.createElement("h3");
    h.slot = "header";
    h.textContent = "Nuevo";
    el.append(h);
    await new Promise((r) => setTimeout(r, 0));
    await el.updateComplete;
    expect(part(el, "header")).not.toBeNull();
  });

  it("reflects variant and padding changes", async () => {
    const el = await mount("<p>x</p>");
    el.variant = "outline";
    el.padding = "none";
    await el.updateComplete;
    expect(el.getAttribute("variant")).toBe("outline");
    expect(el.getAttribute("padding")).toBe("none");
  });
});
