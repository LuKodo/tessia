import { LitElement, css, html, nothing } from "lit";
import type { TemplateResult } from "lit";
import { repeat } from "lit/directives/repeat.js";
import type { TrackingRow } from "../../types/api";
import { portalIcon, portalIconClass, portalRowSummary } from "../../utils/portal-model";
import { ARTIFACTS } from "../../utils/rips-constants";
import { amountText, whenText } from "../../utils/rips-format";
import { subLine } from "../../utils/table-model";
import type { SortState } from "../../utils/table-model";
import "../tx-alert";
import "../tx-button";
import "../tx-icon";
import { dash } from "./rips-dash";
import "./rips-dot";
import "./rips-estado-badge";
import "./rips-steps";
import { iconName, ripsIcon, spinStyles } from "./rips-tones";

const rowKey = (r: TrackingRow) => (r.cargue || "") + "/" + r.factura;

/**
 * Tracking table of invoices. It shows the rows it gets, in the order it gets them: the server
 * filters, sorts and pages (see rips-dashboard). Events (on the host): `ver-detalle` and `ver-portal`,
 * both with the row as `detail`; `sort-change` with the column key as `detail`; `cargar-mas`.
 * Rendering is kept cheap because the table polls: rows are keyed and a row whose object did not
 * change (see stabilizeRows) is not touched by Lit.
 */
export class RipsTable extends LitElement {
  static properties = {
    rows: { attribute: false },
    total: { type: Number },
    filtered: { type: Boolean },
    hasMore: { type: Boolean },
    loading: { type: Boolean },
    loadingMore: { type: Boolean },
    error: { type: String },
    sort: { attribute: false },
  };

  static styles = [
    spinStyles,
    css`
      :host {
        display: flex;
        flex-direction: column;
        gap: 0.25rem;
      }
      .summary {
        font-size: 0.75rem;
        color: var(--tx-text-muted);
      }
      /* Own scroll box (bounded height, horizontal scroll): the header rows stick to its top edge. */
      .load-error {
        margin-bottom: 0.25rem;
      }
      .more {
        display: flex;
        justify-content: center;
        padding: var(--tx-space-2) 0;
      }
      .scroll {
        max-height: 75vh;
        overflow: auto;
        border: 1px solid var(--tx-border);
        border-radius: calc(var(--tx-radius) * 4);
        background: var(--tx-bg);
        box-shadow: var(--tx-shadow);
      }
      table {
        width: 100%;
        min-width: 1400px;
        border-collapse: separate;
        border-spacing: 0;
        font-size: 0.75rem;
      }
      th {
        position: sticky;
        top: 0;
        z-index: 10;
        padding: 0.25rem 0.625rem;
        border-bottom: 1px solid var(--tx-border);
        background: var(--tx-bg-muted);
        color: var(--tx-text-highlighted);
        font-weight: 600;
        text-align: left;
      }
      thead tr {
        height: 1.75rem;
      }
      thead tr + tr th {
        top: 1.75rem;
      }
      th.center,
      td.center {
        text-align: center;
      }
      th.estado {
        min-width: 130px;
      }
      th.sortable {
        cursor: pointer;
        white-space: nowrap;
      }
      th.sortable tx-icon {
        font-size: 0.75rem;
      }
      td {
        padding: 0.25rem 0.625rem;
        vertical-align: middle;
      }
      td.nowrap {
        white-space: nowrap;
      }
      tbody tr:nth-child(even) {
        background: color-mix(in oklab, var(--tx-bg-muted) 50%, transparent);
      }
      tbody tr:hover {
        background: var(--tx-bg-elevated);
      }
      .empty {
        padding: 2rem 0;
        text-align: center;
        color: var(--tx-text-muted);
      }
      .empty .loading {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: var(--tx-space-3);
      }
      .empty tx-icon {
        font-size: 1.25rem;
      }
      .dash,
      .muted {
        color: var(--tx-text-muted);
      }
      .factura {
        font-weight: 700;
        white-space: nowrap;
      }
      .portal-icon {
        margin-right: 0.25rem;
        font-size: 1rem;
      }
      .portal-icon[data-tone="neutral"] {
        color: var(--tx-text-muted);
      }
      .portal-icon[data-tone="error"] {
        color: var(--tx-error);
      }
      .portal-icon[data-tone="warning"] {
        color: var(--tx-warning);
      }
      .portal-icon[data-tone="info"] {
        color: var(--tx-info);
      }
      .portal-icon[data-tone="success"] {
        color: var(--tx-success);
      }
      /* Button-styled text: looks like the plain invoice number, underlined on hover/focus. */
      .invoice-link {
        all: unset;
        cursor: pointer;
        font: inherit;
        font-weight: inherit;
        color: inherit;
        border-radius: 2px;
      }
      .invoice-link:hover,
      .invoice-link:focus-visible {
        text-decoration: underline;
        color: var(--tx-primary);
      }
      .invoice-link:focus-visible {
        outline: var(--tx-focus-ring);
        outline-offset: 1px;
      }
    `,
  ];

  rows: TrackingRow[] = [];
  /** Invoices the server counted for the active filters; undefined until its aggregates arrive. */
  total: number | undefined = undefined;
  /** Whether a filter is active (only changes the wording). */
  filtered = false;
  /** Whether the server has more rows after the loaded ones. */
  hasMore = false;
  loading = false;
  loadingMore = false;
  error = "";
  sort: SortState = { key: null, dir: 1 };

  private sortBy(key: string) {
    this.dispatchEvent(new CustomEvent("sort-change", { detail: key }));
  }

