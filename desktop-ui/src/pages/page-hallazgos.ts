import { LitElement, css, html, nothing } from "lit";
import { live } from "lit/directives/live.js";
import { api as defaultApi } from "../api";
import "../components/hallazgos-tabla";
import "../components/tx-alert";
import "../components/tx-button";
import "../components/tx-icon";
import "../components/tx-input";
import "../components/tx-progress";
import { HallazgosController, type HallazgosApi } from "../controllers/hallazgos-controller";

const TITLE = "Hallazgos — Tessia";

/**
 * Hallazgos page: the catalog of findings the Mutual Ser portal returns when a cargue is queried.
 * Each finding can be marked as generating a glosa and/or returning the invoice and carries a
 * description; every edit is saved on its own (see HallazgosController). `api` is a seam for tests.
 */
export class PageHallazgos extends LitElement {
  static properties = { api: { attribute: false } };

  static styles = css`
    :host {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-4);
    }
    h2 {
      margin: 0;
      font-size: var(--tx-text-xl);
      font-weight: 600;
      color: var(--tx-text-highlighted);
    }
    .intro {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .filtro {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-4);
    }
    tx-input {
      width: 100%;
      max-width: 24rem;
    }
    tx-input tx-icon[slot="leading"] {
      color: var(--tx-text-dimmed);
    }
    .resumen {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
  `;

  api: HallazgosApi = defaultApi;

  private hallazgos = new HallazgosController(this, () => this.api);

  connectedCallback(): void {
    super.connectedCallback();
    document.title = TITLE;
  }

  private renderBody() {
    const state = this.hallazgos;
    if (state.loadError) {
      return html`<tx-alert color="error" variant="subtle" icon="warning-circle" heading="No se pudieron cargar los hallazgos" .description=${state.loadError}>
        <tx-button slot="actions" color="neutral" variant="outline" size="sm" ?loading=${state.loading} @click=${() => void state.load()}>
          Reintentar
        </tx-button>
      </tx-alert>`;
    }
    if (state.loading) return html`<tx-progress indeterminate label="Cargando hallazgos"></tx-progress>`;
    return html`
      <div class="filtro">
        <tx-input
          aria-label="Buscar por código o detalle"
          placeholder="Buscar por código o detalle"
          .value=${live(state.query)}
          @tx-input=${(e: CustomEvent<{ value: string }>) => (state.query = e.detail.value)}
        >
          <tx-icon slot="leading" name="search"></tx-icon>
          ${state.query
            ? html`<tx-button slot="trailing" color="neutral" variant="link" size="sm" icon="xmark" aria-label="Limpiar búsqueda" @click=${() => (state.query = "")}></tx-button>`
            : nothing}
        </tx-input>
        <span class="resumen">${state.summary}</span>
      </div>
      <tx-hallazgos-tabla
        .rows=${state.shown}
        .total=${state.rows.length}
        @toggle-glosa=${(e: CustomEvent<{ codigo: string; value: boolean }>) => state.onToggle(e.detail.codigo, e.detail.value)}
        @toggle-devolucion=${(e: CustomEvent<{ codigo: string; value: boolean }>) => state.onToggleDevolucion(e.detail.codigo, e.detail.value)}
        @edit=${(e: CustomEvent<{ codigo: string; text: string }>) => state.onEdit(e.detail.codigo, e.detail.text)}
        @commit=${(e: CustomEvent<{ codigo: string }>) => state.onCommit(e.detail.codigo)}
      ></tx-hallazgos-tabla>
    `;
  }

  protected render() {
    return html`
      <div>
        <h2>Hallazgos</h2>
        <div class="intro">
          Hallazgos que devuelve el portal de Mutual Ser al consultar un cargue. Marca cuáles generan glosa, cuáles devuelven la factura y agrega una descripción.
        </div>
      </div>
      ${this.renderBody()}
    `;
  }
}

if (!customElements.get("tx-page-hallazgos")) customElements.define("tx-page-hallazgos", PageHallazgos);

declare global {
  interface HTMLElementTagNameMap {
    "tx-page-hallazgos": PageHallazgos;
  }
}
