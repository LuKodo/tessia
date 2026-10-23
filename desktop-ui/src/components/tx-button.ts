import { LitElement, css, html, nothing } from "lit";
import "./tx-icon";
import type { Color, Size, Variant } from "./tx-badge";

/**
 * Button. Attributes mirror Nuxt UI: `color`, `variant` (solid default), `size`, `disabled`, `loading`,
 * `icon`, `trailing-icon`, `href`/`target` (renders an anchor), `type` (`submit` submits the enclosing form),
 * `aria-label` (forwarded to the inner control, needed for icon-only buttons), `block` (full width).
 * Slots: default, `leading`, `trailing`. The click is the native click of the host.
 */
export class TxButton extends LitElement {
  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };

  static properties = {
    color: { type: String, reflect: true },
    variant: { type: String, reflect: true },
    size: { type: String, reflect: true },
    disabled: { type: Boolean, reflect: true },
    loading: { type: Boolean, reflect: true },
    block: { type: Boolean, reflect: true },
    href: { type: String },
    target: { type: String },
    type: { type: String },
    icon: { type: String },
    trailingIcon: { type: String, attribute: "trailing-icon" },
    label: { type: String, attribute: "aria-label" },
  };

  static styles = css`
    :host {
      --_c: var(--tx-primary);
      --_solid: var(--tx-primary);
      --_soft: var(--tx-primary-soft);
      --_on: light-dark(white, var(--tx-taupe-950));
      --_ring: var(--tx-primary);
      --_line: color-mix(in oklab, var(--tx-primary) 35%, transparent);
      --_bg: var(--_solid);
      --_fg: var(--_on);
      --_bd: transparent;
      --_hover: color-mix(in oklab, var(--_solid) 88%, black);
      --_h: 2rem;
      --_px: 0.75rem;
      --_fs: 0.875rem;

      display: inline-block;
      vertical-align: middle;
    }
    :host([block]) {
      display: block;
    }
    :host([disabled]),
    :host([loading]) {
      pointer-events: none;
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

    :host([variant="outline"]) {
      --_bg: transparent;
      --_fg: var(--_c);
      --_bd: var(--_ring);
      --_hover: var(--_soft);
    }
    :host([variant="soft"]) {
      --_bg: var(--_soft);
      --_fg: var(--_c);
      --_hover: color-mix(in oklab, var(--_soft) 85%, var(--_c));
    }
    :host([variant="subtle"]) {
      --_bg: var(--_soft);
      --_fg: var(--_c);
      --_bd: var(--_line);
      --_hover: color-mix(in oklab, var(--_soft) 85%, var(--_c));
    }
    :host([variant="ghost"]) {
      --_bg: transparent;
      --_fg: var(--_c);
      --_hover: var(--_soft);
    }
    :host([variant="link"]) {
      --_bg: transparent;
      --_fg: var(--_c);
      --_hover: transparent;
    }

    :host([size="xs"]) {
      --_h: 1.5rem;
      --_px: 0.5rem;
      --_fs: 0.75rem;
    }
    :host([size="sm"]) {
      --_h: 1.75rem;
      --_px: 0.625rem;
      --_fs: 0.8125rem;
    }
    :host([size="lg"]) {
      --_h: 2.25rem;
      --_px: 0.875rem;
      --_fs: 0.9375rem;
    }
    :host([size="xl"]) {
      --_h: 2.5rem;
      --_px: 1rem;
      --_fs: 1rem;
    }

    .control {
      display: inline-flex;
      align-items: center;
      justify-content: center;
      gap: 0.375rem;
      box-sizing: border-box;
      width: 100%;
      min-height: var(--_h);
      padding: 0 var(--_px);
      border: 1px solid var(--_bd);
      border-radius: var(--tx-radius);
      background: var(--_bg);
      color: var(--_fg);
      font: inherit;
      font-size: var(--_fs);
      font-weight: 500;
      line-height: 1.25;
      text-decoration: none;
      white-space: nowrap;
      cursor: pointer;
      user-select: none;
      transition: background-color 120ms, color 120ms;
    }
    :host([variant="solid"][color="primary"]) .control {
      box-shadow: 0 4px 6px -1px color-mix(in oklab, var(--tx-primary) 30%, transparent);
    }
    .control:hover {
      background: var(--_hover);
    }
    :host([variant="link"]) .control {
      min-height: 0;
      padding: 0;
      border-radius: 0;
    }
    :host([variant="link"]) .control:hover {
      text-decoration: underline;
    }
    .control:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
    .control:is(:disabled, [aria-disabled="true"]) {
      opacity: 0.5;
      cursor: not-allowed;
      box-shadow: none;
    }
    .control[aria-busy="true"] {
      cursor: progress;
    }
    tx-icon {
      font-size: 1.25em;
    }
    .spinner {
      width: 1.1em;
      height: 1.1em;
      flex: none;
      animation: spin 0.8s linear infinite;
    }
    @keyframes spin {
      to {
        transform: rotate(360deg);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .spinner {
        animation: none;
      }
      .control {
        transition: none;
      }
    }
    slot {
      display: contents;
    }
  `;

  color: Color = "primary";
  variant: Variant = "solid";
  size: Size = "md";
  disabled = false;
  loading = false;
  block = false;
  href = "";
  target = "";
  type: "button" | "submit" | "reset" = "button";
  icon = "";
  trailingIcon = "";
  label = "";

  constructor() {
    super();
    // Capture phase on the host: a disabled or loading button never reaches consumer click listeners,
    // even for synthetic clicks dispatched on the host itself.
    this.addEventListener(
      "click",
      (event) => {
        if (this.disabled || this.loading) {
          event.preventDefault();
          event.stopImmediatePropagation();
        }
      },
      true,
    );
    this.addEventListener("click", (event) => this.#afterClick(event));
  }

  // Shadow DOM isolates the inner control from the page's <form>, so submit/reset is done by hand once the
  // consumer's own listeners had the chance to preventDefault().
  #afterClick(event: Event): void {
    if (this.disabled || this.loading || this.href || this.type === "button") return;
    queueMicrotask(() => {
      if (event.defaultPrevented) return;
      const form = this.closest("form");
      if (!form) return;
      if (this.type === "submit") form.requestSubmit();
      else form.reset();
    });
  }

  protected render() {
    const inactive = this.disabled || this.loading;
    const inner = html`${this.loading
        ? html`<svg class="spinner" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <circle cx="12" cy="12" r="9" stroke="currentColor" stroke-opacity="0.25" stroke-width="3" />
            <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" stroke-width="3" stroke-linecap="round" />
          </svg>`
        : html`<slot name="leading">${this.icon ? html`<tx-icon name=${this.icon}></tx-icon>` : nothing}</slot>`}<slot></slot
      ><slot name="trailing">${this.trailingIcon ? html`<tx-icon name=${this.trailingIcon}></tx-icon>` : nothing}</slot>`;
    const label = this.label || nothing;

    if (this.href) {
      return html`<a
        class="control"
        part="control"
        href=${inactive ? nothing : this.href}
        target=${this.target || nothing}
        rel=${this.target === "_blank" ? "noopener noreferrer" : nothing}
        role=${inactive ? "link" : nothing}
        aria-disabled=${inactive ? "true" : nothing}
        aria-busy=${this.loading ? "true" : nothing}
        aria-label=${label}
        >${inner}</a
      >`;
    }
    return html`<button
      class="control"
      part="control"
      type="button"
      ?disabled=${inactive}
      aria-busy=${this.loading ? "true" : nothing}
      aria-label=${label}
    >
      ${inner}
    </button>`;
  }
}

if (!customElements.get("tx-button")) customElements.define("tx-button", TxButton);
