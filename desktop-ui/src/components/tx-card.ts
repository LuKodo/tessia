import { LitElement, css, html, nothing } from "lit";

/**
 * Surface container. Attributes: `variant` (`subtle` default, `solid`, `outline`, `soft`) and `padding`
 * (`md` default, `sm`, `none`). Slots: `header`, default (body) and `footer`; header and footer only render
 * when something is slotted into them. The surface (background, border, radius) is on `:host`, so a page can
 * override it from outside (e.g. `border-left: 4px solid ...` for a status accent).
 * Parts: `header`, `body`, `footer`.
 */
export class TxCard extends LitElement {
  static properties = {
    variant: { type: String, reflect: true },
    padding: { type: String, reflect: true },
  };

  static styles = css`
    :host {
      --_pad: var(--tx-space-4);
      display: block;
      box-sizing: border-box;
      border: 1px solid var(--tx-border);
      border-radius: var(--tx-radius-lg);
      background: var(--tx-bg-muted);
      color: var(--tx-text);
    }
    :host([variant="solid"]) {
      border-color: transparent;
      background: var(--tx-bg-inverted);
      color: var(--tx-text-inverted);
    }
    :host([variant="outline"]) {
      background: var(--tx-bg);
    }
    :host([variant="soft"]) {
      border-color: transparent;
    }
    :host([padding="sm"]) {
      --_pad: var(--tx-space-3);
    }
    :host([padding="none"]) {
      --_pad: 0;
    }
    .header,
    .footer {
      padding: var(--tx-space-3) var(--_pad);
    }
    .header {
      border-bottom: 1px solid var(--tx-border);
    }
    .footer {
      border-top: 1px solid var(--tx-border);
    }
    .body {
      padding: var(--_pad);
    }
  `;

  variant: "solid" | "outline" | "soft" | "subtle" = "subtle";
  padding: "none" | "sm" | "md" = "md";

  #has(name: string): boolean {
    return [...this.children].some((child) => child.getAttribute("slot") === name);
  }

  // Header/footer are slotted by attribute, which never fires `slotchange` while their slot is absent.
  #observer = new MutationObserver(() => this.requestUpdate());

  connectedCallback(): void {
    super.connectedCallback();
    this.#observer.observe(this, { childList: true, attributes: true, attributeFilter: ["slot"] });
  }

  disconnectedCallback(): void {
    this.#observer.disconnect();
    super.disconnectedCallback();
  }

  protected render() {
    return html`${this.#has("header")
        ? html`<div class="header" part="header"><slot name="header"></slot></div>`
        : nothing}
      <div class="body" part="body"><slot></slot></div>
      ${this.#has("footer")
        ? html`<div class="footer" part="footer"><slot name="footer"></slot></div>`
        : nothing}`;
  }
}

if (!customElements.get("tx-card")) customElements.define("tx-card", TxCard);
