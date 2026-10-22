import { LitElement, css, html } from "lit";

/** Label + value pair of the invoice detail modal. Slot: default (the value). */
export class DetalleField extends LitElement {
  static properties = { label: { type: String } };

  static styles = css`
    :host {
      display: block;
    }
    .label {
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
    .value {
      overflow-wrap: break-word;
      font-size: var(--tx-text-sm);
      font-weight: 600;
    }
  `;

  label = "";

  render() {
    return html`<div class="label">${this.label}</div>
      <div class="value"><slot></slot></div>`;
  }
}

if (!customElements.get("detalle-field")) customElements.define("detalle-field", DetalleField);
