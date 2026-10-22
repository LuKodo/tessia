/** Every element matching `selector` under `root`, descending into the shadow roots of nested elements. */
export function deepAll<T extends Element = HTMLElement>(root: Element | ShadowRoot, selector: string): T[] {
  const start: ParentNode = root instanceof Element ? (root.shadowRoot ?? root) : root;
  const found: T[] = [];
  const walk = (node: ParentNode) => {
    found.push(...node.querySelectorAll<T>(selector));
    for (const el of node.querySelectorAll("*")) if (el.shadowRoot) walk(el.shadowRoot);
  };
  walk(start);
  return found;
}

export function deep<T extends Element = HTMLElement>(root: Element | ShadowRoot, selector: string): T {
  const el = deepAll<T>(root, selector)[0];
  if (!el) throw new Error(`no element matches ${selector}`);
  return el;
}

/** Native control inside the shadow root of a `tx-input` / `tx-checkbox` / `tx-switch` / `tx-select`. */
const inner = (host: Element, selector: string) => host.shadowRoot!.querySelector<HTMLInputElement & HTMLSelectElement>(selector)!;

export function typeInto(host: Element, value: string) {
  const input = inner(host, "input");
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true, composed: true }));
}

export function setChecked(host: Element, checked: boolean) {
  const input = inner(host, "input");
  input.checked = checked;
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

/** Toggles a checkbox or switch the way a click on its control would. */
export function toggle(host: Element) {
  setChecked(host, !inner(host, "input").checked);
}

export function choose(host: Element, value: string) {
  const select = inner(host, "select");
  select.value = value;
  select.dispatchEvent(new Event("change", { bubbles: true }));
}

/** Lets pending promise chains (fake fetches, retry polling) finish, then waits for the element to render. */
export async function settle(el: { updateComplete: Promise<unknown> }) {
  for (let i = 0; i < 3; i++) {
    await new Promise((resolve) => setTimeout(resolve, 0));
    await el.updateComplete;
  }
}
