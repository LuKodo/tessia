import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import "./tx-button";
import "./tx-form-field";
import "./tx-icon";
import "./tx-input";
import "./tx-select";
import "./tx-switch";

export interface FilterOption {
  value: string;
  label: string;
}

/**
 * Presentational filter bar of the manual review page: it renders the current filters and reports
 * each change; the page owns the state. A change is a non-bubbling `CustomEvent` with `detail:
 * { value }`: `filter-text`, `filter-cargue`, `filter-fase`, `filter-categoria` and `filter-others`
 * (boolean). "" always means "no filter".
 */
export class RevisionFiltros extends LitElement {
  static properties = {
    text: {},
    cargue: {},
    fase: {},
    categoria: {},
    includeOthers: { type: Boolean, attribute: "include-others" },
    cargueOptions: { attribute: false },
    faseOptions: { attribute: false },
    categoriaOptions: { attribute: false },
    csvUrl: { attribute: "csv-url" },
    csvName: { attribute: "csv-name" },
  };

  static styles = css`
    :host {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-end;
      gap: var(--tx-space-3);
    }
    .search {
      flex: 2 1 260px;
      min-width: 220px;
    }
    .cargue {
      flex: 1 1 200px;
      min-width: 180px;
    }
    .fase,
    .categoria {
      flex: 1 1 160px;
      min-width: 140px;
    }
    tx-switch {
      display: inline-flex;
      align-items: center;
      min-height: 2.5rem;
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

  text = "";
  cargue = "";
  fase = "";
  categoria = "";
  includeOthers = false;
  cargueOptions: FilterOption[] = [];
  faseOptions: FilterOption[] = [];
  categoriaOptions: FilterOption[] = [];
  /** CSV link of the selected cargue; "" when no single cargue is selected. */
  csvUrl = "";
  csvName = "";

  private change(name: string, value: string | boolean) {
    this.dispatchEvent(new CustomEvent(name, { detail: { value } }));
  }

  private onSelect(name: string) {
    return (e: CustomEvent<{ value: unknown }>) => this.change(name, String(e.detail.value ?? ""));
  }

  protected render() {
    const withAll = (options: FilterOption[], label: string) => [{ value: "", label }, ...options];
    return html`
      <tx-form-field class="search" label="Buscar">
        <tx-input
          block
          data-testid="revision-search"
          placeholder="Factura, código o motivo"
          aria-label="Buscar por factura, código o motivo"
          .value=${this.text}
          @tx-input=${(e: CustomEvent<{ value: string }>) => this.change("filter-text", e.detail.value)}
        >
          <tx-icon slot="leading" name="search"></tx-icon>
          ${this.text
            ? html`<tx-button
                slot="trailing"
                color="neutral"
                variant="link"
                size="sm"
                icon="xmark"
                aria-label="Limpiar búsqueda"
                @click=${() => this.change("filter-text", "")}
              ></tx-button>`
            : nothing}
        </tx-input>
      </tx-form-field>
      <tx-form-field class="cargue" label="Cargue">
        <tx-select
          block
          data-testid="revision-cargue"
          .items=${withAll(this.cargueOptions, "(todos)")}
          .value=${this.cargue}
          @tx-change=${this.onSelect("filter-cargue")}
        ></tx-select>
      </tx-form-field>
      <tx-form-field class="fase" label="Fase">
        <tx-select
          block
          data-testid="revision-fase"
          .items=${withAll(this.faseOptions, "(todas)")}
          .value=${this.fase}
          @tx-change=${this.onSelect("filter-fase")}
        ></tx-select>
      </tx-form-field>
      <tx-form-field class="categoria" label="Categoría">
        <tx-select
          block
          data-testid="revision-categoria"
          .items=${withAll(this.categoriaOptions, "(todas)")}
          .value=${this.categoria}
          @tx-change=${this.onSelect("filter-categoria")}
        ></tx-select>
      </tx-form-field>
      <tx-switch
        data-testid="revision-others"
        label="Incluir excluidas y fallas"
        .checked=${this.includeOthers}
        @tx-change=${(e: CustomEvent<{ checked: boolean }>) => this.change("filter-others", e.detail.checked)}
      ></tx-switch>
      <a
        class="csv"
        data-testid="informe-csv"
        href=${ifDefined(this.csvUrl || undefined)}
        download=${ifDefined(this.csvUrl ? this.csvName : undefined)}
        aria-disabled=${this.csvUrl ? "false" : "true"}
      >
        <tx-icon name="download"></tx-icon>Descargar CSV
      </a>
    `;
  }
}

if (!customElements.get("tx-revision-filtros")) customElements.define("tx-revision-filtros", RevisionFiltros);

declare global {
  interface HTMLElementTagNameMap {
    "tx-revision-filtros": RevisionFiltros;
  }
}
