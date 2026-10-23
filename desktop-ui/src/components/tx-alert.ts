import { LitElement, css, html, nothing } from "lit";
import "./tx-icon";
import type { Color, Variant } from "./tx-badge";

/**
 * Inline message. Attributes mirror Nuxt UI: `color`, `variant` (`subtle` default), `icon`, `description`,
 * `closable`. The title attribute is `heading` (a `title` attribute would also trigger the native tooltip).
 * Slots: default (extra content under the text) and `actions` (buttons). `close` event from the close button;
 * the consumer decides whether to remove the alert. Role: `alert` for error/warning, `status` otherwise,
 * unless the consumer sets its own `role` (e.g. `note`).
 */
export class TxAlert extends LitElement {
  static properties = {
    color: { type: String, reflect: true },
    variant: { type: String, reflect: true },
    icon: { type: String },
    heading: { type: String },
    description: { type: String },
    closable: { type: Boolean, reflect: true },
  };

  static styles = css`
    :host {
      --_c: var(--tx-primary);
      --_solid: var(--tx-primary);
      --_soft: var(--tx-primary-soft);
      --_on: light-dark(white, var(--tx-taupe-950));
      --_ring: var(--tx-primary);
      --_line: color-mix(in oklab, var(--tx-primary) 35%, transparent);
      --_bg: var(--_soft);
      --_fg: var(--_c);
      --_bd: var(--_line);

      display: block;
      box-sizing: border-box;
      padding: var(--tx-space-3) var(--tx-space-4);
      border: 1px solid var(--_bd);
      border-radius: var(--tx-radius-lg);
      background: var(--_bg);
      color: var(--_fg);
      font-size: var(--tx-text-sm);
    }
    :host([color="neutral"]) {
      --_c: var(--tx-text);
      --_solid: var(--tx-bg-inverted);
      --_soft: var(--tx-bg-muted);
      --_on: var(--tx-text-inverted);
      --_ring: var(--tx-border-strong);
      --_line: var(--tx-border-strong);
    }
    :host([color="success"]) {
      --_c: var(--tx-success);
      --_solid: var(--tx-success);
      --_soft: var(--tx-success-soft);
      --_ring: var(--tx-success);
      --_line: color-mix(in oklab, var(--tx-success) 35%, transparent);
    }
    :host([color="info"]) {
      --_c: var(--tx-info);
      --_solid: var(--tx-info);
      --_soft: var(--tx-info-soft);
      --_ring: var(--tx-info);
      --_line: color-mix(in oklab, var(--tx-info) 35%, transparent);
    }
    :host([color="warning"]) {
      --_c: var(--tx-warning);
      --_solid: var(--tx-warning);
      --_soft: var(--tx-warning-soft);
      --_on: var(--tx-taupe-950);
      --_ring: var(--tx-warning);
      --_line: color-mix(in oklab, var(--tx-warning) 35%, transparent);
    }
    :host([color="error"]) {
      --_c: var(--tx-error);
      --_solid: var(--tx-error);
      --_soft: var(--tx-error-soft);
      --_ring: var(--tx-error);
      --_line: color-mix(in oklab, var(--tx-error) 35%, transparent);
    }
    :host([variant="solid"]) {
      --_bg: var(--_solid);
      --_fg: var(--_on);
      --_bd: transparent;
    }
    :host([variant="outline"]) {
      --_bg: transparent;
      --_bd: var(--_ring);
    }
    :host([variant="soft"]) {
      --_bd: transparent;
    }

    .layout {
      display: flex;
      align-items: flex-start;
      gap: var(--tx-space-3);
    }
    .lead {
      flex: none;
      font-size: 1.25rem;
    }
    .content {
      flex: 1;
      min-width: 0;
    }
    .title {
      font-weight: 600;
      white-space: pre-line;
    }
    .description {
      margin-top: 0.125rem;
      opacity: 0.9;
      white-space: pre-line;
    }
    .title:only-child,
    .description:only-child {
      margin-top: 0;
    }
    .actions {
      display: flex;
      flex-wrap: wrap;
      gap: var(--tx-space-2);
      margin-top: var(--tx-space-2);
    }
    .actions:not(:has(::slotted(*))) {
      display: none;
    }
    .close {
      flex: none;
      display: inline-flex;
      padding: 0.125rem;
      border: 0;
      border-radius: var(--tx-radius);
      background: transparent;
      color: inherit;
      cursor: pointer;
    }
    .close:hover {
      background: color-mix(in oklab, currentColor 15%, transparent);
    }
    .close:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
    .close svg {
      width: 1.1rem;
      height: 1.1rem;
    }
  `;

  color: Color = "primary";
  variant: Variant = "subtle";
  icon = "";
  heading = "";
  description = "";
  closable = false;

  #autoRole = "";

  protected willUpdate(): void {
    const current = this.getAttribute("role");
    if (current !== null && current !== this.#autoRole) return; // the consumer's own role wins
    this.#autoRole = this.color === "error" || this.color === "warning" ? "alert" : "status";
    this.setAttribute("role", this.#autoRole);
  }

  #close = () => {
    this.dispatchEvent(new CustomEvent("close", { bubbles: false, composed: true }));
  };

  protected render() {
    return html`<div class="layout">
      ${this.icon ? html`<tx-icon class="lead" name=${this.icon}></tx-icon>` : nothing}
      <div class="content">
        ${this.heading ? html`<div class="title">${this.heading}</div>` : nothing}
        ${this.description ? html`<div class="description">${this.description}</div>` : nothing}
        <slot></slot>
        <div class="actions"><slot name="actions"></slot></div>
      </div>
      ${this.closable
        ? html`<button class="close" type="button" aria-label="Cerrar" @click=${this.#close}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" aria-hidden="true">
              <path d="M6 6l12 12M18 6 6 18" />
            </svg>
          </button>`
        : nothing}
    </div>`;
  }
}

if (!customElements.get("tx-alert")) customElements.define("tx-alert", TxAlert);
