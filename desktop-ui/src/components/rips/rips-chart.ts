import { LitElement, css, html, nothing } from "lit";
import { FiltersBinding } from "../../controllers/rips-filters-controller";
import type { RipsFiltersController } from "../../controllers/rips-filters-controller";
import type { Summary } from "../../types/api";
import { buildBars, tooltipOf, visibleCargues } from "../../utils/cargues-chart-model";
import "../tx-alert";
import "../tx-button";
import "../tx-card";
import "../tx-icon";
import "./rips-chart-bars";
import type { BarHoverDetail } from "./rips-chart-bars";
import { chartPalette, ripsIcon, spinStyles } from "./rips-tones";

const LEGEND = [
  { swatch: "--seg-filed", label: "Radicadas" },
  { swatch: "--seg-managed", label: "Gestionadas (sin radicar)" },
  { swatch: "--seg-manual", label: "Revisión manual" },
  { swatch: "--seg-excluded", label: "Excluidas" },
];

/** "Avance de radicación por cargue": one stacked bar per cargue; a click on a bar filters by that cargue. */
export class RipsChart extends LitElement {
  static properties = {
    summary: { attribute: false },
    filters: { attribute: false },
    error: { type: String },
    loading: { type: Boolean },
    width: { state: true },
    showAll: { state: true },
    tip: { state: true },
  };

  static styles = [
    chartPalette,
    spinStyles,
    css`
      :host {
        display: block;
      }
      .head {
        display: flex;
        flex-wrap: wrap;
        align-items: flex-end;
        justify-content: space-between;
        gap: var(--tx-space-3);
        margin-bottom: var(--tx-space-2);
      }
      .title {
        display: flex;
        align-items: center;
        gap: var(--tx-space-3);
      }
      .title tx-icon {
        font-size: 18px;
      }
      h2 {
        margin: 0;
        font-size: var(--tx-text-base);
        font-weight: 700;
        color: var(--tx-text-highlighted);
      }
      .subtitle {
        font-size: 0.75rem;
        color: var(--tx-text-muted);
      }
      .error {
        margin-bottom: 0.25rem;
        font-size: 0.75rem;
        color: var(--tx-error);
      }
      .box {
        position: relative;
      }
      .center {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: var(--tx-space-3);
        padding: 2.5rem 0;
        font-size: var(--tx-text-sm);
        color: var(--tx-text-muted);
        text-align: center;
      }
      .center tx-icon {
        font-size: 1.5rem;
      }
      .tip {
        position: absolute;
        z-index: 3;
        padding: var(--tx-space-2);
        border-radius: var(--tx-radius-lg);
        background: var(--tx-bg-inverted);
        color: var(--tx-text-inverted);
        font-size: 0.75rem;
        white-space: nowrap;
        pointer-events: none;
      }
      .tip b {
        display: block;
        margin-bottom: 0.25rem;
      }
      .legend {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--tx-space-6);
        margin-top: 0.25rem;
      }
      .legend div {
        display: flex;
        align-items: center;
        gap: var(--tx-space-2);
        font-size: 0.75rem;
      }
      .swatch {
        display: inline-block;
        width: 34px;
        height: 11px;
        border-radius: 2px;
      }
      tx-alert {
        display: block;
        margin-top: var(--tx-space-3);
        font-size: 0.75rem;
      }
    `,
  ];

  summary: Summary | null | undefined = null;
  filters: RipsFiltersController | null = null;
  error = "";
  loading = false;
  width = 600;
  showAll = false;
  tip: { i: number; left: number; top: number } | null = null;

  private binding = new FiltersBinding(this);
  private observer: ResizeObserver | undefined;

  protected willUpdate(): void {
    this.binding.follow(this.filters);
  }

  protected firstUpdated(): void {
    const box = this.renderRoot.querySelector(".box");
    if (!box || typeof ResizeObserver === "undefined") return;
    this.observer = new ResizeObserver((entries) => {
      this.width = Math.max(320, entries[0]?.contentRect.width ?? 0);
    });
    this.observer.observe(box);
  }

  disconnectedCallback(): void {
    this.observer?.disconnect();
    this.observer = undefined;
    super.disconnectedCallback();
  }

