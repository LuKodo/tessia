import { LitElement, css, html, nothing } from "lit";
import { FiltersBinding } from "../../controllers/rips-filters-controller";
import type { RipsFiltersController } from "../../controllers/rips-filters-controller";
import { ALERTS, PIPELINES, TEXT_FILTERS } from "../../utils/rips-constants";
import "../tx-badge";
import "../tx-button";
import "../tx-card";
import "../tx-checkbox";
import "../tx-form-field";
import "../tx-icon";
import "../tx-input";
import "../tx-select";
import { ripsIcon } from "./rips-tones";

// "Todos" travels as a sentinel value and maps back to an empty draft field.
const ALL = "todos";
const fromSelect = (v: unknown) => (v === ALL ? "" : String(v ?? ""));
const valueOf = (e: Event) => String((e as CustomEvent<{ value: unknown }>).detail.value ?? "");

/** Search card (text, date, estado and pipeline fields) and the alerts card with its counters. */
export class RipsFilters extends LitElement {
  static properties = {
    filters: { attribute: false },
    estadoOptions: { attribute: false },
    alertCounts: { attribute: false },
  };

  static styles = css`
    :host {
      display: block;
    }
    .layout {
      display: grid;
      grid-template-columns: 1fr;
      gap: 12px;
    }
    @media (min-width: 960px) {
      .layout {
        grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
      }
    }
    h2 {
      margin: 0 0 var(--tx-space-3);
      font-size: var(--tx-text-base);
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    h2.primary {
      margin: 0;
      color: var(--tx-primary);
    }
    /* Container-driven columns: the card is only ~60% wide, so viewport breakpoints would over-count. */
    .fields {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
      gap: 12px;
    }
    .selects {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
      gap: 12px;
      margin-top: var(--tx-space-4);
    }
    .select-row {
      display: flex;
      align-items: flex-end;
      gap: var(--tx-space-2);
    }
    .select-row tx-form-field {
      flex: 1;
      min-width: 0;
    }
    tx-select,
    tx-input {
      width: 100%;
    }
    .alerts-head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: var(--tx-space-3);
    }
    .buttons {
      display: flex;
      gap: var(--tx-space-2);
    }
    .alerts {
      display: grid;
      grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
      gap: 6px;
    }
    .alert-label {
      display: flex;
      width: 100%;
      align-items: center;
      justify-content: space-between;
      gap: 6px;
    }
    .alert-text {
      font-size: 0.75rem;
    }
    .alert-text.danger {
      color: var(--tx-error);
    }
    .active-count {
      margin-top: var(--tx-space-3);
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
  `;

  filters: RipsFiltersController | null = null;
  estadoOptions: string[] = [];
  alertCounts: Record<string, number> = {};

  private binding = new FiltersBinding(this);

  protected willUpdate(): void {
    this.binding.follow(this.filters);
  }

  render() {
    const f = this.filters;
    if (!f) return nothing;
    const estadoItems = [{ value: ALL, label: "Todos" }, ...this.estadoOptions.map((e) => ({ value: e, label: e.replace(/_/g, " ") }))];
    const pipelineItems = [{ value: ALL, label: "Todos" }, ...PIPELINES.map(([v, l]) => ({ value: v, label: l }))];
    return html`<div class="layout">
      <tx-card variant="subtle">
        <h2>Filtros de búsqueda</h2>
        <div class="fields">
          ${TEXT_FILTERS.map(
            ([key, label, placeholder, icon]) => html`<tx-form-field label=${label}>
              <tx-input
                .value=${f.draft.text[key]}
                placeholder=${placeholder}
                @tx-input=${(e: Event) => f.setText(key, valueOf(e))}
                @keydown=${(e: KeyboardEvent) => e.key === "Enter" && f.commit()}
              >
                <tx-icon slot="leading" name=${ripsIcon(icon)}></tx-icon>
              </tx-input>
            </tx-form-field>`,
          )}
          <tx-form-field label="Fecha desde">
            <tx-input type="date" .value=${f.draft.desde} @tx-input=${(e: Event) => f.setField("desde", valueOf(e))}></tx-input>
          </tx-form-field>
          <tx-form-field label="Fecha hasta">
            <tx-input type="date" .value=${f.draft.hasta} @tx-input=${(e: Event) => f.setField("hasta", valueOf(e))}></tx-input>
          </tx-form-field>
        </div>
        <div class="selects">
          <div class="select-row">
            <tx-form-field label="Estado">
              <tx-select .items=${estadoItems} .value=${f.draft.estado || ALL} @tx-change=${(e: Event) => f.setField("estado", fromSelect((e as CustomEvent).detail.value))}></tx-select>
            </tx-form-field>
            <tx-button
              icon="check"
              variant=${f.draft.estado ? "solid" : "outline"}
              title="Activar este filtro"
              aria-label="Activar el filtro de estado"
              @click=${() => f.commit()}
            ></tx-button>
          </div>
          <div class="select-row">
            <tx-form-field label="Pipeline">
              <tx-select .items=${pipelineItems} .value=${f.draft.pipeline || ALL} @tx-change=${(e: Event) => f.setField("pipeline", fromSelect((e as CustomEvent).detail.value))}></tx-select>
            </tx-form-field>
            <tx-button
              icon="check"
              variant=${f.draft.pipeline ? "solid" : "outline"}
              title="Activar este filtro"
              aria-label="Activar el filtro de pipeline"
              @click=${() => f.commit()}
            ></tx-button>
          </div>
        </div>
      </tx-card>

      <tx-card variant="subtle">
        <div class="alerts-head">
          <h2 class="primary">Alertas y pendientes</h2>
          <div class="buttons">
            <tx-button size="sm" variant="outline" @click=${() => f.clear()}>Limpiar</tx-button>
            <tx-button size="sm" @click=${() => f.commit()}>Filtrar</tx-button>
          </div>
        </div>
        <div class="alerts">
          ${ALERTS.map(
            ([code, label, danger]) => html`<tx-checkbox .checked=${f.draft.alerts.has(code)} @tx-change=${() => f.toggleAlert(code)}>
              <span slot="label" class="alert-label" data-testid="alert-label">
                <span class="alert-text ${danger ? "danger" : ""}">${label}</span>
                <tx-badge size="sm" variant="subtle" color="neutral" data-testid="alert-count">${this.alertCounts[code] || 0}</tx-badge>
              </span>
            </tx-checkbox>`,
          )}
        </div>
        ${f.activeCount > 0
          ? html`<div class="active-count">
              ${f.activeCount} filtro(s) activo(s) sobre la tabla de seguimiento${f.alertsMulti ? " · alertas: cualquiera de las marcadas" : ""}
            </div>`
          : nothing}
      </tx-card>
    </div>`;
  }
}

if (!customElements.get("rips-filters")) customElements.define("rips-filters", RipsFilters);
