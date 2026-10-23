import { LitElement, css, html, nothing } from "lit";
import type { PortalView } from "../../types/api";
import { fmtTime } from "../../utils/format";
import { groupPortalFindings, portalIcon, portalIconClass, portalInfoRows, portalStatusColor } from "../../utils/portal-model";
import "../tx-alert";
import "../tx-badge";
import "../tx-icon";
import { iconName, ripsIcon } from "./rips-tones";

/**
 * Presentational body of the portal modal: renders one GET /api/facturas/{f}/portal payload.
 * No fetching here, so it can be rendered in isolation.
 */
export class PortalContenido extends LitElement {
  static properties = { payload: { attribute: false } };

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-4);
    }
    .status {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-2);
    }
    .status tx-icon {
      font-size: 1.5rem;
    }
    [data-tone="neutral"] {
      color: var(--tx-text-muted);
    }
    [data-tone="error"] {
      color: var(--tx-error);
    }
    [data-tone="warning"] {
      color: var(--tx-warning);
    }
    [data-tone="info"] {
      color: var(--tx-info);
    }
    [data-tone="success"] {
      color: var(--tx-success);
    }
    .factura {
      font-size: var(--tx-text-base);
      font-weight: 700;
    }
    .muted {
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
    h3 {
      margin: 0 0 0.25rem;
      font-size: 0.75rem;
      font-weight: 500;
      letter-spacing: 0.1em;
      text-transform: uppercase;
      color: var(--tx-text);
    }
    .info {
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
      gap: 0.5rem 1rem;
    }
    .info .value {
      overflow-wrap: break-word;
      font-size: var(--tx-text-sm);
      font-weight: 600;
    }
    .group {
      margin-bottom: var(--tx-space-3);
    }
    .group-head {
      display: flex;
      align-items: center;
      gap: var(--tx-space-2);
      margin-bottom: 0.25rem;
    }
    .finding {
      margin-bottom: 0.25rem;
      padding: var(--tx-space-2);
      border: 1px solid var(--tx-border);
      border-radius: var(--tx-radius-lg);
    }
    .finding-head {
      display: flex;
      flex-wrap: wrap;
      align-items: baseline;
      gap: var(--tx-space-2);
    }
    .finding-field {
      font-family: var(--tx-font-mono);
      font-size: 0.75rem;
      word-break: break-all;
    }
    .finding-detail {
      overflow-wrap: anywhere;
      font-size: var(--tx-text-sm);
    }
  `;

  payload: PortalView | null = null;

  render() {
    const p = this.payload;
    if (!p) return nothing;
    const status = portalIcon(p);
    const rows = portalInfoRows(p);
    const groups = groupPortalFindings(p.hallazgos);
    const consulted = !!p.consultado;
    const n = Number(p.consultas) || 0;
    const origin = p.origen === "portal" ? "portal" : "base de datos";
    const footer = `Consultado ${fmtTime(p.consultado_en) || "—"} · ${n} consulta${n === 1 ? "" : "s"} · ${origin}`;
    return html`
      ${p.error
        ? html`<tx-alert class="portal-error" color="warning" variant="subtle" role="alert" icon=${ripsIcon("alertCircleFilled")} description=${p.error}></tx-alert>`
        : nothing}
      <div class="status">
        <tx-icon name=${iconName(portalIconClass(status.key))} data-tone=${status.color} data-testid="portal-status-icon"></tx-icon>
        <span class="factura">${p.factura}</span>
        ${consulted && p.estado
          ? html`<tx-badge size="md" variant="solid" color=${portalStatusColor(p.estado)}>${p.estado}</tx-badge>`
          : nothing}
        <span class="muted">${status.label}</span>
      </div>
      ${!consulted
        ? html`<tx-alert
            class="portal-empty"
            color="info"
            variant="subtle"
            role="note"
            icon=${ripsIcon("info")}
            description=${p.mensaje || "El portal aún no tiene registro de esta factura"}
          ></tx-alert>`
        : html`${rows.length
              ? html`<section>
                  <h3>Datos del portal</h3>
                  <div class="info">
                    ${rows.map(([label, value]) => html`<div><div class="muted">${label}</div><div class="value">${value}</div></div>`)}
                  </div>
                </section>`
              : nothing}
            <section>
              <h3>Hallazgos</h3>
              ${!groups.length
                ? html`<tx-alert
                    class="portal-clean"
                    color="success"
                    variant="subtle"
                    role="note"
                    icon=${ripsIcon("circleCheckFilled")}
                    description="Sin hallazgos: el portal validó el JSON sin advertencias"
                  ></tx-alert>`
                : nothing}
              ${groups.map(
                (g) => html`<div class="group">
                  <div class="group-head">
                    <tx-badge size="md" variant="subtle" color=${g.color}>${g.label}</tx-badge>
                    <span class="muted">${g.items.length}</span>
                  </div>
                  ${g.items.map(
                    (h) => html`<div class="finding">
                      <div class="finding-head">
                        <strong class="finding-code">${h.codigo}</strong>
                        ${h.campo ? html`<code class="finding-field">${h.campo}</code>` : nothing}
                      </div>
                      <div class="finding-detail">${h.detalle}</div>
                    </div>`,
                  )}
                </div>`,
              )}
            </section>`}
      <div class="portal-footer muted">${footer}</div>
    `;
  }
}

if (!customElements.get("portal-contenido")) customElements.define("portal-contenido", PortalContenido);
