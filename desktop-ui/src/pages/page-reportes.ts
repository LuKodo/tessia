import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import "../components/reportes-filtros";
import "../components/reportes-tabla";
import "../components/tx-alert";
import "../components/tx-badge";
import "../components/tx-button";
import "../components/tx-progress";
import { NO_FILTERS, rangeError, summaryText, toQuery } from "../components/reportes-model";
import type { ReportFilters } from "../components/reportes-model";
import type { Cargue, RadicadasReport } from "../types/api";
import { errorMessage } from "../utils/error-message";

type ValueEvent<T> = CustomEvent<{ value: T }>;

/**
 * "Reportes": every invoice already filed (all four filing steps confirmed) with its filing date and
 * notes, filterable by cargue, filing day and invoice number. The table is a preview of the first
 * rows; "Descargar CSV" is a plain link to the API with the same filters, which downloads all of them.
 * Every filter change queries again (latest answer wins); the search waits for the typing to pause.
 */
export class PageReportes extends LitElement {
  static properties = {
    cargues: { state: true },
    cargueError: { state: true },
    filters: { state: true },
    applied: { state: true },
    report: { state: true },
    loading: { state: true },
    loadError: { state: true },
  };

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-4);
    }
    h2 {
      margin: 0;
      font-size: 1.5rem;
      font-weight: 600;
      color: var(--tx-text-highlighted);
    }
    .lead {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .bar {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-2);
      font-size: var(--tx-text-sm);
      color: var(--tx-text);
    }
  `;

  cargues: Cargue[] = [];
  cargueError = "";
  /** What the controls show. */
  filters: ReportFilters = NO_FILTERS;
  /** What the table and the CSV link were queried with. */
  applied: ReportFilters = NO_FILTERS;
  report: RadicadasReport | null = null;
  loading = true;
  loadError = "";
  /** How long the search waits for the typing to pause. */
  searchDelayMs = 300;

  private requestId = 0;
  private timer: ReturnType<typeof setTimeout> | undefined;

  connectedCallback() {
    super.connectedCallback();
    document.title = "Reportes — Tessia";
    void this.loadCargues();
    void this.load();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    clearTimeout(this.timer);
    this.requestId++; // an answer still in flight must not touch a detached page
  }

  // --- loading -----------------------------------------------------------------------------

  /** The cargue list only feeds the filter: a failure leaves the report usable. */
  private async loadCargues() {
    try {
      this.cargues = await api.cargues();
      this.cargueError = "";
    } catch (e) {
      this.cargueError = errorMessage(e);
    }
  }

  /** Queries the report with the current filters; a newer query supersedes an older one. */
  private async load() {
    const id = ++this.requestId;
    const filters = this.filters;
    this.applied = filters;
    this.loading = true;
    this.loadError = "";
    try {
      const report = await api.radicadas(toQuery(filters));
      if (id === this.requestId) this.report = report;
    } catch (e) {
      if (id === this.requestId) this.loadError = errorMessage(e);
    } finally {
      if (id === this.requestId) this.loading = false;
    }
  }

  /** Applies a filter change: now, or after `delay` ms (typing). A range the API would refuse waits. */
  private change(next: ReportFilters, delay = 0) {
    this.filters = next;
    clearTimeout(this.timer);
    if (rangeError(next)) return;
    if (delay > 0) this.timer = setTimeout(() => void this.load(), delay);
    else void this.load();
  }

  // --- rendering ---------------------------------------------------------------------------

  private get csvUrl() {
    return rangeError(this.filters) ? "" : api.radicadasCsvUrl(toQuery(this.applied));
  }

  private renderReport(report: RadicadasReport) {
    return html`
      <div class="bar">
        <span data-testid="reportes-summary">${summaryText(report)}</span>
        ${report.truncado ? html`<tx-badge size="sm" variant="subtle" color="warning">Vista parcial</tx-badge>` : nothing}
      </div>
      <tx-reportes-tabla .rows=${report.filas}></tx-reportes-tabla>
    `;
  }

  protected render() {
    const f = this.filters;
    return html`
      <div>
        <h2>Reportes</h2>
        <div class="lead">
          Facturas radicadas (los cuatro pasos de radicación confirmados) con su fecha de radicación y sus observaciones. Filtra por cargue,
          fecha o factura y descarga el resultado completo en CSV.
        </div>
      </div>
      ${this.cargueError
        ? html`<tx-alert
            data-testid="cargues-error"
            color="warning"
            variant="subtle"
            icon="warning-triangle"
            heading="No se pudo cargar la lista de cargues"
            description=${this.cargueError}
          >
            <tx-button slot="actions" color="neutral" variant="outline" size="sm" @click=${() => void this.loadCargues()}>Reintentar</tx-button>
          </tx-alert>`
        : nothing}
      <tx-reportes-filtros
        .filters=${f}
        .cargueOptions=${this.cargues.map((c) => ({ value: c.id, label: c.nombre || c.id }))}
        .csvUrl=${this.csvUrl}
        .rangeError=${rangeError(f)}
        @filter-cargues=${(e: ValueEvent<string[]>) => this.change({ ...f, cargues: e.detail.value })}
        @filter-desde=${(e: ValueEvent<string>) => this.change({ ...f, desde: e.detail.value })}
        @filter-hasta=${(e: ValueEvent<string>) => this.change({ ...f, hasta: e.detail.value })}
        @filter-text=${(e: ValueEvent<string>) => this.change({ ...f, q: e.detail.value }, this.searchDelayMs)}
        @filter-clear=${() => this.change(NO_FILTERS)}
      ></tx-reportes-filtros>
      ${this.loadError
        ? html`<tx-alert
            data-testid="reportes-error"
            color="error"
            variant="subtle"
            icon="warning-circle"
            heading="No se pudo cargar el reporte"
            description=${this.loadError}
          >
            <tx-button slot="actions" color="neutral" variant="outline" size="sm" @click=${() => void this.load()}>Reintentar</tx-button>
          </tx-alert>`
        : nothing}
      ${this.loading ? html`<tx-progress indeterminate label="Consultando facturas radicadas"></tx-progress>` : nothing}
      ${this.report ? this.renderReport(this.report) : nothing}
    `;
  }
}

if (!customElements.get("tx-page-reportes")) customElements.define("tx-page-reportes", PageReportes);

declare global {
  interface HTMLElementTagNameMap {
    "tx-page-reportes": PageReportes;
  }
}
