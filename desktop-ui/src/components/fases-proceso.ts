import { LitElement, css, html, nothing } from "lit";
import { phaseCards } from "../utils/phase-progress";
import type { JobPhaseProgress } from "../types/api";
import "./tx-badge";
import "./tx-card";
import "./tx-progress";

/**
 * Live per-phase cards of the current job (`job.fases`): friendly phase name, settled/total text, a
 * progress bar and small badges for in progress / done / manual review / excluded / errors. A phase
 * with no invoices yet reads "en cola". `radicadas` is `job.radicadas`, the confirmed radicación
 * total. Read-only: it fires no events and renders nothing when there are no phases.
 */
export class TxFasesProceso extends LitElement {
  static properties = {
    fases: { attribute: false },
    radicadas: { type: Number },
  };

  static styles = css`
    :host {
      display: block;
    }
    /* Seven phases plus Radicación: 4 columns x 2 rows on wide screens, 2 on tablets, 1 on phones. */
    .grid {
      display: grid;
      grid-template-columns: repeat(4, minmax(0, 1fr));
      gap: var(--tx-space-2);
    }
    @media (max-width: 959px) {
      .grid {
        grid-template-columns: repeat(2, minmax(0, 1fr));
      }
    }
    @media (max-width: 599px) {
      .grid {
        grid-template-columns: minmax(0, 1fr);
      }
    }
    .caption {
      display: flex;
      justify-content: space-between;
      gap: var(--tx-space-2);
      margin-bottom: var(--tx-space-1);
      font-size: 0.75rem;
      line-height: 1.2;
    }
    .name {
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    .count {
      flex: none;
      color: var(--tx-text-muted);
    }
    .badges {
      display: flex;
      flex-wrap: wrap;
      gap: var(--tx-space-1);
      margin-top: var(--tx-space-1);
    }
    .time {
      margin-top: var(--tx-space-1);
      font-size: 0.6875rem;
      color: var(--tx-text-muted);
    }
  `;

  fases?: JobPhaseProgress[];
  radicadas?: number;

  render() {
    const cards = phaseCards(this.fases, this.radicadas);
    if (!cards.length) return nothing;
    return html`
      <div class="grid">
        ${cards.map(
          (c) => html`
            <tx-card padding="sm" data-testid="fase-card" data-fase=${c.key} data-estado=${c.queued ? "en-cola" : "activa"}>
              <div class="caption">
                <span class="name">${c.name}</span>
                <span class="count">${c.text}</span>
              </div>
              <tx-progress size="sm" .color=${c.color} .value=${c.pct} .label=${c.name} .valueText=${`${c.text.replace("/", " de ")} facturas`}></tx-progress>
              <div class="badges">
                ${c.queued ? html`<tx-badge size="sm" variant="subtle" color="neutral">en cola</tx-badge>` : nothing}
                ${c.badges.map((b) => html`<tx-badge size="sm" variant="subtle" color=${b.color}>${b.label}</tx-badge>`)}
              </div>
              ${c.time ? html`<div class="time">${c.time}</div>` : nothing}
            </tx-card>
          `,
        )}
      </div>
    `;
  }
}

if (!customElements.get("tx-fases-proceso")) customElements.define("tx-fases-proceso", TxFasesProceso);