  private sortIcon(key: string): string {
    return ripsIcon(this.sort.key !== key ? "sort" : this.sort.dir === 1 ? "arrowUp" : "arrowDown");
  }

  private sortState(key: string): "ascending" | "descending" | "none" {
    return this.sort.key !== key ? "none" : this.sort.dir === 1 ? "ascending" : "descending";
  }

  private sortableHeader(key: string, label: string): TemplateResult {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Enter" && e.key !== " ") return;
      e.preventDefault();
      this.sortBy(key);
    };
    return html`<th rowspan="2" class="sortable" tabindex="0" aria-sort=${this.sortState(key)} @click=${() => this.sortBy(key)} @keydown=${onKey}>
      ${label} <tx-icon name=${this.sortIcon(key)}></tx-icon>
    </th>`;
  }

  private emit(name: "ver-detalle" | "ver-portal", row: TrackingRow) {
    this.dispatchEvent(new CustomEvent(name, { detail: row }));
  }

  private rowTemplate(r: TrackingRow): TemplateResult {
    const p = portalIcon(portalRowSummary(r));
    return html`<tr>
      <td class="center">
        <tx-button
          icon=${ripsIcon("eye")}
          size="xs"
          color="neutral"
          variant="ghost"
          title=${`Ver detalle de ${r.factura}`}
          aria-label=${`Ver detalle de ${r.factura}`}
          @click=${() => this.emit("ver-detalle", r)}
        ></tx-button>
      </td>
      <td class="factura" title=${r.cargue_nombre || r.cargue || ""}>
        <tx-icon class="portal-icon" name=${iconName(portalIconClass(p.key))} label=${p.label} data-tone=${p.color} title=${p.label}></tx-icon
        ><button type="button" class="invoice-link" title=${`Ver resultado del portal de ${r.factura}`} @click=${() => this.emit("ver-portal", r)}>${r.factura}</button>
      </td>
      ${ARTIFACTS.map(
        (artifact, i) => html`<td class="center"><rips-dot .status=${(r.artefactos || [])[i] || "pendiente"} .label=${artifact[1]}></rips-dot></td>`,
      )}
      <td><rips-steps .list=${r.radicacion}></rips-steps></td>
      <td><rips-estado-badge .row=${r}></rips-estado-badge></td>
      <td class="nowrap">${dash(r.fecha_factura)}</td>
      <td>${dash(r.ssc)}</td>
      <td>${dash()}</td>
      <td>${dash()}</td>
      <td>${dash(r.subplan)}</td>
      <td class="nowrap">
        <div><span class="muted">XML:</span> ${dash(amountText(r.valor_xml))}</div>
        <div><span class="muted">Convenio:</span> ${dash(amountText(r.valor_convenio))}</div>
      </td>
      <td>${dash(r.autorizacion)}</td>
      <td>${dash(r.id_mipres)}</td>
      <td class="nowrap">${dash(whenText(r.actualizado_en))}</td>
    </tr>`;
  }

  render() {
    const summaryLine = subLine({ total: this.total, loaded: this.rows.length, filtered: this.filtered });
    return html`<div class="summary">${summaryLine}</div>
      ${this.error
        ? html`<tx-alert
            class="load-error"
            color="error"
            variant="subtle"
            role="alert"
            icon=${ripsIcon("circleX")}
            description=${`No se pudieron cargar las facturas: ${this.error}`}
          ></tx-alert>`
        : nothing}
      <div class="scroll">
        <table class="tracking-table">
          <thead>
            <tr>
              <th rowspan="2">Ver</th>
              <th rowspan="2">Factura</th>
              <th colspan="5" class="center">Proceso</th>
              <th rowspan="2" class="estado">Estado</th>
              ${this.sortableHeader("fecha_factura", "Fecha factura")}
              <th rowspan="2">SSC</th>
              <th rowspan="2">N.º caso MEG</th>
              <th rowspan="2">SLA Mutualser</th>
              <th rowspan="2">Subplan</th>
              <th rowspan="2">Valor total</th>
              <th rowspan="2">Autorización</th>
              <th rowspan="2">ID MIPRES</th>
              ${this.sortableHeader("actualizado_en", "Actualizado")}
            </tr>
            <tr>
              <th class="center">ZIP</th>
              <th class="center">JSON</th>
              <th class="center">TXT/CUV</th>
              <th class="center">ZIP CRC</th>
              <th class="center">Radicación (1-2-3-4)</th>
            </tr>
          </thead>
          <tbody>
            ${this.loading && !this.rows.length
              ? html`<tr>
                  <td colspan="17" class="empty">
                    <div class="loading"><tx-icon class="spin" name="refresh" label="Cargando"></tx-icon><span>Cargando facturas…</span></div>
                  </td>
                </tr>`
              : !this.rows.length
                ? html`<tr>
                    <td colspan="17" class="empty">${this.filtered ? "Ninguna factura coincide con el filtro" : "Sin facturas cargadas"}</td>
                  </tr>`
                : nothing}
            ${repeat(this.rows, rowKey, (r) => this.rowTemplate(r))}
          </tbody>
        </table>
      </div>
      ${this.hasMore
        ? html`<div class="more">
            <tx-button variant="outline" ?loading=${this.loadingMore} @click=${() => this.dispatchEvent(new CustomEvent("cargar-mas"))}>Cargar más</tx-button>
          </div>`
        : nothing}`;
  }
}

if (!customElements.get("rips-table")) customElements.define("rips-table", RipsTable);
