import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-file-upload";
import type { TxFileUpload } from "../../src/components/tx-file-upload";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(props: Partial<TxFileUpload> = {}) {
  const el = document.createElement("tx-file-upload") as TxFileUpload;
  Object.assign(el, props);
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const picker = (el: TxFileUpload) => el.shadowRoot!.querySelector<HTMLInputElement>('input[type="file"]')!;
const zone = (el: TxFileUpload) => el.shadowRoot!.querySelector<HTMLButtonElement>(".zone")!;
const names = (el: TxFileUpload) => [...el.shadowRoot!.querySelectorAll(".files .name")].map((n) => n.textContent!.trim());

const csv = (name = "convenio.csv", body = "a,b") => new File([body], name, { type: "text/csv" });

function pick(el: TxFileUpload, files: File[]) {
  Object.defineProperty(picker(el), "files", { configurable: true, value: files });
  picker(el).dispatchEvent(new Event("change", { bubbles: true }));
}

function dropEvent(type: string, files: File[] = []) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "dataTransfer", { value: { files, dropEffect: "none" } });
  return event;
}

describe("tx-file-upload", () => {
  it("renders a drop zone with the default Spanish prompt", async () => {
    const el = await mount();
    expect(zone(el).textContent).toContain("Arrastra un archivo o haz clic");
    const many = await mount({ multiple: true });
    expect(zone(many).textContent).toContain("Arrastra archivos o haz clic");
  });

  it("uses label and description props, or the slots", async () => {
    const el = await mount({ label: "CSV de convenio", description: "Elegir archivo (.csv)" });
    expect(zone(el).textContent).toContain("CSV de convenio");
    expect(zone(el).textContent).toContain("Elegir archivo (.csv)");
    expect(el.shadowRoot!.querySelector('slot[name="label"]')).not.toBeNull();
    expect(el.shadowRoot!.querySelector('slot[name="description"]')).not.toBeNull();
  });

  it("passes accept and multiple to the native picker", async () => {
    const el = await mount({ accept: ".csv", multiple: true });
    expect(picker(el).getAttribute("accept")).toBe(".csv");
    expect(picker(el).multiple).toBe(true);
  });

  it("opens the native picker when the zone is clicked", async () => {
    const el = await mount();
    const click = vi.spyOn(picker(el), "click");
    zone(el).click();
    expect(click).toHaveBeenCalledTimes(1);
  });

  it("emits tx-change with the picked files and lists them", async () => {
    const el = await mount();
    const handler = vi.fn();
    document.body.addEventListener("tx-change", handler);
    const file = csv();
    pick(el, [file]);
    await el.updateComplete;
    const event = handler.mock.calls[0]![0] as CustomEvent;
    expect(event.detail).toEqual({ files: [file] });
    expect(event.composed).toBe(true);
    expect(el.files).toEqual([file]);
    expect(names(el)).toEqual(["convenio.csv"]);
  });

  it("keeps only the first file unless multiple", async () => {
    const el = await mount();
    pick(el, [csv("a.csv"), csv("b.csv")]);
    await el.updateComplete;
    expect(names(el)).toEqual(["a.csv"]);
    const many = await mount({ multiple: true });
    pick(many, [csv("a.csv"), csv("b.csv")]);
    await many.updateComplete;
    expect(names(many)).toEqual(["a.csv", "b.csv"]);
  });

  it("removes a file from the list and announces the new selection", async () => {
    const el = await mount({ multiple: true });
    pick(el, [csv("a.csv"), csv("b.csv")]);
    await el.updateComplete;
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    const remove = el.shadowRoot!.querySelector<HTMLButtonElement>(".files li button")!;
    expect(remove.getAttribute("aria-label")).toBe("Quitar a.csv");
    remove.click();
    await el.updateComplete;
    expect(names(el)).toEqual(["b.csv"]);
    expect((handler.mock.calls[0]![0] as CustomEvent).detail.files.map((f: File) => f.name)).toEqual(["b.csv"]);
  });

  it("clears the list when the page resets files", async () => {
    const el = await mount();
    pick(el, [csv()]);
    await el.updateComplete;
    el.files = [];
    await el.updateComplete;
    expect(names(el)).toEqual([]);
  });

  it("accepts dropped files, marking the zone while dragging", async () => {
    const el = await mount();
    const over = dropEvent("dragover");
    zone(el).dispatchEvent(over);
    await el.updateComplete;
    expect(over.defaultPrevented).toBe(true);
    expect(zone(el).classList.contains("dragging")).toBe(true);
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    const drop = dropEvent("drop", [csv("dropped.csv")]);
    zone(el).dispatchEvent(drop);
    await el.updateComplete;
    expect(drop.defaultPrevented).toBe(true);
    expect(zone(el).classList.contains("dragging")).toBe(false);
    expect(names(el)).toEqual(["dropped.csv"]);
    expect(handler).toHaveBeenCalledTimes(1);
  });

  it("clears the dragging mark when the drag leaves", async () => {
    const el = await mount();
    zone(el).dispatchEvent(dropEvent("dragenter"));
    await el.updateComplete;
    zone(el).dispatchEvent(dropEvent("dragleave"));
    await el.updateComplete;
    expect(zone(el).classList.contains("dragging")).toBe(false);
  });

  it("drops only files that match accept", async () => {
    const el = await mount({ accept: ".csv,image/*" });
    zone(el).dispatchEvent(dropEvent("drop", [csv("a.CSV"), new File(["x"], "b.xlsx"), new File(["x"], "c.png", { type: "image/png" })]));
    await el.updateComplete;
    expect(names(el)).toEqual(["a.CSV"]);
    const many = await mount({ accept: ".csv,image/*", multiple: true });
    zone(many).dispatchEvent(dropEvent("drop", [csv("a.CSV"), new File(["x"], "b.xlsx"), new File(["x"], "c.png", { type: "image/png" })]));
    await many.updateComplete;
    expect(names(many)).toEqual(["a.CSV", "c.png"]);
  });

  it("ignores everything while disabled", async () => {
    const el = await mount({ disabled: true });
    expect(zone(el).disabled).toBe(true);
    const handler = vi.fn();
    el.addEventListener("tx-change", handler);
    zone(el).dispatchEvent(dropEvent("drop", [csv()]));
    expect(handler).not.toHaveBeenCalled();
    expect(picker(el).disabled).toBe(true);
  });

  it("supports a compact button variant", async () => {
    const el = await mount({ variant: "button" });
    expect(el.getAttribute("variant")).toBe("button");
    expect(zone(el).classList.contains("button")).toBe(true);
    expect(zone(el).textContent).toContain("Elegir archivo");
  });

  it("formats file sizes", async () => {
    const el = await mount();
    pick(el, [csv("a.csv", "x".repeat(1536))]);
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector(".files .size")!.textContent!.trim()).toBe("1.5 KB");
  });

  it("names the group with the field label and flags invalid state", async () => {
    const el = await mount({ fieldLabel: "Archivo", invalid: true });
    expect(el.shadowRoot!.querySelector('[role="group"]')!.getAttribute("aria-label")).toBe("Archivo");
    expect(zone(el).getAttribute("aria-invalid")).toBeNull();
    expect(zone(el).classList.contains("invalid")).toBe(true);
  });
});
