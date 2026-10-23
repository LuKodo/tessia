import { LitElement, css, html, nothing } from "lit";
import type { InvoiceTracking } from "../../types/api";
import { formatMoney, glosaRisk, hasConvenio, valueDiff } from "../../utils/detalle-model";
import { amountText } from "../../utils/rips-format";
import "../tx-badge";
import "../tx-button";
import "../tx-skeleton";
import "./detalle-card";
import "./detalle-field";

/**
 * "CONVENIO" block of the invoice detail: crossed values, glosa risk and the convenio lines.
 * The lines come with the invoice's detail row, fetched on demand: while `loading` the lines
 * are skeletons, and on `error` the block says so and fires `retry`.
 */
export class DetalleConvenio extends LitElement {
  static properties = { row: { attribute: false }, loading: { type: Boolean }, error: {} };

  static styles = css`
    :host {
      display: block;
    }
    .grid {
      display: grid;
      grid-template-columns: 1fr;
      gap: 0.625rem;
    }
    @media (min-width: 640px) {
      .grid {
        grid-template-columns: repeat(2, 1fr);
      }
    }
    @media (min-width: 1024px) {
      .grid {
        grid-template-columns: repeat(3, 1fr);
      }
    }
    .muted {
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
    .skeleton-lines {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-2);
      margin-top: var(--tx-space-3);
    }
    .load-error {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-2);
      margin-top: var(--tx-space-3);
      font-size: var(--tx-text-sm);
      color: var(--tx-error);
    }
    .none {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    tx-badge {
      max-width: 100%;
      white-space: normal;
    }
    .lines {
      margin-top: var(--tx-space-3);
      overflow-x: auto;
    }
    table.detail-table {
      width: 100%;
      border-collapse: collapse;
      font-size: 0.75rem;
    }
    .detail-table th,
    .detail-table td {
      padding: 0.25rem 0.5rem;
      border-bottom: 1px solid var(--tx-border);
      text-align: left;
    }
    .detail-table th {
      color: var(--tx-text-highlighted);
      font-weight: 600;
    }
  `;

  row: InvoiceTracking | null = null;
  loading = false;
  error = "";

  render() {
    const row = this.row;
    if (!row) return nothing;
    const diff = valueDiff(row);
    const lines = row.lineas_convenio || [];
    const pending = this.loading || !!this.error; // the lines are not known yet: never claim there are none
    return html`<detalle-card title="CONVENIO" aria-busy=${this.loading ? "true" : "false"}>
      ${hasConvenio(row)
        ? html`<div class="grid">
            <detalle-field label="SSC">${row.ssc || "-"}</detalle-field>
            <detalle-field label="Subplan">${row.subplan || "-"}</detalle-field>
            <detalle-field label="Valor factura XML">${amountText(row.valor_xml) || "-"}</detalle-field>
            <detalle-field label="Valor informe convenio">${amountText(row.valor_convenio) || "-"}</detalle-field>
            ${diff
              ? html`<div>
                  <div class="muted">Diferencia XML − convenio</div>
                  <tx-badge size="md" variant="subtle" color=${diff.ok ? "success" : "error"}>${formatMoney(diff.diff)}</tx-badge>
                </div>`
              : nothing}
            <detalle-field label="Autorización">${row.autorizacion || "-"}</detalle-field>
            <detalle-field label="Id Mipres">${row.id_mipres || "-"}</detalle-field>
            ${glosaRisk(row)
              ? html`<div>
                  <tx-badge
                    size="md"
                    variant="subtle"
                    color="error"
                    icon="shield-alert"
                    title="Se radicó (paso 4 completo) pero no tiene número de autorización en el informe de convenio."
                    >Riesgo de GLOSA (radicada sin autorización)</tx-badge
                  >
                </div>`
              : nothing}
          </div>`
        : pending
          ? nothing
          : html`<div class="none">Sin datos de convenio cruzados para esta factura todavía.</div>`}
      ${this.loading
        ? html`<div class="skeleton-lines">
            <tx-skeleton style="height: 1rem"></tx-skeleton>
            <tx-skeleton style="height: 1rem; width: 80%"></tx-skeleton>
          </div>`
        : nothing}
      ${this.error
        ? html`<div class="load-error" role="alert">
            <span>No se pudo cargar el detalle de la factura: ${this.error}</span>
            <tx-button size="sm" variant="outline" color="neutral" @click=${() => this.dispatchEvent(new CustomEvent("retry"))}>Reintentar</tx-button>
          </div>`
        : nothing}
      ${lines.length > 0
        ? html`<div class="lines">
            <table class="detail-table">
              <thead>
                <tr>
                  <th>SSC</th>
                  <th>CUM</th>
                  <th>Vlr. Unitario</th>
                  <th>Vlr. Total</th>
                  <th>Autorización</th>
                  <th>Id Mipres</th>
                </tr>
              </thead>
              <tbody>
                ${lines.map(
                  (l) => html`<tr>
                    <td>${l.ssc || "-"}</td>
                    <td>${l.cum || "-"}</td>
                    <td>${amountText(l.valor_unitario) || "-"}</td>
                    <td>${amountText(l.valor_total) || "-"}</td>
                    <td>${l.autorizacion || "-"}</td>
                    <td>${l.id_mipres || "-"}</td>
                  </tr>`,
                )}
              </tbody>
            </table>
          </div>`
        : nothing}
    </detalle-card>`;
  }
}

if (!customElements.get("detalle-convenio")) customElements.define("detalle-convenio", DetalleConvenio);
