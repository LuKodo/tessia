import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import "../components/revision-filtros";
import "../components/revision-tabla";
import "../components/tx-alert";
import "../components/tx-badge";
import "../components/tx-button";
import "../components/tx-progress";
import { DialogsController } from "../controllers/dialogs-controller";
import type { Cargue } from "../types/api";
import { errorMessage } from "../utils/error-message";
import { categoryColor, categoryLabel } from "../utils/informe-model";
import { runInvoiceRetry } from "../utils/retry-model";
import {
  allVisibleState,
  cargueScope,
  categoryOptions,
  filterRows,
  isRetryable,
  mergeReports,
  phaseOptions,
  retryableRows,
  rowKey,
  runBatchRetry,
  toggleAllVisible,
  toggleRow,
} from "../utils/revision-batch-model";
import type { CargueReport, RevisionRow } from "../utils/revision-batch-model";

type FilterEvent = CustomEvent<{ value: string }>;

/**
 * "Revisión manual": the reports of every cargue merged into one table, with filters, row selection
 * and retry of one invoice or of the selected ones (sequentially, after a single confirmation).
 * `?cargue=` preselects a cargue, as linked from the process panel.
 */
export class PageRevisionManual extends LitElement {
  static properties = {
    cargues: { state: true },
    reports: { state: true },
    loading: { state: true },
    loadError: { state: true },
    text: { state: true },
    cargue: { state: true },
    fase: { state: true },
    categoria: { state: true },
    includeOthers: { state: true },
    selected: { state: true },
    retrying: { state: true },
    retryStatus: { state: true },
    retryError: { state: true },
    retryOutput: { state: true },
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
    }
    .grow {
      flex-grow: 1;
    }
    .multiline {
      white-space: pre-line;
    }
    pre.retry-output {
      max-height: 11rem;
      margin: 0;
      overflow: auto;
      font-family: var(--tx-font-mono);
      font-size: 0.75rem;
      color: var(--tx-text);
      overflow-wrap: anywhere;
      white-space: pre-wrap;
    }
  `;

  cargues: Cargue[] = [];
  reports: CargueReport[] = [];
  loading = true;
  loadError = "";
  text = "";
  /** Preselected by the "Informe de revisión manual" link of the process panel (`?cargue=`). */
  cargue = new URLSearchParams(window.location.search).get("cargue") ?? "";
  fase = "";
  categoria = "";
  includeOthers = false;
  selected: ReadonlySet<string> = new Set();
  retrying = false;
  retryStatus = "";
  retryError = "";
  retryOutput = "";

  private dialogs = new DialogsController(this);
  private requestId = 0;

  connectedCallback() {
    super.connectedCallback();
    document.title = "Revisión manual — Tessia";
    void this.reload();
  }

  disconnectedCallback() {
    super.disconnectedCallback();
    this.requestId++; // a load still in flight must not touch a detached page
  }

  // --- derived state -----------------------------------------------------------------------

  private get allRows() {
    return mergeReports(this.reports);
  }

  private get rows() {
    return filterRows(this.allRows, {
      text: this.text,
      cargue: this.cargue,
      fase: this.fase,
      categoria: this.categoria,
      includeOthers: this.includeOthers,
    });
  }

  // --- loading -----------------------------------------------------------------------------

  /**
   * Loads the cargue list and the report of each one. A cargue whose report fails keeps its error
   * and does not hide the others.
   */
  async reload() {
    const id = ++this.requestId;
    this.loadError = "";
    try {
      const list = await api.cargues();
      const loaded = await Promise.all(
        list.map(async (c): Promise<CargueReport> => {
          try {
            return { cargue: c.id, report: await api.informe(c.id) };
          } catch (e) {
            return { cargue: c.id, report: null, error: errorMessage(e) };
          }
        }),
      );
      if (id !== this.requestId) return;
      this.cargues = list;
      this.reports = loaded;
      // Drop what is no longer retryable (the retried invoices) and an unknown ?cargue=.
      const keys = new Set(retryableRows(mergeReports(loaded), loaded).map(rowKey));
      this.selected = new Set([...this.selected].filter((k) => keys.has(k)));
      if (this.cargue && !list.some((c) => c.id === this.cargue)) this.cargue = "";
    } catch (e) {
      if (id === this.requestId) this.loadError = errorMessage(e);
    } finally {
      if (id === this.requestId) this.loading = false;
    }
  }

  private async refresh() {
    await this.reload();
    if (this.loadError) throw new Error(this.loadError);
  }

  // --- retries -----------------------------------------------------------------------------

  private retryUi = {
    isBusy: () => this.retrying,
    setBusy: (value: boolean) => {
      this.retrying = value;
    },
    currentJob: () => api.jobsCurrent(),
    history: () => api.jobsHistory(),
    setStatus: (value: string) => {
      this.retryStatus = value;
    },
    setError: (value: string) => {
      this.retryError = value;
    },
    setOutput: (value: string) => {
      this.retryOutput = value;
    },
  };

  private retryRow(row: RevisionRow) {
    return runInvoiceRetry(row, {
      ...this.retryUi,
      isEligible: () => isRetryable(row, this.reports),
      confirm: (invoice) =>
        this.dialogs.confirmAction(
          `Se volverá a procesar ${invoice.factura}: puede contactar Medicar/Ministerio y actualizar archivos y datos locales.`,
          { title: "Reintentar factura", confirmText: "Sí, reintentar" },
        ),
      submit: (invoice) => api.retryInvoice(row.cargue, invoice.factura ?? ""),
      refresh: () => this.refresh(),
    });
  }

  private retrySelected(selectedRows: RevisionRow[]) {
    return runBatchRetry(
      selectedRows.map((row) => ({ cargue: row.cargue, item: row })),
      {
        ...this.retryUi,
        isEligible: ({ cargue: id, item }) => isRetryable({ ...(item as RevisionRow), cargue: id }, this.reports),
        confirm: (count) =>
          this.dialogs.confirmAction(
            `Se volverán a procesar ${count} ${count === 1 ? "factura" : "facturas"}, una tras otra: puede contactar Medicar/Ministerio y actualizar archivos y datos locales.`,
            { title: "Reintentar facturas seleccionadas", confirmText: "Sí, reintentar" },
          ),
        submit: (id, factura) => api.retryInvoice(id, factura),
        refresh: () => this.refresh(),
      },
    );
  }

  // --- rendering ---------------------------------------------------------------------------

  private renderLoadError() {
    return html`<tx-alert color="error" variant="subtle" icon="warning-circle" heading="No se pudo cargar el informe" description=${this.loadError}>
      <tx-button slot="actions" color="neutral" variant="outline" size="sm" @click=${() => void this.reload()}>Reintentar</tx-button>
    </tx-alert>`;
  }

  private renderReport() {
    const rows = this.rows;
    const reports = this.reports;
    const retryable = new Set(retryableRows(rows, reports).map(rowKey));
    const selectedRows = rows.filter((row) => retryable.has(rowKey(row)) && this.selected.has(rowKey(row)));
    const scope = cargueScope(reports, this.cargue);
    const chips = [{ categoria: "revision_manual", value: scope.conteos.revision_manual }];
    if (this.includeOthers) {
      chips.push({ categoria: "excluida", value: scope.conteos.excluidas }, { categoria: "falla", value: scope.conteos.fallas });
    }
    // Absolute URL on the API base: the CSV is served by `tessia serve`, not by this app.
    const csvUrl = this.cargue ? api.informeCsvUrl(this.cargue) : "";

    return html`
      <tx-revision-filtros
        .text=${this.text}
        .cargue=${this.cargue}
        .fase=${this.fase}
        .categoria=${this.categoria}
        .includeOthers=${this.includeOthers}
        .cargueOptions=${this.cargues.map((c) => ({ value: c.id, label: c.nombre || c.id }))}
        .faseOptions=${phaseOptions(this.allRows)}
        .categoriaOptions=${categoryOptions(this.includeOthers)}
        .csvUrl=${csvUrl}
        .csvName=${`informe-revision-manual-${this.cargue}.csv`}
        @filter-text=${(e: FilterEvent) => (this.text = e.detail.value)}
        @filter-cargue=${(e: FilterEvent) => (this.cargue = e.detail.value)}
        @filter-fase=${(e: FilterEvent) => (this.fase = e.detail.value)}
        @filter-categoria=${(e: FilterEvent) => (this.categoria = e.detail.value)}
        @filter-others=${(e: CustomEvent<{ value: boolean }>) => (this.includeOthers = e.detail.value)}
      ></tx-revision-filtros>

