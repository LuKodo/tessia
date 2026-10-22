import { LitElement, css, html, nothing } from "lit";
import "./tx-icon";
import type { Color } from "./tx-badge";

/**
 * Avatar. Attributes: `src` (image; falls back to icon/initials when it fails), `alt` (accessible name, also
 * the source of the initials), `icon`, `text` (literal short text), `color` (`neutral` default) and `size`
 * (`xs..xl`, default `md`). With an `alt` the host is an `img` role labelled by it; otherwise it is decorative.
 */
export class TxAvatar extends LitElement {
  static properties = {
    src: { type: String },
    alt: { type: String },
    icon: { type: String },
    text: { type: String },
    color: { type: String, reflect: true },
    size: { type: String, reflect: true },
    _failed: { state: true },
  };

  static styles = css`
    :host {
      --_c: var(--tx-text);
      --_soft: var(--tx-bg-muted);
      --_s: 2rem;
      display: inline-flex;
      flex: none;
      align-items: center;
      justify-content: center;
      overflow: hidden;
      box-sizing: border-box;
      width: var(--_s);
      height: var(--_s);
      border-radius: 9999px;
      background: var(--_soft);
      color: var(--_c);
      font-size: calc(var(--_s) * 0.42);
      font-weight: 500;
      line-height: 1;
      vertical-align: middle;
      user-select: none;
    }
    :host([color="primary"]) {
      --_c: var(--tx-primary);
      --_soft: var(--tx-primary-soft);
    }
    :host([color="success"]) {
      --_c: var(--tx-success);
      --_soft: var(--tx-success-soft);
    }
    :host([color="info"]) {
      --_c: var(--tx-info);
      --_soft: var(--tx-info-soft);
    }
    :host([color="warning"]) {
      --_c: var(--tx-warning);
      --_soft: var(--tx-warning-soft);
    }
    :host([color="error"]) {
      --_c: var(--tx-error);
      --_soft: var(--tx-error-soft);
    }
    :host([size="xs"]) {
      --_s: 1.25rem;
    }
    :host([size="sm"]) {
      --_s: 1.5rem;
    }
    :host([size="lg"]) {
      --_s: 2.25rem;
    }
    :host([size="xl"]) {
      --_s: 2.5rem;
    }
    img {
      width: 100%;
      height: 100%;
      object-fit: cover;
    }
    tx-icon {
      --tx-icon-size: 60%;
    }
  `;

  src = "";
  alt = "";
  icon = "";
  text = "";
  color: Color = "neutral";
  size: "xs" | "sm" | "md" | "lg" | "xl" = "md";
  _failed = false;

  protected willUpdate(changed: Map<string, unknown>): void {
    if (changed.has("src")) this._failed = false; // a new source gets another chance
    if (this.alt) {
      this.setAttribute("role", "img");
      this.setAttribute("aria-label", this.alt);
      this.removeAttribute("aria-hidden");
    } else {
      this.removeAttribute("role");
      this.removeAttribute("aria-label");
      this.setAttribute("aria-hidden", "true");
    }
  }

  get #initials(): string {
    return this.alt
      .split(/\s+/)
      .filter(Boolean)
      .slice(0, 2)
      .map((word) => word.charAt(0).toLocaleUpperCase())
      .join("");
  }

  protected render() {
    if (this.src && !this._failed) {
      return html`<img src=${this.src} alt="" @error=${() => (this._failed = true)} />`;
    }
    if (this.icon) return html`<tx-icon name=${this.icon}></tx-icon>`;
    const text = this.text || this.#initials;
    return text ? html`<span class="text">${text}</span>` : nothing;
  }
}

if (!customElements.get("tx-avatar")) customElements.define("tx-avatar", TxAvatar);
