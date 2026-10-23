import { LitElement, css, html } from "lit";
import { repeat } from "lit/directives/repeat.js";
import "./tx-badge";
import "./tx-card";
import { fmtTime } from "../utils/format";
import { statusColor } from "../utils/status-color";
import type { RadicadaRow } from "../types/api";

const rowKey = (row: RadicadaRow) => `${row.cargue}\u0000${row.factura}`;

/** Presentational table of the "Reportes" page: one row per filed invoice. It emits no events. */
export class ReportesTabla extends LitElement {
  static properties = {
    rows: { attribute: false },
  };

  static styles = css`
    :host {
      display: block;
    }
    .empty {
      padding: var(--tx-space-6);
      text-align: center;
      color: var(--tx-text-muted);
    }
    .scroll {
      max-height: calc(100vh - 380px);
      overflow: auto;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      font-size: var(--tx-text-sm);
      color: var(--tx-text);
    }
    th {
      position: sticky;
      top: 0;
      z-index: 1;
      padding: var(--tx-space-2) var(--tx-space-3);
      background: var(--tx-bg-elevated);
      text-align: left;
      font-size: 0.75rem;
      font-weight: 600;
      color: var(--tx-text-muted);
    }
    td {
      padding: var(--tx-space-2) var(--tx-space-3);
      vertical-align: top;
    }
    tr.reportes-row {
      border-top: 1px solid var(--tx-border);
    }
    .nowrap {
      white-space: nowrap;
    }
    .factura {
      font-weight: 500;
    }
    .observaciones {
      min-width: 18rem;
      overflow-wrap: anywhere;
    }
    .none {
      color: var(--tx-text-muted);
    }
  `;

  rows: RadicadaRow[] = [];

  private renderRow(row: RadicadaRow) {
    const filed = fmtTime(row.fecha_radicacion);
    return html`<tr class="reportes-row">
      <td class="nowrap" title=${row.cargue}>${row.cargue_nombre || row.cargue}</td>
      <td class="factura nowrap">${row.factura}</td>
      <td class="nowrap ${filed ? "" : "none"}">${filed || "—"}</td>
      <td class="nowrap"><tx-badge size="sm" variant="subtle" color=${statusColor(row.estado)}>${row.estado}</tx-badge></td>
      <td class="observaciones ${row.observaciones ? "" : "none"}">${row.observaciones || "—"}</td>
    </tr>`;
  }

  protected render() {
    if (this.rows.length === 0) {
      return html`<tx-card><div class="empty" data-testid="reportes-empty">Sin facturas radicadas con estos filtros</div></tx-card>`;
    }
    return html`<tx-card padding="none">
      <div class="scroll">
        <table>
          <thead>
            <tr>
              <th>Cargue</th>
              <th>Factura</th>
              <th>Fecha de radicación</th>
              <th>Estado</th>
              <th>Observaciones</th>
            </tr>
          </thead>
          <tbody>
            ${repeat(this.rows, rowKey, (row) => this.renderRow(row))}
          </tbody>
        </table>
      </div>
    </tx-card>`;
  }
}

if (!customElements.get("tx-reportes-tabla")) customElements.define("tx-reportes-tabla", ReportesTabla);

declare global {
  interface HTMLElementTagNameMap {
    "tx-reportes-tabla": ReportesTabla;
  }
}
