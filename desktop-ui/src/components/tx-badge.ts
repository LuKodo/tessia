import { LitElement, css, html, nothing } from "lit";
import "./tx-icon";

export type Color = "primary" | "neutral" | "success" | "warning" | "error" | "info";
export type Variant = "solid" | "outline" | "soft" | "subtle" | "ghost" | "link";
export type Size = "xs" | "sm" | "md" | "lg" | "xl";

/**
 * Small status label. Attributes mirror Nuxt UI: `color`, `variant` (solid default), `size`, `icon`,
 * `trailing-icon`. Slots: default, `leading`, `trailing`. Style the host (text-transform, white-space,
 * max-width...) from outside; the look lives on `:host`.
 */
export class TxBadge extends LitElement {
  static properties = {
    color: { type: String, reflect: true },
    variant: { type: String, reflect: true },
    size: { type: String, reflect: true },
    icon: { type: String },
    trailingIcon: { type: String, attribute: "trailing-icon" },
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
      --_h: 1.25rem;
      --_px: 0.5rem;
      --_fs: 0.75rem;

      display: inline-flex;
      align-items: center;
      gap: 0.25rem;
      box-sizing: border-box;
      min-height: var(--_h);
      padding: 0 var(--_px);
      border: 1px solid var(--_bd);
      border-radius: var(--tx-radius);
      background: var(--_bg);
      color: var(--_fg);
      font-size: var(--_fs);
      font-weight: 500;
      line-height: 1;
      vertical-align: middle;
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
    }
    :host([variant="soft"]) {
      --_bg: var(--_soft);
      --_fg: var(--_c);
    }
    :host([variant="subtle"]) {
      --_bg: var(--_soft);
      --_fg: var(--_c);
      --_bd: var(--_line);
    }
    :host([variant="ghost"]),
    :host([variant="link"]) {
      --_bg: transparent;
      --_fg: var(--_c);
    }

    :host([size="xs"]) {
      --_h: 1rem;
      --_px: 0.25rem;
      --_fs: 0.625rem;
    }
    :host([size="sm"]) {
      --_h: 1.125rem;
      --_px: 0.375rem;
      --_fs: 0.6875rem;
    }
    :host([size="lg"]) {
      --_h: 1.5rem;
      --_px: 0.625rem;
      --_fs: 0.875rem;
    }
    :host([size="xl"]) {
      --_h: 1.75rem;
      --_px: 0.75rem;
      --_fs: 1rem;
    }
    tx-icon {
      font-size: 1.1em;
    }
    slot {
      display: contents;
    }
  `;

  color: Color = "primary";
  variant: Variant = "solid";
  size: Size = "md";
  icon = "";
  trailingIcon = "";

  protected render() {
    return html`<slot name="leading">${this.icon ? html`<tx-icon name=${this.icon}></tx-icon>` : nothing}</slot
      ><slot></slot
      ><slot name="trailing">${this.trailingIcon ? html`<tx-icon name=${this.trailingIcon}></tx-icon>` : nothing}</slot>`;
  }
}

if (!customElements.get("tx-badge")) customElements.define("tx-badge", TxBadge);
