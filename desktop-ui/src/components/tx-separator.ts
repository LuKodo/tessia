import { LitElement, css, html, nothing } from "lit";

/**
 * Divider line. Attributes: `orientation` (`horizontal` default | `vertical`), `label` (text centered on the
 * line; slotted content works too) and `decorative` (`role="none"`). Sets `role="separator"` unless the
 * consumer provided a role. Size vertical separators from outside (`height`).
 */
export class TxSeparator extends LitElement {
  static properties = {
    orientation: { type: String, reflect: true },
    label: { type: String },
    decorative: { type: Boolean },
  };

  static styles = css`
    :host {
      display: flex;
      align-items: center;
      gap: var(--tx-space-3);
      color: var(--tx-text-muted);
      font-size: 0.75rem;
    }
    :host([orientation="vertical"]) {
      display: inline-flex;
      flex-direction: column;
      align-self: stretch;
      min-height: 1rem;
    }
    .line {
      flex: 1;
      border: 0 solid var(--tx-border);
      border-top-width: 1px;
    }
    :host([orientation="vertical"]) .line {
      border-top-width: 0;
      border-left-width: 1px;
    }
    .label {
      flex: none;
    }
  `;

  orientation: "horizontal" | "vertical" = "horizontal";
  label = "";
  decorative = false;

  #autoRole = "";
  #observer = new MutationObserver(() => this.requestUpdate());

  connectedCallback(): void {
    super.connectedCallback();
    this.#observer.observe(this, { childList: true, characterData: true, subtree: true });
  }

  disconnectedCallback(): void {
    this.#observer.disconnect();
    super.disconnectedCallback();
  }

  #hasContent(): boolean {
    return [...this.childNodes].some((n) => n.nodeType === Node.ELEMENT_NODE || (n.textContent ?? "").trim() !== "");
  }

  protected willUpdate(): void {
    const current = this.getAttribute("role");
    if (current === null || current === this.#autoRole) {
      this.#autoRole = this.decorative ? "none" : "separator";
      this.setAttribute("role", this.#autoRole);
    }
    if (this.orientation === "vertical") this.setAttribute("aria-orientation", "vertical");
    else this.removeAttribute("aria-orientation");
  }

  protected render() {
    const labelled = this.label !== "" || this.#hasContent();
    return html`<span class="line"></span>${labelled
        ? html`<span class="label">${this.label}<slot></slot></span><span class="line"></span>`
        : nothing}`;
  }
}

if (!customElements.get("tx-separator")) customElements.define("tx-separator", TxSeparator);
