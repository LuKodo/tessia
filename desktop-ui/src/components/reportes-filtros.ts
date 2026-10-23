import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import "./tx-button";
import "./tx-checkbox-group";
import "./tx-form-field";
import "./tx-icon";
import "./tx-input";
import { hasFilters, NO_FILTERS } from "./reportes-model";
import type { ReportFilters } from "./reportes-model";

export interface CargueOption {
  value: string;
  label: string;
}

type ValueEvent<T> = CustomEvent<{ value: T }>;

/**
 * Presentational filter bar of the "Reportes" page: it renders the current filters and reports each
 * change; the page owns the state. Non-bubbling `CustomEvent`s: `filter-text`, `filter-desde` and
 * `filter-hasta` (`detail: { value: string }`), `filter-cargues` (`detail: { value: string[] }`, a new
 * array; none checked means every cargue) and `filter-clear`.
 *
 * The inner `tx-input` / `tx-change` events of the controls bubble and are composed, so each handler
 * stops them here: a listener of an ancestor must not mistake them for its own controls' changes.
 */
export class ReportesFiltros extends LitElement {
  static properties = {
    filters: { attribute: false },
    cargueOptions: { attribute: false },
    csvUrl: { attribute: "csv-url" },
    rangeError: { attribute: "range-error" },
  };

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-3);
    }
    .row {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-end;
      gap: var(--tx-space-3);
    }
    .search {
      flex: 2 1 260px;
      min-width: 220px;
    }
    .date {
      flex: 0 1 11rem;
      min-width: 9rem;
    }
    .grow {
      flex-grow: 1;
    }
    fieldset.cargues {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-2);
      min-width: 0;
      margin: 0;
      padding: var(--tx-space-3);
      border: 1px solid var(--tx-border);
      border-radius: var(--tx-radius);
    }
    fieldset.cargues legend {
      padding: 0 var(--tx-space-1);
      font-size: var(--tx-text-sm);
      font-weight: 500;
      color: var(--tx-text-highlighted);
    }
    .hint {
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
    .scroll {
      max-height: 8.5rem;
      overflow: auto;
    }
    .range-error {
      font-size: var(--tx-text-sm);
      color: var(--tx-error);
    }
    /* A native anchor: tx-button has no \`download\` attribute. Same look as an outlined neutral button. */
    a.csv {
      display: inline-flex;
      align-items: center;
      gap: 0.375rem;
      box-sizing: border-box;
      min-height: 2.5rem;
      padding: 0 0.875rem;
      border: 1px solid var(--tx-border-strong);
      border-radius: var(--tx-radius);
      color: var(--tx-text);
      font-size: var(--tx-text-sm);
      font-weight: 500;
      text-decoration: none;
      white-space: nowrap;
      transition: background-color 120ms;
    }
    a.csv:hover {
      background: var(--tx-bg-muted);
    }
    a.csv:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
    a.csv[aria-disabled="true"] {
      opacity: 0.5;
      cursor: not-allowed;
    }
    a.csv[aria-disabled="true"]:hover {
      background: transparent;
    }
    a.csv tx-icon {
      font-size: 1.25em;
    }
  `;

  filters: ReportFilters = NO_FILTERS;
  cargueOptions: CargueOption[] = [];
  /** Link of the full CSV for the current filters; "" disables the download. */
  csvUrl = "";
  /** Why the date range is not valid ("" when it is); shown next to the dates. */
  rangeError = "";

  private emit<T>(name: string, detail: T) {
    this.dispatchEvent(new CustomEvent(name, { detail }));
  }

  /** Handler of a control's `tx-input` / `tx-change`: keeps the event inside and reports ours. */
  private on<T>(name: string, read: (e: ValueEvent<T>) => T = (e) => e.detail.value) {
    return (e: Event) => {
      e.stopPropagation();
      this.emit(name, { value: read(e as ValueEvent<T>) });
    };
  }

  protected render() {
    const f = this.filters;
    return html`
      <div class="row">
        <tx-form-field class="search" label="Buscar">
          <tx-input
            block
            data-testid="reportes-search"
            placeholder="Número de factura"
            aria-label="Buscar por número de factura"
            .value=${f.q}
            @tx-input=${this.on<string>("filter-text")}
          >
            <tx-icon slot="leading" name="search"></tx-icon>
          </tx-input>
        </tx-form-field>
        <tx-form-field class="date" label="Radicada desde">
          <tx-input block type="date" data-testid="reportes-desde" .value=${f.desde} @tx-input=${this.on<string>("filter-desde")}></tx-input>
        </tx-form-field>
        <tx-form-field class="date" label="Radicada hasta">
          <tx-input block type="date" data-testid="reportes-hasta" .value=${f.hasta} @tx-input=${this.on<string>("filter-hasta")}></tx-input>
        </tx-form-field>
        <div class="grow"></div>
        ${hasFilters(f)
          ? html`<tx-button data-testid="reportes-clear" color="neutral" variant="outline" icon="xmark" @click=${() => this.emit("filter-clear", {})}
              >Limpiar filtros</tx-button
            >`
          : nothing}
        <a
          class="csv"
          data-testid="reportes-csv"
          href=${ifDefined(this.csvUrl || undefined)}
          aria-disabled=${this.csvUrl ? "false" : "true"}
        >
          <tx-icon name="download"></tx-icon>Descargar CSV
        </a>
      </div>
      ${this.rangeError ? html`<div class="range-error" data-testid="range-error" role="alert">${this.rangeError}</div>` : nothing}
      ${this.cargueOptions.length > 0
        ? html`<fieldset class="cargues">
            <legend>Cargues</legend>
            <span class="hint">Sin marcar ninguno se incluyen todos los cargues.</span>
            <div class="scroll">
              <tx-checkbox-group
                orientation="horizontal"
                aria-label="Cargues a incluir"
                .items=${this.cargueOptions}
                .value=${f.cargues}
                @tx-change=${this.on<string[]>("filter-cargues")}
                @tx-input=${(e: Event) => e.stopPropagation()}
              ></tx-checkbox-group>
            </div>
          </fieldset>`
        : nothing}
    `;
  }
}

if (!customElements.get("tx-reportes-filtros")) customElements.define("tx-reportes-filtros", ReportesFiltros);

declare global {
  interface HTMLElementTagNameMap {
    "tx-reportes-filtros": ReportesFiltros;
  }
}
