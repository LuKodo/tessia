import { LitElement, css, html, nothing } from "lit";
import { styleMap } from "lit/directives/style-map.js";
import type { Color } from "./tx-badge";

/**
 * Progress bar. Attributes: `value` (omit for indeterminate), `max` (default 100), `color`, `size`
 * (`xs..xl`, default `md`), `indeterminate` (force the busy animation), `label` (accessible name) and
 * `value-text` (human text for assistive tech, e.g. "3 de 10 pasos"). The host reflects
 * `data-state="indeterminate|loading|complete"`. The inner track has `role="progressbar"`.
 */
export class TxProgress extends LitElement {
  static properties = {
    value: { type: Number },
    max: { type: Number },
    color: { type: String, reflect: true },
    size: { type: String, reflect: true },
    indeterminate: { type: Boolean },
    label: { type: String },
    valueText: { type: String, attribute: "value-text" },
  };

  static styles = css`
    :host {
      --_solid: var(--tx-primary);
      --_h: 0.5rem;
      display: block;
    }
    :host([color="neutral"]) {
      --_solid: var(--tx-bg-inverted);
    }
    :host([color="success"]) {
      --_solid: var(--tx-success);
    }
    :host([color="info"]) {
      --_solid: var(--tx-info);
    }
    :host([color="warning"]) {
      --_solid: var(--tx-warning);
    }
    :host([color="error"]) {
      --_solid: var(--tx-error);
    }
    :host([size="xs"]) {
      --_h: 0.125rem;
    }
    :host([size="sm"]) {
      --_h: 0.25rem;
    }
    :host([size="lg"]) {
      --_h: 0.75rem;
    }
    :host([size="xl"]) {
      --_h: 1rem;
    }
    .track {
      position: relative;
      overflow: hidden;
      height: var(--_h);
      border-radius: 9999px;
      background: var(--tx-bg-muted);
    }
    .fill {
      height: 100%;
      border-radius: inherit;
      background: var(--_solid);
      transition: width 300ms ease;
    }
    .fill.busy {
      position: absolute;
      inset-block: 0;
      left: 0;
      width: 40%;
      animation: carousel 1.6s ease-in-out infinite;
    }
    @keyframes carousel {
      0% {
        transform: translateX(-100%);
      }
      100% {
        transform: translateX(250%);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .fill {
        transition: none;
      }
      .fill.busy {
        animation: none;
        left: 30%;
      }
    }
  `;

  value: number | null = null;
  max = 100;
  color: Color = "primary";
  size: "xs" | "sm" | "md" | "lg" | "xl" = "md";
  indeterminate = false;
  label = "";
  valueText = "";

  get #busy(): boolean {
    return this.indeterminate || this.value == null || Number.isNaN(this.value);
  }

  get #clamped(): number {
    const max = this.max > 0 ? this.max : 100;
    return Math.min(Math.max(this.value ?? 0, 0), max);
  }

  protected willUpdate(): void {
    const max = this.max > 0 ? this.max : 100;
    this.dataset.state = this.#busy ? "indeterminate" : this.#clamped >= max ? "complete" : "loading";
  }

  protected render() {
    const busy = this.#busy;
    const max = this.max > 0 ? this.max : 100;
    return html`<div
      class="track"
      role="progressbar"
      aria-valuemin="0"
      aria-valuemax=${max}
      aria-valuenow=${busy ? nothing : this.#clamped}
      aria-valuetext=${this.valueText || nothing}
      aria-label=${this.label || nothing}
    >
      <div class=${busy ? "fill busy" : "fill"} style=${busy ? "" : styleMap({ width: `${(this.#clamped / max) * 100}%` })}></div>
    </div>`;
  }
}

if (!customElements.get("tx-progress")) customElements.define("tx-progress", TxProgress);
