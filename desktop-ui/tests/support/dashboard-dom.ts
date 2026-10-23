import { vi } from "vitest";

/** Shared DOM helpers of the dashboard element tests. */

/** Creates `tag`, assigns `props`, attaches it to the body and waits for its first render. */
export async function mountEl<T extends HTMLElement & { updateComplete: Promise<boolean> }>(tag: string, props: Partial<T> | Record<string, unknown> = {}): Promise<T> {
  const el = document.createElement(tag) as T;
  Object.assign(el, props);
  document.body.append(el);
  await el.updateComplete;
  return el;
}

/** Queries the shadow root of `el` first, then its light DOM (slotted children of a primitive). */
export const q = <E extends Element = HTMLElement>(el: Element, selector: string): E | null =>
  el.shadowRoot?.querySelector<E>(selector) ?? el.querySelector<E>(selector);
export const qa = <E extends Element = HTMLElement>(el: Element, selector: string): E[] => [
  ...(el.shadowRoot?.querySelectorAll<E>(selector) ?? el.querySelectorAll<E>(selector)),
];
export const text = (el: Element): string => (el.shadowRoot?.textContent ?? el.textContent ?? "").replace(/\s+/g, " ").trim();

/** Lets pending promises and one Lit update cycle settle. */
export const settle = async (el?: { updateComplete: Promise<boolean> }) => {
  for (let i = 0; i < 4; i++) {
    await Promise.resolve();
    if (vi.isFakeTimers()) await vi.advanceTimersByTimeAsync(0);
    else await new Promise((r) => setTimeout(r, 0));
  }
  await el?.updateComplete;
};

export function click(target: Element | null): void {
  target?.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true, cancelable: true }));
}

export function key(target: Element | null, k: string): void {
  target?.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, composed: true, cancelable: true }));
}

/** Text of `node` including the shadow roots of nested custom elements (whitespace collapsed). */
export function deepText(node: Node): string {
  const parts: string[] = [];
  const walk = (n: Node) => {
    if (n.nodeType === Node.TEXT_NODE) parts.push(n.textContent ?? "");
    const el = n as Element;
    if (el.shadowRoot) el.shadowRoot.childNodes.forEach(walk);
    n.childNodes.forEach(walk);
  };
  walk(node);
  return parts.join(" ").replace(/\s+/g, " ").trim();
}
