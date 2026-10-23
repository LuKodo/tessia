import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { FiltersBinding } from "../../controllers/rips-filters-controller";
import type { RipsFiltersController } from "../../controllers/rips-filters-controller";
import type { Kpis } from "../../types/api";
import { KPIS, KPI_FILTER } from "../../utils/rips-constants";
import type { KpiKey } from "../../utils/rips-constants";
import "../tx-card";
import "../tx-icon";
import "../tx-progress";
import { KPI_HUE, ripsIcon, spinStyles } from "./rips-tones";

/** The ten KPI cards. The ones that map to a filter are keyboard buttons that toggle it. */
export class RipsKpis extends LitElement {
  static properties = {
    counts: { attribute: false },
    filters: { attribute: false },
    loading: { type: Boolean },
  };

  static styles = [
    spinStyles,
    css`
      :host {
        display: block;
      }
      .grid {
        display: grid;
        grid-template-columns: 1fr;
        gap: 12px;
      }
      @media (min-width: 640px) {
        .grid {
          grid-template-columns: 1fr 1fr;
        }
      }
      .kpi {
        position: relative;
        cursor: default;
        background: color-mix(in oklab, var(--hue) 9%, var(--tx-bg-elevated));
      }
      .kpi.clickable {
        cursor: pointer;
      }
      .kpi:focus-visible {
        outline: var(--tx-focus-ring);
        outline-offset: 2px;
      }
      /* Active state: an overlay ring tinted with the hue of the card. */
      .kpi__ring {
        position: absolute;
        inset: -1px;
        border: 1px solid var(--hue);
        border-radius: inherit;
        box-shadow: 0 0 0 2px color-mix(in srgb, var(--hue) 30%, transparent);
        pointer-events: none;
      }
      .kpi__row {
        display: grid;
        grid-template-columns: auto 1fr auto;
        align-items: center;
        column-gap: 10px;
      }
      .kpi__icon {
        display: inline-grid;
        place-items: center;
        width: 30px;
        height: 30px;
        border-radius: var(--tx-radius-lg);
        background: var(--tx-bg-elevated);
        color: var(--hue);
        font-size: 1rem;
      }
      .label {
        font-size: var(--tx-text-sm);
        color: var(--tx-text-highlighted);
      }
      .sub {
        font-size: 0.75rem;
        color: var(--tx-text-muted);
      }
      .count {
        font-size: var(--tx-text-lg);
        font-weight: 700;
        color: var(--tx-text-highlighted);
      }
      .spinner {
        color: var(--hue);
        font-size: 1.25rem;
      }
      tx-progress {
        margin-top: var(--tx-space-2);
        --_solid: var(--hue);
      }
    `,
  ];

  counts: Kpis | null | undefined = null;
  filters: RipsFiltersController | null = null;
  loading = false;

  private binding = new FiltersBinding(this);

  protected willUpdate(): void {
    this.binding.follow(this.filters);
  }

  private activate(key: KpiKey) {
    if (KPI_FILTER[key]) this.filters?.clickKPI(key);
  }

  private onKey(e: KeyboardEvent, key: KpiKey) {
    if (e.key !== "Enter" && e.key !== " ") return;
    e.preventDefault();
    this.activate(key);
  }

  render() {
    const total = this.counts?.total || 0;
    const loading = this.loading && !this.counts;
    return html`<div class="grid">
      ${KPIS.map(([key, label, sub, icon, color]) => {
        const count = this.counts?.[key] || 0;
        const pct = total ? Math.round((100 * count) / total) : 0;
        const clickable = !!KPI_FILTER[key];
        const active = this.filters?.isKPIActive(key) ?? false;
        return html`<tx-card
          class="kpi ${clickable ? "clickable" : ""}"
          padding="sm"
          style="--hue: ${KPI_HUE[color]}"
          role=${ifDefined(clickable ? "button" : undefined)}
          tabindex=${ifDefined(clickable ? "0" : undefined)}
          aria-pressed=${ifDefined(clickable ? String(active) : undefined)}
          @click=${() => this.activate(key)}
          @keydown=${(e: KeyboardEvent) => this.onKey(e, key)}
        >
          ${active ? html`<span class="kpi__ring"></span>` : nothing}
          <div class="kpi__row">
            <span class="kpi__icon"><tx-icon name=${ripsIcon(icon)}></tx-icon></span>
            <div>
              <div class="label">${label}</div>
              ${sub ? html`<div class="sub">(${sub})</div>` : nothing}
            </div>
            ${loading
              ? html`<tx-icon class="spinner spin" name="refresh" label="Cargando" data-testid="kpi-spinner"></tx-icon>`
              : html`<div class="count" data-testid="kpi-count">${count}</div>`}
          </div>
          <tx-progress
            size="xs"
            .value=${loading ? null : pct}
            label=${label}
            value-text=${loading ? "Cargando" : `${pct}% del total`}
          ></tx-progress>
        </tx-card>`;
      })}
    </div>`;
  }
}

if (!customElements.get("rips-kpis")) customElements.define("rips-kpis", RipsKpis);
