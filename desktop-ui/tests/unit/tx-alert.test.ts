import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-alert";
import type { TxAlert } from "../../src/components/tx-alert";

async function mount(attrs: Record<string, string> = {}, inner = "") {
  const el = document.createElement("tx-alert") as TxAlert;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  el.innerHTML = inner;
  document.body.append(el);
  await el.updateComplete;
  return el;
}
const q = (el: TxAlert, sel: string) => el.shadowRoot!.querySelector<HTMLElement>(sel);

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-alert", () => {
  it("defaults to subtle primary", async () => {
    const el = await mount({ heading: "Aviso" });
    expect(el.getAttribute("color")).toBe("primary");
    expect(el.getAttribute("variant")).toBe("subtle");
  });

  it("renders heading and description", async () => {
    const el = await mount({ heading: "Fallo", description: "No se pudo guardar" });
    expect(q(el, ".title")!.textContent).toBe("Fallo");
    expect(q(el, ".description")!.textContent).toBe("No se pudo guardar");
  });

  it("omits the empty text blocks", async () => {
    const el = await mount({ heading: "Solo título" });
    expect(q(el, ".description")).toBeNull();
    const el2 = await mount({ description: "Solo descripción" });
    expect(q(el2, ".title")).toBeNull();
  });

  it("renders the leading icon from the registry", async () => {
    const el = await mount({ heading: "x", icon: "check-circle" });
    expect(q(el, "tx-icon")!.getAttribute("name")).toBe("check-circle");
  });

  it("renders no icon by default", async () => {
    const el = await mount({ heading: "x" });
    expect(q(el, "tx-icon")).toBeNull();
  });

  it("uses role=alert for error/warning and role=status otherwise", async () => {
    expect((await mount({ color: "error", heading: "x" })).getAttribute("role")).toBe("alert");
    expect((await mount({ color: "warning", heading: "x" })).getAttribute("role")).toBe("alert");
    expect((await mount({ color: "info", heading: "x" })).getAttribute("role")).toBe("status");
  });

  it("keeps a role the consumer set", async () => {
    const el = await mount({ color: "info", role: "note", heading: "x" });
    expect(el.getAttribute("role")).toBe("note");
  });

  it("follows the color when the role was automatic", async () => {
    const el = await mount({ color: "info", heading: "x" });
    el.color = "error";
    await el.updateComplete;
    expect(el.getAttribute("role")).toBe("alert");
  });

  it("projects the default and actions slots", async () => {
    const el = await mount({ heading: "x" }, `<p>Detalle</p><button slot="actions">Reintentar</button>`);
    const def = q(el, ".content slot:not([name])") as HTMLSlotElement;
    const actions = q(el, 'slot[name="actions"]') as HTMLSlotElement;
    expect(def.assignedNodes()[0]?.textContent).toBe("Detalle");
    expect(actions.assignedNodes()[0]?.textContent).toBe("Reintentar");
  });

  it("has no close button unless closable", async () => {
    const el = await mount({ heading: "x" });
    expect(q(el, ".close")).toBeNull();
  });

  it("emits a close event from the labelled close button", async () => {
    const el = await mount({ heading: "x", closable: "" });
    const onClose = vi.fn();
    el.addEventListener("close", onClose);
    const btn = q(el, ".close")!;
    expect(btn.getAttribute("aria-label")).toBe("Cerrar");
    btn.click();
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
