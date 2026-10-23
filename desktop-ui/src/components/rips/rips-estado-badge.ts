import { LitElement, css, html, nothing } from "lit";
import type { TrackingRow } from "../../types/api";
import { isPositiveDetail } from "../../utils/estado-detalle";
import { estadoClass } from "../../utils/rips-constants";
import "../tx-badge";

/** Status pill of a row, shared by the tracking table and the detail modal so both always agree. */
export class RipsEstadoBadge extends LitElement {
  static properties = { row: { attribute: false }, showDetail: { type: Boolean, attribute: "show-detail" } };

  static styles = css`
    :host {
      display: block;
    }
    tx-badge {
      text-transform: uppercase;
      white-space: nowrap;
    }
    .estado-detail {
      max-width: 150px;
      margin-top: 0.25rem;
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
    .estado-detail.positive {
      color: var(--tx-success);
    }
  `;

  row: TrackingRow | null = null;
  showDetail = true;

  render() {
    const row = this.row;
    if (!row) return nothing;
    const text = row.estado || (row.resultado === "preparada" ? "SIN INICIAR" : row.resultado);
    const detail = this.showDetail ? row.estado_detalle || "" : "";
    const short = detail.length > 90 ? detail.slice(0, 90) + "…" : detail;
    return html`<tx-badge size="sm" variant="subtle" color=${estadoClass(text)}>${(text || "").replace(/_/g, " ")}</tx-badge
      >${detail
        ? html`<div class="estado-detail ${isPositiveDetail(detail) ? "positive" : ""}" title=${detail}>${short}</div>`
        : nothing}`;
  }
}

if (!customElements.get("rips-estado-badge")) customElements.define("rips-estado-badge", RipsEstadoBadge);
