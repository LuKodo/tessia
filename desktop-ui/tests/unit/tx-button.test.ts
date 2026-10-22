import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-button";
import { TxButton } from "../../src/components/tx-button";

async function mount(attrs: Record<string, string> = {}, inner = "Guardar", parent: ParentNode = document.body) {
  const el = document.createElement("tx-button") as TxButton;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.innerHTML = inner;
  parent.append(el);
  await el.updateComplete;
  return el;
}
const control = (el: TxButton) => el.shadowRoot!.querySelector<HTMLElement>(".control")!;

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-button", () => {
  it("is registered once and defaults to solid primary md", async () => {
    expect(customElements.get("tx-button")).toBeDefined();
    const el = await mount();
    expect(el.color).toBe("primary");
    expect(el.variant).toBe("solid");
    expect(el.size).toBe("md");
    expect(el.getAttribute("variant")).toBe("solid");
    expect(control(el).tagName).toBe("BUTTON");
    expect(control(el).getAttribute("type")).toBe("button");
  });

  it("reflects color, variant and size to attributes for styling", async () => {
    const el = await mount();
    el.color = "error";
    el.variant = "outline";
    el.size = "sm";
    await el.updateComplete;
    expect(el.getAttribute("color")).toBe("error");
    expect(el.getAttribute("variant")).toBe("outline");
    expect(el.getAttribute("size")).toBe("sm");
  });

  it("slots the default content and the leading/trailing slots", async () => {
    const el = await mount({}, `<span slot="leading">L</span>Texto<span slot="trailing">T</span>`);
    const slots = [...el.shadowRoot!.querySelectorAll("slot")].map((s) => s.getAttribute("name"));
    expect(slots).toEqual(["leading", null, "trailing"]);
  });

  it("renders a leading and a trailing icon from the registry", async () => {
    const el = await mount({ icon: "download", "trailing-icon": "nav-arrow-right" });
    const icons = [...el.shadowRoot!.querySelectorAll("tx-icon")].map((i) => i.getAttribute("name"));
    expect(icons).toEqual(["download", "nav-arrow-right"]);
  });

  it("lets a click through as the native click", async () => {
    const el = await mount();
    const onClick = vi.fn();
    el.addEventListener("click", onClick);
    control(el).click();
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("swallows clicks while disabled and disables the inner control", async () => {
    const el = await mount({ disabled: "" });
    const onClick = vi.fn();
    el.addEventListener("click", onClick);
    control(el).click();
    el.click();
    expect(onClick).not.toHaveBeenCalled();
    expect(control(el).hasAttribute("disabled")).toBe(true);
  });

  it("is busy and non-clickable while loading, and shows a spinner", async () => {
    const el = await mount({ loading: "" });
    const onClick = vi.fn();
    el.addEventListener("click", onClick);
    el.click();
    expect(onClick).not.toHaveBeenCalled();
    expect(control(el).getAttribute("aria-busy")).toBe("true");
    expect(control(el).hasAttribute("disabled")).toBe(true);
    expect(el.shadowRoot!.querySelector(".spinner")).not.toBeNull();
  });

  it("renders an anchor when href is set and drops the href while disabled", async () => {
    const el = await mount({ href: "/ejecuciones" });
    expect(control(el).tagName).toBe("A");
    expect(control(el).getAttribute("href")).toBe("/ejecuciones");
    el.disabled = true;
    await el.updateComplete;
    expect(control(el).hasAttribute("href")).toBe(false);
    expect(control(el).getAttribute("aria-disabled")).toBe("true");
  });

  it("adds rel=noopener for target=_blank anchors", async () => {
    const el = await mount({ href: "https://example.org", target: "_blank" });
    expect(control(el).getAttribute("rel")).toContain("noopener");
  });

  it("forwards aria-label to the inner control for icon-only buttons", async () => {
    const el = await mount({ icon: "xmark", "aria-label": "Cerrar" }, "");
    expect(control(el).getAttribute("aria-label")).toBe("Cerrar");
  });

  it("submits the enclosing form when type=submit", async () => {
    const form = document.createElement("form");
    document.body.append(form);
    const onSubmit = vi.fn((e: Event) => e.preventDefault());
    form.addEventListener("submit", onSubmit);
    const el = await mount({ type: "submit" }, "Guardar", form);
    control(el).click();
    await Promise.resolve();
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it("does not submit when the consumer prevents the click", async () => {
    const form = document.createElement("form");
    document.body.append(form);
    const onSubmit = vi.fn((e: Event) => e.preventDefault());
    form.addEventListener("submit", onSubmit);
    const el = await mount({ type: "submit" }, "Guardar", form);
    el.addEventListener("click", (e) => e.preventDefault());
    control(el).click();
    await Promise.resolve();
    expect(onSubmit).not.toHaveBeenCalled();
  });

  it("delegates focus to the inner control (happy-dom does not emulate it, so the option is checked)", async () => {
    const el = await mount();
    expect((TxButton as unknown as { shadowRootOptions: ShadowRootInit }).shadowRootOptions.delegatesFocus).toBe(true);
    control(el).focus();
    expect(el.shadowRoot!.activeElement).toBe(control(el));
  });
});
