import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-textarea";
import type { TxTextarea } from "../../src/components/tx-textarea";

afterEach(() => {
  document.body.innerHTML = "";
});

async function mount(props: Partial<TxTextarea> = {}) {
  const el = document.createElement("tx-textarea") as TxTextarea;
  Object.assign(el, props);
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const inner = (el: TxTextarea) => el.shadowRoot!.querySelector("textarea")!;

function type(el: TxTextarea, value: string) {
  const area = inner(el);
  area.value = value;
  area.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
}

/** happy-dom does no layout: fake the scroll height and the line metrics the autoresize reads. */
function fakeLayout(area: HTMLTextAreaElement, scrollHeight: number) {
  Object.defineProperty(area, "scrollHeight", { configurable: true, value: scrollHeight });
  vi.spyOn(window, "getComputedStyle").mockReturnValue({
    lineHeight: "20px",
    paddingTop: "6px",
    paddingBottom: "6px",
    borderTopWidth: "0px",
    borderBottomWidth: "0px",
  } as CSSStyleDeclaration);
}

describe("tx-textarea", () => {
  afterEach(() => vi.restoreAllMocks());

  it("renders value, placeholder and rows", async () => {
    const el = await mount({ value: "hola", placeholder: "Describe", rows: 4 });
    const area = inner(el);
    expect(area.value).toBe("hola");
    expect(area.placeholder).toBe("Describe");
    expect(area.getAttribute("rows")).toBe("4");
  });

  it("defaults to 3 rows", async () => {
    expect(inner(await mount()).getAttribute("rows")).toBe("3");
  });

  it("emits tx-input on every edit and tx-change on commit", async () => {
    const el = await mount();
    const input = vi.fn();
    const change = vi.fn();
    el.addEventListener("tx-input", input);
    el.addEventListener("tx-change", change);
    type(el, "a");
    type(el, "ab");
    expect(el.value).toBe("ab");
    expect(input.mock.calls.map((c) => (c[0] as CustomEvent).detail)).toEqual([{ value: "a" }, { value: "ab" }]);
    inner(el).dispatchEvent(new Event("change", { bubbles: true }));
    expect(change).toHaveBeenCalledTimes(1);
    expect((change.mock.calls[0]![0] as CustomEvent).detail).toEqual({ value: "ab" });
  });

  it("lets the host observe keydown and blur for commit-on-blur editors", async () => {
    const el = await mount();
    const keydown = vi.fn();
    const focusout = vi.fn();
    el.addEventListener("keydown", keydown);
    // Real browsers also retarget the non-bubbling composed `blur` to the host; happy-dom does not,
    // so the bubbling `focusout` stands in for it here.
    el.addEventListener("focusout", focusout);
    inner(el).dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", ctrlKey: true, bubbles: true, composed: true }));
    inner(el).dispatchEvent(new FocusEvent("focusout", { bubbles: true, composed: true }));
    expect(keydown).toHaveBeenCalledTimes(1);
    expect((keydown.mock.calls[0]![0] as KeyboardEvent).ctrlKey).toBe(true);
    expect(focusout).toHaveBeenCalledTimes(1);
  });

  it("grows with the content when autoresize is on, capped at maxrows", async () => {
    const el = await mount({ autoresize: true, rows: 1, maxrows: 3 });
    const area = inner(el);
    fakeLayout(area, 32); // one line of content: 20 + padding 12
    type(el, "una");
    expect(area.style.height).toBe("32px");
    Object.defineProperty(area, "scrollHeight", { configurable: true, value: 200 });
    type(el, "muchas\nlineas\nde\ntexto\nmas");
    expect(area.style.height).toBe("72px"); // 3 rows * 20 + 12
  });

  it("does not set an inline height without autoresize", async () => {
    const el = await mount();
    fakeLayout(inner(el), 100);
    type(el, "x");
    expect(inner(el).style.height).toBe("");
  });

  it("flags invalid state, label and description like the input", async () => {
    const el = await mount({ invalid: true, fieldLabel: "Descripción", fieldDescription: "Máx 500" });
    const area = inner(el);
    expect(area.getAttribute("aria-invalid")).toBe("true");
    expect(area.getAttribute("aria-label")).toBe("Descripción");
    expect(el.shadowRoot!.getElementById(area.getAttribute("aria-describedby")!)!.textContent).toBe("Máx 500");
  });

  it("disables the native textarea", async () => {
    expect(inner(await mount({ disabled: true })).disabled).toBe(true);
  });

  it("focus() focuses the inner textarea", async () => {
    const el = await mount();
    el.focus();
    expect(el.shadowRoot!.activeElement).toBe(inner(el));
  });
});