  private showAtPointer = (e: Event) => {
    const { index, clientX, clientY } = (e as CustomEvent<BarHoverDetail>).detail;
    const rect = this.renderRoot.querySelector(".box")?.getBoundingClientRect();
    if (rect) this.tip = { i: index, left: clientX - rect.left + 14, top: clientY - rect.top };
  };

  private showAtRow = (e: Event, geo: ReturnType<typeof buildBars>) => {
    const i = (e as CustomEvent<{ index: number }>).detail.index;
    const r = geo.rows[i];
    if (r) this.tip = { i, left: geo.barX, top: r.y + r.h };
  };

  render() {
    const cargues = this.summary?.cargues || [];
    const view = visibleCargues(cargues, this.showAll);
    const geo = buildBars(view.items, this.width);
    const item = this.tip ? view.items[this.tip.i] : undefined;
    const tooltip = item ? tooltipOf(item) : null;
    return html`<tx-card variant="subtle">
      <div class="head">
        <div class="title">
          <tx-icon name=${ripsIcon("chartBar")}></tx-icon>
          <div>
            <h2>Avance de radicación por cargue</h2>
            <div class="subtitle">
              Cada cargue en una barra: radicadas, gestionadas sin radicar, excluidas y revisión manual suman el total; del más reciente al más antiguo
            </div>
          </div>
        </div>
        ${view.canToggle
          ? html`<tx-button variant="ghost" color="neutral" size="sm" @click=${() => (this.showAll = !this.showAll)}
              >${this.showAll ? "Ver menos" : `Ver todos (${cargues.length})`}</tx-button
            >`
          : nothing}
      </div>
      ${this.error ? html`<div class="error">No se pudo cargar el resumen: ${this.error}</div>` : nothing}
      <div class="box" data-testid="chart-box" @mouseleave=${() => (this.tip = null)}>
        ${this.loading && !this.summary
          ? html`<div class="center" role="status"><tx-icon class="spin" name="refresh"></tx-icon><span>Cargando resumen…</span></div>`
          : !cargues.length
            ? html`<div class="center">Todavía no hay cargues con facturas.</div>`
            : html`<rips-chart-bars
                .geo=${geo}
                .filters=${this.filters}
                @bar-hover=${this.showAtPointer}
                @bar-focus=${(e: Event) => this.showAtRow(e, geo)}
                @bar-leave=${() => (this.tip = null)}
              ></rips-chart-bars>`}
        ${tooltip && this.tip
          ? html`<div class="tip" data-testid="chart-tip" style="left: ${this.tip.left}px; top: ${this.tip.top}px">
              <b>${tooltip.title}</b>
              ${tooltip.cargue !== tooltip.title && tooltip.cargue !== "." ? html`<div>Cargue: ${tooltip.cargue}</div>` : nothing}
              ${tooltip.inicio ? html`<div>Inicio del cargue: ${tooltip.inicio}</div>` : nothing}
              <div>Total de facturas: ${tooltip.total}</div>
              <div>Radicadas: ${tooltip.radicadas}</div>
              <div>Gestionadas (sin radicar): ${tooltip.gestionadasSinRadicar}</div>
              ${tooltip.revisionManual ? html`<div>Revisión manual: ${tooltip.revisionManual}</div>` : nothing}
              <div>Excluidas: ${tooltip.excluidas}</div>
              <div>Gestionadas en total (incluye radicadas): ${tooltip.gestionadas}</div>
              <div>% radicado sobre gestionadas: ${tooltip.pct}%</div>
              <div>$ gestionado: ${tooltip.valorCargado}</div>
              <div>$ radicado: ${tooltip.valorRadicado}</div>
            </div>`
          : nothing}
      </div>
      <div class="legend">
        ${LEGEND.map((l) => html`<div><span class="swatch" style="background: var(${l.swatch})"></span><span class="name">${l.label}</span></div>`)}
      </div>
      <tx-alert color="info" variant="subtle" icon=${ripsIcon("info")} role="note">
        Cada cargue se cuenta completo, sin importar las fechas de sus facturas. Radicadas + Gestionadas (las que siguen sin radicar) + Excluidas +
        Revisión manual = Total; "radicada" = los 4 pasos de radicación confirmados y el % radicado es sobre todas las gestionadas. Haz clic en un
        cargue para filtrar las tarjetas y la tabla; otro clic quita el filtro.
      </tx-alert>
    </tx-card>`;
  }
}

if (!customElements.get("rips-chart")) customElements.define("rips-chart", RipsChart);
