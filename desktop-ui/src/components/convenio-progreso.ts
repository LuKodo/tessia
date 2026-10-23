import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { convenioStrip, type ConvenioInput } from "../utils/convenio-progress";
import "./tx-badge";
import "./tx-progress";

/**
 * Step 0 ("Preparación del convenio") strip. `convenio` is `job.convenio`; the strip renders nothing
 * when the job has no convenio info.
 */
export class TxConvenioProgreso extends LitElement {
  static properties = { convenio: { attribute: false } };

  static styles = css`
    :host {
      display: block;
    }
    .head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: var(--tx-space-2);
      margin-bottom: var(--tx-space-1);
      font-size: 0.75rem;
      line-height: 1.2;
    }
    .title {
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    .text {
      flex: none;
      color: var(--tx-text-muted);
    }
    .text.error,
    .message {
      color: var(--tx-error);
    }
    .message {
      margin-top: var(--tx-space-1);
      font-size: 0.75rem;
    }
    .chips {
      display: flex;
      flex-wrap: wrap;
      gap: var(--tx-space-1);
      margin-top: var(--tx-space-1);
    }
  `;

  convenio?: ConvenioInput | null;

  render() {
    const strip = convenioStrip(this.convenio);
    if (!strip) return nothing;
    return html`
      <div class="strip" data-estado=${strip.estado}>
        <div class="head">
          <span class="title">${strip.title}</span>
          <span class="text ${strip.estado === "error" ? "error" : ""}" aria-live="polite">${strip.text}</span>
        </div>
        <tx-progress
          size="sm"
          .color=${strip.color}
          .value=${strip.determinate ? (strip.pct ?? 0) : undefined}
          .label=${strip.aria.label}
          .valueText=${strip.aria.text}
        ></tx-progress>
        ${strip.message ? html`<div class="message" role="alert">${strip.message}</div>` : nothing}
        ${strip.chips.length
          ? html`<div class="chips">${strip.chips.map((c) => html`<tx-badge size="sm" variant="subtle" color=${ifDefined(c.color)}>${c.label}</tx-badge>`)}</div>`
          : nothing}
      </div>
    `;
  }
}

if (!customElements.get("tx-convenio-progreso")) customElements.define("tx-convenio-progreso", TxConvenioProgreso);
