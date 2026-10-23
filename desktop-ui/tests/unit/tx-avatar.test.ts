import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-avatar";
import type { TxAvatar } from "../../src/components/tx-avatar";

async function mount(attrs: Record<string, string> = {}) {
  const el = document.createElement("tx-avatar") as TxAvatar;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  document.body.append(el);
  await el.updateComplete;
  return el;
}
const q = (el: TxAvatar, sel: string) => el.shadowRoot!.querySelector<HTMLElement>(sel);

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-avatar", () => {
  it("defaults to neutral md", async () => {
    const el = await mount({ text: "A" });
    expect(el.getAttribute("color")).toBe("neutral");
    expect(el.getAttribute("size")).toBe("md");
  });

  it("renders an image with its alt text", async () => {
    const el = await mount({ src: "/u.png", alt: "Ana Pérez" });
    const img = q(el, "img")!;
    expect(img.getAttribute("src")).toBe("/u.png");
    expect(img.getAttribute("alt")).toBe("");
    expect(el.getAttribute("role")).toBe("img");
    expect(el.getAttribute("aria-label")).toBe("Ana Pérez");
  });

  it("falls back to the icon when the image fails", async () => {
    const el = await mount({ src: "/broken.png", alt: "Ana", icon: "page" });
    q(el, "img")!.dispatchEvent(new Event("error"));
    await el.updateComplete;
    expect(q(el, "img")).toBeNull();
    expect(q(el, "tx-icon")!.getAttribute("name")).toBe("page");
  });

  it("renders an icon as a decorative avatar", async () => {
    const el = await mount({ icon: "check-circle" });
    expect(q(el, "tx-icon")!.getAttribute("name")).toBe("check-circle");
    expect(el.hasAttribute("role")).toBe(false);
    expect(el.getAttribute("aria-hidden")).toBe("true");
  });

  it("renders the given text", async () => {
    const el = await mount({ text: "+3" });
    expect(q(el, ".text")!.textContent).toBe("+3");
  });

  it("derives up to two initials from alt when there is no image, icon or text", async () => {
    const el = await mount({ alt: "maría josé pérez" });
    expect(q(el, ".text")!.textContent).toBe("MJ");
    expect(el.getAttribute("aria-label")).toBe("maría josé pérez");
  });

  it("renders nothing visible without any source", async () => {
    const el = await mount();
    expect(q(el, "img")).toBeNull();
    expect(q(el, "tx-icon")).toBeNull();
    expect(q(el, ".text")).toBeNull();
  });

  it("retries the image when src changes after a failure", async () => {
    const el = await mount({ src: "/broken.png", alt: "Ana" });
    q(el, "img")!.dispatchEvent(new Event("error"));
    await el.updateComplete;
    el.src = "/ok.png";
    await el.updateComplete;
    expect(q(el, "img")!.getAttribute("src")).toBe("/ok.png");
  });
});
