import { LitElement, css, html } from "lit";
import { api } from "../../api";
import { PollController } from "../../controllers/poll-controller";
import { RipsFiltersController } from "../../controllers/rips-filters-controller";
import type { SeguimientoConsulta, Summary, TrackingRow } from "../../types/api";
import { errorMessage } from "../../utils/error-message";
import { monthOf, monthRange } from "../../utils/month-range";
import { PAGE_SIZE, loadMore, loadRange } from "./consulta-pager";
import type { FetchPage, Page } from "./consulta-pager";
import "./detalle-modal";
import "./portal-modal";
import "./rips-chart";
import "./rips-filters";
import "./rips-kpis";
import "./rips-table";
import { stabilizeRows } from "./stabilize-rows";

const rowKey = (r: Pick<TrackingRow, "cargue" | "factura">) => (r.cargue || "") + "/" + r.factura;

/**
 * "Analizar" tab: cargue chart, KPI cards, filters, tracking table and the detail and portal modals.
 *
 * The server does the work (GET /api/seguimiento/consulta): this element only sends the applied
 * filters and the sort and renders the answers. Two requests go out together every 10 s: the rows
 * (`agregados=no`; the loaded range, one page at first, more after "Cargar más") and the aggregates
 * (`limit=0`: total, KPI cards and filter facets). Changing the filters or the sort starts again
 * from the first page.
 */
export class RipsDashboard extends LitElement {
  static properties = { pageSize: { attribute: false }, detalle: { state: true }, portalFactura: { state: true }, portalOpen: { state: true }, loadingMore: { state: true } };

  static styles = css`
    :host {
      display: block;
    }
    .column {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-4);
      padding: var(--tx-space-4);
    }
  `;

  /** Rows per page; the server's default. */
  pageSize = PAGE_SIZE;

  private filters = new RipsFiltersController(this, () => this.restart());
  // How many rows the table holds, so a poll refetches what the user loaded and not just page one.
  // Null: one page. Set again when the filters or the sort change.
  private target: number | null = null;
  // Set by "Cargar más": the next fetch of the rows appends to this page instead of starting over.
  private appendTo: Page | null = null;
  private rows = new PollController<Page>(this, () => this.fetchRows(), 10000, { pauseWhenHidden: true });
  private aggregates = new PollController<SeguimientoConsulta>(this, () => api.seguimientoConsulta({ ...this.filters.query, limit: 0 }), 10000, {
    pauseWhenHidden: true,
  });
  // The chart lists whole cargues, whatever the filters; it keeps reading the month summary.
  private summary = new PollController<Summary>(this, () => api.resumen(monthRange(monthOf(new Date()))), 30000, { pauseWhenHidden: true });

  detalle: string | null = null;
  portalFactura = "";
  portalOpen = false;
  loadingMore = false;

  private async fetchRows(): Promise<Page> {
    const query = this.filters.query;
    const fetchPage: FetchPage = (req) => api.seguimientoConsulta({ ...query, ...req, agregados: false });
    const base = this.appendTo;
    this.appendTo = null;
    const page = base ? await loadMore(fetchPage, base, this.pageSize) : await loadRange(fetchPage, this.target ?? this.pageSize);
    return { ...page, rows: stabilizeRows(this.rows.data?.rows, page.rows) };
  }

  // The applied filters or the sort changed: back to the first page, rows and aggregates.
  private restart(): void {
    this.target = null;
    this.appendTo = null;
    void this.rows.refresh();
    void this.aggregates.refresh();
  }

  private async loadMore(): Promise<void> {
    const page = this.rows.data;
    if (!page?.next || this.loadingMore) return;
    this.appendTo = page;
    // A refetch from the start (a poll that supersedes this one) must cover the new page as well.
    this.target = page.rows.length + this.pageSize;
    this.loadingMore = true;
    try {
      await this.rows.refresh();
    } finally {
      this.loadingMore = false;
    }
  }

  private openPortal = (e: Event) => {
    this.portalFactura = (e as CustomEvent<TrackingRow>).detail.factura;
    this.portalOpen = true;
  };

  render() {
    const page = this.rows.data;
    const loaded = page?.rows || [];
    const f = this.filters;
    const answer = this.aggregates.data;
    // Keep the open modal's row live: re-find it by key on every poll instead of freezing a snapshot.
    const liveDetalle = this.detalle ? loaded.find((r) => rowKey(r) === this.detalle) || null : null;
    const failure = this.rows.error || this.aggregates.error;
    return html`<div class="column">
      <rips-chart
        .summary=${this.summary.data}
        .filters=${f}
        .error=${this.summary.error ? errorMessage(this.summary.error) : ""}
        .loading=${this.summary.loading}
      ></rips-chart>
      <rips-kpis .counts=${answer?.kpis} .filters=${f} .loading=${this.aggregates.loading}></rips-kpis>
      <rips-filters .filters=${f} .estadoOptions=${answer?.facets?.estados || []} .alertCounts=${answer?.facets?.alertas || {}}></rips-filters>
      <rips-table
        .rows=${loaded}
        .total=${answer?.total}
        .filtered=${f.activeCount > 0}
        .hasMore=${!!page?.next}
        .loading=${this.rows.loading}
        .loadingMore=${this.loadingMore}
        .error=${failure ? errorMessage(failure) : ""}
        .sort=${f.sort}
        @sort-change=${(e: Event) => f.sortBy((e as CustomEvent<string>).detail)}
        @cargar-mas=${() => void this.loadMore()}
        @ver-detalle=${(e: Event) => (this.detalle = rowKey((e as CustomEvent<TrackingRow>).detail))}
        @ver-portal=${this.openPortal}
      ></rips-table>
      <detalle-modal .row=${liveDetalle} @close=${() => (this.detalle = null)}></detalle-modal>
      <!-- A fetch/refresh may store a new portal result: re-poll the rows so the table icon updates now. -->
      <portal-modal
        .open=${this.portalOpen}
        .factura=${this.portalFactura}
        @loaded=${() => void this.rows.refresh()}
        @close=${() => (this.portalOpen = false)}
      ></portal-modal>
    </div>`;
  }
}

if (!customElements.get("rips-dashboard")) customElements.define("rips-dashboard", RipsDashboard);
