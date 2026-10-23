import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-badge";
import type { TxBadge } from "../../src/components/tx-badge";

async function mount(attrs: Record<string, string> = {}, inner = "Listo") {
  const el = document.createElement("tx-badge") as TxBadge;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.innerHTML = inner;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-badge", () => {
  it("defaults to solid primary md and reflects the props", async () => {
    const el = await mount();
    expect(el.getAttribute("color")).toBe("primary");
    expect(el.getAttribute("variant")).toBe("solid");
    expect(el.getAttribute("size")).toBe("md");
    el.color = "warning";
    el.variant = "subtle";
    el.size = "sm";
    await el.updateComplete;
    expect(el.getAttribute("color")).toBe("warning");
    expect(el.getAttribute("variant")).toBe("subtle");
    expect(el.getAttribute("size")).toBe("sm");
  });

  it("projects the default slot", async () => {
    const el = await mount({}, "Pendiente");
    const slot = el.shadowRoot!.querySelector<HTMLSlotElement>("slot:not([name])")!;
    expect(slot.assignedNodes()[0]?.textContent).toBe("Pendiente");
  });

  it("renders leading and trailing icons and their slots", async () => {
    const el = await mount({ icon: "check", "trailing-icon": "xmark" });
    const icons = [...el.shadowRoot!.querySelectorAll("tx-icon")].map((i) => i.getAttribute("name"));
    expect(icons).toEqual(["check", "xmark"]);
    expect(el.shadowRoot!.querySelector('slot[name="leading"]')).not.toBeNull();
    expect(el.shadowRoot!.querySelector('slot[name="trailing"]')).not.toBeNull();
  });

  it("renders no icon when none is requested", async () => {
    const el = await mount();
    expect(el.shadowRoot!.querySelector("tx-icon")).toBeNull();
  });
});
