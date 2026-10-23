import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-icon";
import type { TxIcon } from "../../src/components/tx-icon";
import { ICONS } from "../../src/components/icons/icons";

async function mount(attrs: Record<string, string>) {
  const el = document.createElement("tx-icon") as TxIcon;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  document.body.append(el);
  await el.updateComplete;
  return el;
}
const svg = (el: TxIcon) => el.shadowRoot!.querySelector("svg");

beforeEach(() => {
  vi.spyOn(console, "warn").mockImplementation(() => {});
});
afterEach(() => {
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("icon registry", () => {
  it("holds the 45 iconoir icons the old app used", () => {
    expect(Object.keys(ICONS)).toHaveLength(45);
    for (const name of ["check-circle", "warning-triangle", "nav-arrow-down", "xmark", "wifi-xmark", "square"]) {
      expect(ICONS[name], name).toBeTruthy();
    }
  });

  it("stores trusted SVG body markup only", () => {
    for (const [name, body] of Object.entries(ICONS)) {
      expect(body, name).toMatch(/^<(path|g)\b/);
      expect(body, name).not.toMatch(/<script|on\w+=/i);
    }
  });
});

describe("tx-icon", () => {
  it("renders the named icon as an inline 24x24 svg", async () => {
    const el = await mount({ name: "check" });
    const s = svg(el)!;
    expect(s.getAttribute("viewBox")).toBe("0 0 24 24");
    expect(s.querySelector("path")).not.toBeNull();
    expect(s.getAttribute("aria-hidden")).toBe("true");
  });

  it("renders nothing and warns once for an unknown name", async () => {
    const warn = vi.mocked(console.warn);
    const first = await mount({ name: "does-not-exist" });
    const second = await mount({ name: "does-not-exist" });
    expect(svg(first)).toBeNull();
    expect(svg(second)).toBeNull();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(String(warn.mock.calls[0]?.[0])).toContain("does-not-exist");
  });

  it("re-renders when the name changes", async () => {
    const el = await mount({ name: "check" });
    const before = svg(el)!.innerHTML;
    el.name = "xmark";
    await el.updateComplete;
    expect(svg(el)!.innerHTML).not.toBe(before);
  });

  it("is a labelled image when a label is given", async () => {
    const el = await mount({ name: "info-circle", label: "Información" });
    expect(el.getAttribute("role")).toBe("img");
    expect(el.getAttribute("aria-label")).toBe("Información");
    expect(svg(el)!.getAttribute("aria-hidden")).toBe("true");
  });

  it("sizes by token, number of pixels or CSS length", async () => {
    const el = await mount({ name: "check", size: "lg" });
    expect(el.style.getPropertyValue("--tx-icon-size")).toBe("1.5rem");
    el.size = "20";
    await el.updateComplete;
    expect(el.style.getPropertyValue("--tx-icon-size")).toBe("20px");
    el.size = "2rem";
    await el.updateComplete;
    expect(el.style.getPropertyValue("--tx-icon-size")).toBe("2rem");
  });
});
