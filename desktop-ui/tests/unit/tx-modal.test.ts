import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-modal";
import type { TxModal } from "../../src/components/tx-modal";

async function mount(markup: string): Promise<TxModal> {
  const host = document.createElement("div");
  host.innerHTML = markup;
  document.body.append(host);
  const el = host.querySelector("tx-modal") as TxModal;
  await el.updateComplete;
  return el;
}

const dialog = (el: TxModal) => el.shadowRoot!.querySelector("dialog")!;
const closeButton = (el: TxModal) => el.shadowRoot!.querySelector<HTMLButtonElement>("button.close");

afterEach(() => {
  document.body.innerHTML = "";
  document.documentElement.style.overflow = "";
});

describe("tx-modal", () => {
  it("is registered once and exports its class", () => {
    expect(customElements.get("tx-modal")).toBeDefined();
  });

  it("stays closed until `open` is set and then opens the native dialog as modal", async () => {
    const el = await mount(`<tx-modal title="Hola"><p>cuerpo</p></tx-modal>`);
    expect(dialog(el).open).toBe(false);
    el.open = true;
    await el.updateComplete;
    expect(dialog(el).open).toBe(true);
    expect(dialog(el).getAttribute("aria-modal")).toBe("true");
    el.open = false;
    await el.updateComplete;
    expect(dialog(el).open).toBe(false);
  });

  it("opens straight away when the attribute is present at first render", async () => {
    const el = await mount(`<tx-modal open title="Hola"></tx-modal>`);
    expect(dialog(el).open).toBe(true);
  });

  it("labels the dialog with its title and describes it with its description", async () => {
    const el = await mount(`<tx-modal open title="Resultado" description="FE-1001"></tx-modal>`);
    const d = dialog(el);
    const title = el.shadowRoot!.getElementById(d.getAttribute("aria-labelledby")!)!;
    const description = el.shadowRoot!.getElementById(d.getAttribute("aria-describedby")!)!;
    expect(title.textContent!.trim()).toBe("Resultado");
    expect(description.textContent!.trim()).toBe("FE-1001");
    expect(d.getAttribute("role")).toBeNull(); // <dialog> already has the dialog role
  });

  it("renders the title and description slots in place of the props", async () => {
    const el = await mount(`<tx-modal open title="ignored"><span slot="title">Factura</span><span slot="description">detalle</span></tx-modal>`);
    const header = el.shadowRoot!.querySelector("header")!;
    const slots = [...header.querySelectorAll("slot")].map((s) => s.getAttribute("name"));
    expect(slots).toContain("title");
    expect(slots).toContain("description");
  });

  it("projects the body (default and `body` slot), `actions` and `footer`", async () => {
    const el = await mount(
      `<tx-modal open title="T"><span slot="actions">acc</span><p id="b">cuerpo</p><button slot="footer" id="f">Cerrar</button></tx-modal>`,
    );
    const root = el.shadowRoot!;
    expect(root.querySelector("slot[name=body] > slot:not([name])")).not.toBeNull();
    expect(root.querySelector("slot[name=actions]")).not.toBeNull();
    expect(root.querySelector("footer slot[name=footer]")).not.toBeNull();
  });

  it("omits the footer when there is nothing slotted into it", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    expect(el.shadowRoot!.querySelector("footer")).toBeNull();
  });

  it("renders the footer once something is slotted into it later", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    const button = document.createElement("button");
    button.slot = "footer";
    el.append(button);
    await new Promise((r) => setTimeout(r, 0));
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("footer slot[name=footer]")).not.toBeNull();
  });

  it("clicking the close button asks to close with tx-open-change and tx-close", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    const seen: string[] = [];
    el.addEventListener("tx-open-change", (e) => seen.push(`open-change:${(e as CustomEvent).detail.open}`));
    el.addEventListener("tx-close", () => seen.push("close"));
    closeButton(el)!.click();
    await el.updateComplete;
    expect(seen).toEqual(["open-change:false", "close"]);
    expect(el.open).toBe(false);
    expect(dialog(el).open).toBe(false);
  });

  it("labels the close button in Spanish", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    expect(closeButton(el)!.getAttribute("aria-label")).toBe("Cerrar");
  });

  it("events bubble out of nested shadow roots (composed)", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    let heard = false;
    document.body.addEventListener("tx-open-change", () => (heard = true), { once: true });
    closeButton(el)!.click();
    expect(heard).toBe(true);
  });

  it("Escape (the native cancel event) is intercepted and requests a close", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    const cancel = new Event("cancel", { cancelable: true });
    dialog(el).dispatchEvent(cancel);
    await el.updateComplete;
    expect(cancel.defaultPrevented).toBe(true); // the element, not the browser, closes the dialog
    expect(el.open).toBe(false);
  });

  it("a click on the backdrop closes, a click inside the panel does not", async () => {
    const el = await mount(`<tx-modal open title="T"><p>cuerpo</p></tx-modal>`);
    el.shadowRoot!.querySelector<HTMLElement>(".panel")!.click();
    await el.updateComplete;
    expect(el.open).toBe(true);
    dialog(el).click();
    await el.updateComplete;
    expect(el.open).toBe(false);
  });

  it("the parent can veto a close with preventDefault", async () => {
    const el = await mount(`<tx-modal open title="T"></tx-modal>`);
    el.addEventListener("tx-open-change", (e) => e.preventDefault());
    closeButton(el)!.click();
    await el.updateComplete;
    expect(el.open).toBe(true);
    expect(dialog(el).open).toBe(true);
  });

  it("a non-dismissible modal ignores Escape and backdrop clicks and has no close button", async () => {
    const el = await mount(`<tx-modal open title="T" dismissible="false"></tx-modal>`);
    expect(el.dismissible).toBe(false);
    expect(closeButton(el)).toBeNull();
    const cancel = new Event("cancel", { cancelable: true });
    dialog(el).dispatchEvent(cancel);
    dialog(el).click();
    await el.updateComplete;
    expect(cancel.defaultPrevented).toBe(true);
    expect(el.open).toBe(true);
  });

  it("restores focus to the element that had it before opening", async () => {
    const host = document.createElement("div");
    host.innerHTML = `<button id="opener">abrir</button><tx-modal title="T"></tx-modal>`;
    document.body.append(host);
    const opener = host.querySelector<HTMLButtonElement>("#opener")!;
    const el = host.querySelector("tx-modal") as TxModal;
    await el.updateComplete;
    opener.focus();
    expect(document.activeElement).toBe(opener);
    el.open = true;
    await el.updateComplete;
    el.shadowRoot!.querySelector<HTMLButtonElement>("button.close")!.focus();
    expect(document.activeElement).not.toBe(opener);
    el.open = false;
    await el.updateComplete;
    expect(document.activeElement).toBe(opener);
  });

  it("moves focus to a slotted [autofocus] element on open", async () => {
    const el = await mount(`<tx-modal title="T"><button id="go" autofocus>Ir</button></tx-modal>`);
    el.open = true;
    await el.updateComplete;
    expect(document.activeElement!.id).toBe("go");
  });

  it("sizes the panel from `size`, and `width` wins over it", async () => {
    const el = await mount(`<tx-modal open title="T" size="xl"></tx-modal>`);
    expect(dialog(el).style.getPropertyValue("--tx-modal-width")).toBe("56rem");
    el.width = "30rem";
    await el.updateComplete;
    expect(dialog(el).style.getPropertyValue("--tx-modal-width")).toBe("30rem");
  });

  it("bare mode renders only the content: no header, close button or footer", async () => {
    const el = await mount(`<tx-modal open bare label="Confirmar"><p>x</p></tx-modal>`);
    expect(el.shadowRoot!.querySelector("header")).toBeNull();
    expect(closeButton(el)).toBeNull();
    expect(dialog(el).getAttribute("aria-label")).toBe("Confirmar");
    expect(dialog(el).hasAttribute("aria-labelledby")).toBe(false);
  });

  it("locks page scroll while open and releases it on close and on disconnect", async () => {
    const el = await mount(`<tx-modal title="T"></tx-modal>`);
    el.open = true;
    await el.updateComplete;
    expect(document.documentElement.style.overflow).toBe("hidden");
    el.open = false;
    await el.updateComplete;
    expect(document.documentElement.style.overflow).toBe("");
    el.open = true;
    await el.updateComplete;
    el.remove();
    expect(document.documentElement.style.overflow).toBe("");
  });

  it("keeps the page locked until the last of two stacked modals closes", async () => {
    const a = await mount(`<tx-modal title="A"></tx-modal>`);
    const b = await mount(`<tx-modal title="B"></tx-modal>`);
    a.open = true;
    b.open = true;
    await Promise.all([a.updateComplete, b.updateComplete]);
    a.open = false;
    await a.updateComplete;
    expect(document.documentElement.style.overflow).toBe("hidden");
    b.open = false;
    await b.updateComplete;
    expect(document.documentElement.style.overflow).toBe("");
  });
});
