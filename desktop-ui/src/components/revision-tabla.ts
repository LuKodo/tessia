import { LitElement, css, html, nothing } from "lit";
import { repeat } from "lit/directives/repeat.js";
import "./tx-badge";
import "./tx-button";
import "./tx-card";
import "./tx-checkbox";
import { fmtTime } from "../utils/format";
import { categoryColor, categoryLabel, phaseLabel } from "../utils/informe-model";
import { rowKey } from "../utils/revision-batch-model";
import type { RevisionRow } from "../utils/revision-batch-model";

/**
 * Presentational table of the manual review page. Only the rows whose key is in `retryable` get a
 * checkbox and a "Reintentar" button; the page decides which ones those are.
 *
 * Events (non-bubbling, `detail` in braces): `row-select` {key, checked}, `row-select-all`
 * {checked} and `row-retry` {row}.
 */
export class RevisionTabla extends LitElement {
  static properties = {
    rows: { attribute: false },
    retryable: { attribute: false },
    selected: { attribute: false },
    allState: { attribute: "all-state" },
    retrying: { type: Boolean },
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
      max-height: calc(100vh - 330px);
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
    th.check {
      width: 2.5rem;
    }
    td {
      padding: var(--tx-space-2) var(--tx-space-3);
      vertical-align: top;
    }
    tr.revision-row {
      border-top: 1px solid var(--tx-border);
    }
    .nowrap {
      white-space: nowrap;
    }
    .factura {
      font-weight: 500;
    }
    .codigo {
      font-family: var(--tx-font-mono);
      font-size: 0.75rem;
      overflow-wrap: anywhere;
    }
    .motivo {
      min-width: 15rem;
      overflow-wrap: anywhere;
    }
  `;

  rows: RevisionRow[] = [];
  /** Keys (`rowKey`) of the rows that can be retried. */
  retryable: ReadonlySet<string> = new Set();
  /** Keys of the rows currently selected. */
  selected: ReadonlySet<string> = new Set();
  allState: "none" | "some" | "all" = "none";
  retrying = false;

  private emit<T>(name: string, detail: T) {
    this.dispatchEvent(new CustomEvent<T>(name, { detail }));
  }

  private renderRow(row: RevisionRow) {
    const key = rowKey(row);
    const canRetry = this.retryable.has(key);
    return html`<tr class="revision-row">
      <td>
        ${canRetry
          ? html`<tx-checkbox
              data-testid="row-select"
              aria-label=${`Seleccionar ${row.factura}`}
              .checked=${this.selected.has(key)}
              ?disabled=${this.retrying}
              @tx-change=${(e: CustomEvent<{ checked: boolean }>) => this.emit("row-select", { key, checked: e.detail.checked })}
            ></tx-checkbox>`
          : nothing}
      </td>
      <td class="cargue nowrap">${row.cargue}</td>
      <td class="factura nowrap">${row.factura}</td>
      <td class="nowrap">${phaseLabel(row.fase)}</td>
      <td class="nowrap">
        <tx-badge size="sm" variant="subtle" color=${categoryColor(row.categoria)}>${categoryLabel(row.categoria)}</tx-badge>
      </td>
      <td class="codigo">${row.codigo}</td>
      <td class="motivo">${row.motivo}</td>
      <td class="nowrap">${fmtTime(row.actualizado_en)}</td>
      <td class="nowrap">
        ${canRetry
          ? html`<tx-button
              data-testid="retry-invoice"
              size="sm"
              variant="subtle"
              ?disabled=${this.retrying}
              @click=${() => this.emit("row-retry", { row })}
              >Reintentar</tx-button
            >`
          : nothing}
      </td>
    </tr>`;
  }

  protected render() {
    if (this.rows.length === 0) {
      return html`<tx-card><div class="empty" data-testid="revision-empty">Sin facturas por revisar</div></tx-card>`;
    }
    return html`<tx-card padding="none">
      <div class="scroll">
        <table>
          <thead>
            <tr>
              <th class="check">
                <tx-checkbox
                  data-testid="select-all"
                  aria-label="Seleccionar todas las visibles"
                  .checked=${this.allState === "all"}
                  .indeterminate=${this.allState === "some"}
                  ?disabled=${this.retrying || this.retryable.size === 0}
                  @tx-change=${(e: CustomEvent<{ checked: boolean }>) => this.emit("row-select-all", { checked: e.detail.checked })}
                ></tx-checkbox>
              </th>
              <th>Cargue</th>
              <th>Factura</th>
              <th>Fase</th>
              <th>Categoría</th>
              <th>Código</th>
              <th>Motivo</th>
              <th>Actualizado</th>
              <th>Acción</th>
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

if (!customElements.get("tx-revision-tabla")) customElements.define("tx-revision-tabla", RevisionTabla);

declare global {
  interface HTMLElementTagNameMap {
    "tx-revision-tabla": RevisionTabla;
  }
}
