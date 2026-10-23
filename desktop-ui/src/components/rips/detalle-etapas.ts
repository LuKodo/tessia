import { LitElement, css, html, nothing } from "lit";
import type { TrackingRow } from "../../types/api";
import { stageCards, timelineItems } from "../../utils/detalle-model";
import type { TimelineState } from "../../utils/detalle-model";
import "../tx-icon";
import "../tx-timeline";
import "./detalle-card";
import { ripsIcon } from "./rips-tones";

const TIMELINE: Record<TimelineState, { color: string; suffix: string }> = {
  completado: { color: "success", suffix: "(completado)" },
  reproceso: { color: "warning", suffix: "(pendiente por reproceso - Nota de Ajuste)" },
  pendiente: { color: "neutral", suffix: "(pendiente)" },
};

/** "DETALLE POR ETAPA" cards and the filing timeline, side by side. */
export class DetalleEtapas extends LitElement {
  static properties = { row: { attribute: false } };

  static styles = css`
    :host {
      display: block;
    }
    .layout {
      display: grid;
      grid-template-columns: 1fr;
      gap: var(--tx-space-3);
    }
    @media (min-width: 1024px) {
      .layout {
        grid-template-columns: 7fr 5fr;
      }
    }
    .col {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-2);
    }
    .heading {
      font-size: 0.75rem;
      font-weight: 700;
      color: var(--tx-text-muted);
    }
    .stage {
      display: flex;
      align-items: center;
      gap: var(--tx-space-2);
      padding: var(--tx-space-3);
      border: 1px solid var(--tx-border);
      border-left-width: 4px;
      border-radius: var(--tx-radius-lg);
      background: var(--tx-bg-muted);
    }
    .stage.ok {
      border-left-color: var(--tx-success);
    }
    .stage.error {
      border-left-color: var(--tx-error);
    }
    .stage tx-icon {
      font-size: 18px;
    }
    .stage.ok tx-icon {
      color: var(--tx-success);
    }
    .stage.error tx-icon {
      color: var(--tx-error);
    }
    .stage span {
      font-size: var(--tx-text-sm);
      font-weight: 600;
    }
    .step-title {
      font-size: var(--tx-text-sm);
      font-weight: 600;
    }
    .suffix-success {
      color: var(--tx-success);
    }
    .suffix-warning {
      color: var(--tx-warning);
    }
    .suffix-neutral {
      color: var(--tx-text-muted);
    }
  `;

  row: TrackingRow | null = null;

  render() {
    if (!this.row) return nothing;
    const stages = stageCards(this.row);
    const timeline = timelineItems(this.row);
    const items = timeline.map((t) => ({ title: t.label, color: TIMELINE[t.state].color }));
    return html`<div class="layout">
      <div class="col">
        <div class="heading">DETALLE POR ETAPA</div>
        ${stages.map(
          (c) => html`<div class="stage ${c.ok ? "ok" : "error"}" data-testid="stage-card">
            <tx-icon name=${ripsIcon(c.ok ? "circleCheckFilled" : "alertCircleFilled")}></tx-icon>
            <span>${c.titulo}</span>
          </div>`,
        )}
      </div>
      <div class="col">
        <div class="heading">LÍNEA DE TIEMPO DE RADICACIÓN</div>
        <detalle-card>
          <tx-timeline .items=${items} size="xs" color="neutral">
            ${timeline.map(
              (t, i) => html`<span slot="title-${i}" class="step-title"
                >${t.label} <span class="suffix-${TIMELINE[t.state].color}">${TIMELINE[t.state].suffix}</span></span
              >`,
            )}
          </tx-timeline>
        </detalle-card>
      </div>
    </div>`;
  }
}

if (!customElements.get("detalle-etapas")) customElements.define("detalle-etapas", DetalleEtapas);
