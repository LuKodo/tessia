import { LitElement, css, nothing, svg } from "lit";
import { repeat } from "lit/directives/repeat.js";
import { FiltersBinding } from "../../controllers/rips-filters-controller";
import type { RipsFiltersController } from "../../controllers/rips-filters-controller";
import type { CarguePoint } from "../../types/api";
import { cargueFilterValue } from "../../utils/cargues-chart-model";
import type { buildBars } from "../../utils/cargues-chart-model";
import { chartPalette } from "./rips-tones";

export type ChartGeometry = ReturnType<typeof buildBars>;

export interface BarHoverDetail {
  index: number;
  clientX: number;
  clientY: number;
}

const ariaOf = (r: { full: string; pctText: string; lines: string[] }) => `${r.full}: ${r.pctText}. ${r.lines.join(" · ")}`;

/**
 * The hand-rolled SVG of the cargue chart: one clickable row per cargue (toggles the cargue filter).
 * Events (bubbling, composed): `bar-hover` {index, clientX, clientY}, `bar-focus` {index}, `bar-leave`.
 */
export class RipsChartBars extends LitElement {
  static properties = { geo: { attribute: false }, filters: { attribute: false } };

  static styles = [
    chartPalette,
    css`
      :host {
        display: block;
      }
      .chart-svg {
        display: block;
        width: 100%;
        height: auto;
      }
      .row {
        outline: none;
      }
      .row.clickable {
        cursor: pointer;
      }
      .row-bg {
        fill: transparent;
        stroke: transparent;
        stroke-width: 2;
      }
      .row:hover .row-bg {
        fill: var(--tx-bg-muted);
      }
      .row.active .row-bg {
        fill: var(--tx-primary-soft);
        stroke: var(--tx-primary);
      }
      .row:focus-visible .row-bg {
        stroke: var(--tx-primary);
      }
      .label {
        fill: var(--tx-text-highlighted);
        font-size: 12px;
      }
      .row.active .label {
        font-weight: 700;
      }
      .track {
        fill: var(--tx-border);
      }
      .seg {
        stroke: var(--tx-bg-elevated);
        stroke-width: 1;
      }
      .seg-filed {
        fill: var(--seg-filed);
      }
      .seg-managed {
        fill: var(--seg-managed);
      }
      .seg-manual {
        fill: var(--seg-manual);
      }
      .seg-excluded {
        fill: var(--seg-excluded);
      }
      .info {
        fill: var(--tx-text-muted);
        font-size: 12px;
      }
      .pct {
        fill: var(--tx-text-highlighted);
        font-size: 12px;
        font-weight: 700;
      }
    `,
  ];

  geo: ChartGeometry | null = null;
  filters: RipsFiltersController | null = null;

  private binding = new FiltersBinding(this);

  protected willUpdate(): void {
    this.binding.follow(this.filters);
  }

  private fire(name: string, detail?: unknown) {
    this.dispatchEvent(new CustomEvent(name, { detail, bubbles: true, composed: true }));
  }

  private toggle(c: CarguePoint) {
    const value = cargueFilterValue(c);
    if (value) this.filters?.toggleText("cargue", value);
  }

  render() {
    const geo = this.geo;
    if (!geo) return nothing;
    return svg`<svg class="chart-svg" viewBox="0 0 ${geo.W} ${geo.H}" role="group" aria-label="Avance de radicación por cargue">
      ${repeat(
        geo.rows,
        (r) => r.cargue.cargue,
        (r, i) => {
          const value = cargueFilterValue(r.cargue);
          const active = !!value && !!this.filters?.isTextActive("cargue", value);
          const hover = (e: MouseEvent) => this.fire("bar-hover", { index: i, clientX: e.clientX, clientY: e.clientY });
          return svg`<g
            class="row ${active ? "active" : ""} ${value ? "clickable" : ""}"
            role="button"
            tabindex=${value ? 0 : -1}
            aria-pressed=${active}
            aria-label=${ariaOf(r)}
            @click=${() => this.toggle(r.cargue)}
            @keydown=${(e: KeyboardEvent) => {
              if (e.key !== "Enter" && e.key !== " ") return;
              e.preventDefault();
              this.toggle(r.cargue);
            }}
            @mouseenter=${hover}
            @mousemove=${hover}
            @focus=${() => this.fire("bar-focus", { index: i })}
            @blur=${() => this.fire("bar-leave")}
          >
            <title>${r.full}</title>
            <rect class="row-bg" x="0" y=${r.y} width=${geo.W} height=${r.h} rx="4" />
            <text class="label" x=${geo.barX} y=${r.labelY}>${r.label}</text>
            <text class="pct" x=${geo.barX + geo.barW} y=${r.labelY} text-anchor="end">${r.pctText}</text>
            <rect class="track" x=${geo.barX} y=${r.barY} width=${geo.barW} height=${r.barH} rx="3" />
            ${r.segments.map((s) => svg`<rect class="seg ${s.cls}" x=${s.x} y=${r.barY} width=${s.w} height=${r.barH} />`)}
            ${r.lines.map((line, k) => svg`<text class="info" x=${geo.barX} y=${r.linesY + k * r.lineH}>${line}</text>`)}
          </g>`;
        },
      )}
    </svg>`;
  }
}

if (!customElements.get("rips-chart-bars")) customElements.define("rips-chart-bars", RipsChartBars);
