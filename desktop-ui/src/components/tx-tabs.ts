import { LitElement, css, html, nothing } from "lit";
import { icon } from "./shell-icons";

export interface TabItem {
  label: string;
  value: string;
  /** Name from the icon set in `icons.ts`. */
  icon?: string;
  disabled?: boolean;
}

const COLORS: Record<string, string> = {
  primary: "var(--tx-primary)",
  neutral: "var(--tx-text-highlighted)",
  success: "var(--tx-success)",
  warning: "var(--tx-warning)",
  error: "var(--tx-error)",
  info: "var(--tx-info)",
};
const FONT: Record<string, string> = {
  xs: "0.75rem",
  sm: "var(--tx-text-sm)",
  md: "var(--tx-text-sm)",
  lg: "var(--tx-text-base)",
  xl: "var(--tx-text-lg)",
};

let uid = 0;

/**
 * Tabs with the ARIA tablist pattern (automatic activation, roving tabindex).
 *
 * Properties: `items` (`{label, value, icon?, disabled?}[]`, set from JS), `value` (selected item,
 * the first enabled one by default), `variant` (link|pill, default link), `color`
 * (primary|neutral|success|warning|error|info, default primary), `size` (xs..xl, default md).
 *
 * Panels: put the content of each tab in a light-DOM child with `slot="<item value>"`, e.g.
 * `<tx-tabs><div slot="operate">...</div></tx-tabs>`. Every panel is rendered (hidden unless
 * active), so slotted content keeps its state while its tab is not selected.
 *
 * Events (bubbling, composed): `tx-change` {value} when the selection changes by click or key.
 * Keys: ArrowLeft/ArrowRight (wrapping), Home, End; disabled tabs are skipped.
 */
export class TxTabs extends LitElement {
  static properties = {
    items: { attribute: false },
    value: { type: String, reflect: true },
    variant: { type: String, reflect: true },
    color: { type: String },
    size: { type: String },
  };

  declare items: TabItem[];
  declare value: string;
  declare variant: "link" | "pill";
  declare color: string;
  declare size: string;

  static styles = css`
    :host {
      display: block;
    }
    [role="tablist"] {
      display: flex;
      gap: var(--tx-space-1);
      overflow-x: auto;
      font-size: var(--_font, var(--tx-text-sm));
    }
    [role="tablist"][data-variant="link"] {
      border-bottom: 1px solid var(--tx-border);
    }
    [role="tab"] {
      display: inline-flex;
      align-items: center;
      gap: var(--tx-space-2);
      padding: var(--tx-space-2) var(--tx-space-4);
      border: 0;
      background: transparent;
      color: var(--tx-text-muted);
      font: inherit;
      font-weight: 500;
      white-space: nowrap;
      cursor: pointer;
    }
    [role="tab"]:hover:not(:disabled):not([aria-selected="true"]) {
      color: var(--tx-text-highlighted);
    }
    [role="tab"]:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }
    [role="tab"]:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: -2px;
    }
    [data-variant="link"] [role="tab"] {
      margin-bottom: -1px;
      border-bottom: 2px solid transparent;
    }
    [data-variant="link"] [role="tab"][aria-selected="true"] {
      border-bottom-color: var(--_color);
      color: var(--_color);
    }
    [data-variant="pill"] [role="tab"] {
      border-radius: var(--tx-radius);
    }
    [data-variant="pill"] [role="tab"][aria-selected="true"] {
      background: var(--tx-bg-muted);
      color: var(--_color);
    }
    [role="tabpanel"] {
      padding-top: var(--tx-space-3);
    }
    [role="tabpanel"]:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
  `;

  private uid = ++uid;

  constructor() {
    super();
    this.items = [];
    this.value = "";
    this.variant = "link";
    this.color = "primary";
    this.size = "md";
  }

  protected willUpdate(): void {
    if (!this.items.some((i) => i.value === this.value && !i.disabled)) this.value = this.items.find((i) => !i.disabled)?.value ?? "";
  }

  private select(value: string, focus: boolean): void {
    const changed = value !== this.value;
    this.value = value;
    if (changed) this.dispatchEvent(new CustomEvent("tx-change", { detail: { value }, bubbles: true, composed: true }));
    if (focus) {
      void this.updateComplete.then(() => this.renderRoot.querySelector<HTMLElement>(`[role=tab][data-value="${CSS.escape(value)}"]`)?.focus());
    }
  }

  private onKeydown(event: KeyboardEvent): void {
    const enabled = this.items.filter((i) => !i.disabled);
    if (enabled.length === 0) return;
    const at = enabled.findIndex((i) => i.value === this.value);
    let next: number;
    switch (event.key) {
      case "ArrowRight":
        next = (at + 1) % enabled.length;
        break;
      case "ArrowLeft":
        next = (at - 1 + enabled.length) % enabled.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = enabled.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    this.select(enabled[next]!.value, true);
  }

  render() {
    const base = `tx-tabs-${this.uid}`;
    const style = `--_color: ${COLORS[this.color] ?? COLORS.primary}; --_font: ${FONT[this.size] ?? FONT.md}`;
    return html`
      <div role="tablist" data-variant=${this.variant} style=${style} @keydown=${this.onKeydown}>
        ${this.items.map(
          (item, i) => html`<button
            type="button"
            role="tab"
            id="${base}-tab-${i}"
            data-value=${item.value}
            aria-selected=${item.value === this.value ? "true" : "false"}
            aria-controls="${base}-panel-${i}"
            tabindex=${item.value === this.value ? 0 : -1}
            ?disabled=${item.disabled}
            @click=${() => this.select(item.value, false)}
          >
            ${item.icon ? icon(item.icon, 16) : nothing}<span>${item.label}</span>
          </button>`,
        )}
      </div>
      ${this.items.map(
        (item, i) => html`<div
          role="tabpanel"
          id="${base}-panel-${i}"
          aria-labelledby="${base}-tab-${i}"
          tabindex="0"
          ?hidden=${item.value !== this.value}
        >
          <slot name=${item.value}></slot>
        </div>`,
      )}
    `;
  }
}

if (!customElements.get("tx-tabs")) customElements.define("tx-tabs", TxTabs);
