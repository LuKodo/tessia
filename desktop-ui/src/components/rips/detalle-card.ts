import { LitElement, css, html, nothing } from "lit";
import "../tx-card";

/** Bordered, tinted card used for every block of the invoice detail modal. Slot: default. */
export class DetalleCard extends LitElement {
  static properties = { title: { type: String } };

  static styles = css`
    :host {
      display: block;
      flex-shrink: 0;
    }
    .title {
      margin-bottom: var(--tx-space-2);
      font-size: 0.75rem;
      font-weight: 700;
      color: var(--tx-text-muted);
    }
  `;

  title = "";

  render() {
    return html`<tx-card variant="subtle">
      ${this.title ? html`<div class="title" data-testid="card-title">${this.title}</div>` : nothing}
      <slot></slot>
    </tx-card>`;
  }
}

if (!customElements.get("detalle-card")) customElements.define("detalle-card", DetalleCard);