      <div class="bar">
        ${chips.map((c) => html`<tx-badge size="md" variant="subtle" color=${categoryColor(c.categoria)}>${categoryLabel(c.categoria)}: ${c.value}</tx-badge>`)}
        <div class="grow"></div>
        <tx-button
          data-testid="retry-selected"
          icon="refresh"
          ?disabled=${this.retrying || selectedRows.length === 0}
          ?loading=${this.retrying}
          @click=${() => void this.retrySelected(selectedRows)}
          >Reintentar seleccionadas (${selectedRows.length})</tx-button
        >
      </div>

      ${scope.errors.map(
        (e) => html`<tx-alert
          data-testid="cargue-error"
          color="error"
          variant="subtle"
          icon="warning-circle"
          heading=${`No se pudo cargar el informe de ${e.cargue}`}
          description=${e.error}
        ></tx-alert>`,
      )}
      ${scope.incompleto
        ? html`<tx-alert color="warning" variant="subtle" icon="warning-triangle" heading="Base de datos no disponible: solo se muestran los datos del convenio"></tx-alert>`
        : nothing}

      <tx-revision-tabla
        .rows=${rows}
        .retryable=${retryable}
        .selected=${this.selected}
        .allState=${allVisibleState(this.selected, rows, reports)}
        .retrying=${this.retrying}
        @row-select=${(e: CustomEvent<{ key: string; checked: boolean }>) => (this.selected = toggleRow(this.selected, e.detail.key, e.detail.checked))}
        @row-select-all=${(e: CustomEvent<{ checked: boolean }>) => (this.selected = toggleAllVisible(this.selected, rows, reports, e.detail.checked))}
        @row-retry=${(e: CustomEvent<{ row: RevisionRow }>) => void this.retryRow(e.detail.row)}
      ></tx-revision-tabla>

      ${this.retryStatus ? html`<tx-alert color="info" variant="subtle" icon="info-circle" heading=${this.retryStatus}></tx-alert>` : nothing}
      ${this.retryError
        ? html`<tx-alert class="multiline" color="error" variant="subtle" icon="warning-circle" heading=${this.retryError}></tx-alert>`
        : nothing}
      ${this.retryOutput ? html`<pre class="retry-output">${this.retryOutput}</pre>` : nothing}
    `;
  }

  protected render() {
    return html`
      <div>
        <h2>Revisión manual</h2>
        <div class="lead">
          Facturas que requieren revisión manual en todos los cargues. Reintenta una factura o varias seleccionadas: se procesan una tras otra.
        </div>
      </div>
      ${this.loadError
        ? this.renderLoadError()
        : this.loading
          ? html`<tx-progress indeterminate label="Cargando el informe"></tx-progress>`
          : this.renderReport()}
    `;
  }
}

if (!customElements.get("tx-page-revision-manual")) customElements.define("tx-page-revision-manual", PageRevisionManual);

declare global {
  interface HTMLElementTagNameMap {
    "tx-page-revision-manual": PageRevisionManual;
  }
}
