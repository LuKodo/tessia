import { LitElement, css, html, nothing } from "lit";
import { classMap } from "lit/directives/class-map.js";
import { live } from "lit/directives/live.js";
import { repeat } from "lit/directives/repeat.js";
import type { HallazgoRow } from "../controllers/hallazgos-controller";
import { MAX_DESCRIPTION, descriptionError } from "../utils/hallazgos-model";
import "./tx-card";
import "./tx-checkbox";
import "./tx-icon";
import "./tx-textarea";

/**
 * Presentational table of the Hallazgos page: it renders the rows it receives and reports edits with
 * (non-bubbling) CustomEvents, so the page decides what to do with them:
 * `toggle-glosa` / `toggle-devolucion` with `detail: { codigo, value }`, `edit` with `{ codigo, text }` and
 * `commit` with `{ codigo }` (description blur or Ctrl/Cmd+Enter).
 * `total` is the size of the whole catalog (before the search filter), to tell "empty" from "no match".
 * A plain table instead of a table component: every row is an inline editor with its own save state.
 */
export class HallazgosTabla extends LitElement {
  static properties = {
    rows: { attribute: false },
    total: { type: Number },
  };

  static styles = css`
    :host {
      display: block;
    }
    .empty {
      text-align: center;
      color: var(--tx-text-muted);
    }
    .scroll {
      max-height: calc(100vh - 270px);
      overflow: auto;
    }
    table {
      width: 100%;
      border-collapse: collapse;
      font-size: var(--tx-text-sm);
    }
    th {
      position: sticky;
      top: 0;
      z-index: 1;
      padding: var(--tx-space-2) var(--tx-space-3);
      background: var(--tx-bg-elevated);
      color: var(--tx-text-muted);
      font-size: 0.75rem;
      font-weight: 600;
      text-align: left;
    }
    th.codigo-col {
      width: 8rem;
    }
    th.detalle-col {
      width: 30%;
    }
    th.glosa-col {
      width: 7rem;
    }
    th.devolucion-col {
      width: 7.5rem;
    }
    td {
      padding: var(--tx-space-2) var(--tx-space-3);
    }
    tr.hallazgo-row {
      border-top: 1px solid var(--tx-border);
      vertical-align: top;
    }
    tr.hallazgo-row--glosa {
      background: color-mix(in oklab, var(--tx-warning) 5%, transparent);
    }
    td.codigo {
      font-family: var(--tx-font-mono);
      font-weight: 700;
      white-space: nowrap;
    }
    tr.hallazgo-row--glosa td.codigo {
      box-shadow: inset 4px 0 0 var(--tx-warning);
    }
    .devolucion-icon {
      margin-left: 0.25rem;
      width: 1rem;
      height: 1rem;
      vertical-align: text-bottom;
      color: var(--tx-error);
    }
    td.detalle {
      overflow-wrap: anywhere;
    }
    .editor {
      display: flex;
      align-items: flex-start;
      gap: var(--tx-space-3);
    }
    .campo {
      display: flex;
      flex: 1;
      flex-direction: column;
      gap: var(--tx-space-1);
      min-width: 0;
    }
    tx-textarea {
      width: 100%;
    }
    .pie {
      display: flex;
      justify-content: space-between;
      gap: var(--tx-space-2);
      font-size: 0.75rem;
    }
    .descripcion-error {
      color: var(--tx-error);
    }
    .contador {
      color: var(--tx-text-muted);
    }
    .estado {
      display: flex;
      flex: none;
      align-items: center;
      box-sizing: border-box;
      width: 7.5rem;
      min-height: 2.5rem;
      font-size: 0.75rem;
    }
    .estado .ok,
    .estado .fallo {
      display: inline-flex;
      gap: 0.25rem;
    }
    .estado .ok {
      align-items: center;
      color: var(--tx-success);
    }
    .estado .fallo {
      align-items: flex-start;
      color: var(--tx-error);
    }
    .estado tx-icon {
      width: 1rem;
      height: 1rem;
    }
    .estado .fallo tx-icon {
      flex: none;
    }
    .estado .guardando {
      color: var(--tx-text-muted);
      animation: spin 1s linear infinite;
    }
    @keyframes spin {
      to {
        transform: rotate(360deg);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .estado .guardando {
        animation: none;
      }
    }
  `;

  rows: HallazgoRow[] = [];
  total = 0;

  private send(name: string, detail: object): void {
    this.dispatchEvent(new CustomEvent(name, { detail }));
  }

  private onKeydown(event: KeyboardEvent, codigo: string): void {
    if (event.key !== "Enter" || !(event.ctrlKey || event.metaKey)) return;
    event.preventDefault();
    this.send("commit", { codigo });
  }

  private renderEstado(row: HallazgoRow) {
    if (row.status === "saving") return html`<tx-icon class="guardando" name="refresh"></tx-icon>`;
    if (row.status === "saved") return html`<span class="ok"><tx-icon name="check-circle"></tx-icon>Guardado</span>`;
    if (row.status === "error") {
      return html`<span class="fallo" title=${row.message}><tx-icon name="warning-circle"></tx-icon>${row.message}</span>`;
    }
    return nothing;
  }

  private renderRow(row: HallazgoRow) {
    const error = descriptionError(row.descripcion);
    return html`<tr
      class=${classMap({
        "hallazgo-row": true,
        "hallazgo-row--glosa": row.genera_glosa,
        "hallazgo-row--devolucion": Boolean(row.devolucion),
      })}
    >
      <td class="codigo">
        ${row.codigo}${row.devolucion
          ? html`<tx-icon class="devolucion-icon" name="undo" label="Devuelve la factura" title="Devuelve la factura"></tx-icon>`
          : nothing}
      </td>
      <td class="detalle">${row.detalle}</td>
      <td>
        <tx-checkbox
          color="warning"
          aria-label="¿${row.codigo} genera glosa?"
          .checked=${live(row.genera_glosa)}
          @tx-change=${(e: CustomEvent<{ checked: boolean }>) => this.send("toggle-glosa", { codigo: row.codigo, value: e.detail.checked === true })}
        ></tx-checkbox>
      </td>
      <td>
        <tx-checkbox
          color="error"
          aria-label="¿${row.codigo} devuelve la factura?"
          .checked=${live(Boolean(row.devolucion))}
          @tx-change=${(e: CustomEvent<{ checked: boolean }>) =>
            this.send("toggle-devolucion", { codigo: row.codigo, value: e.detail.checked === true })}
        ></tx-checkbox>
      </td>
      <td>
        <div class="editor">
          <div class="campo">
            <tx-textarea
              aria-label="Descripción de ${row.codigo}"
              placeholder="Describe por qué genera o no glosa"
              autoresize
              .rows=${1}
              .maxrows=${3}
              color=${error ? "error" : "primary"}
              .value=${live(row.descripcion)}
              @tx-input=${(e: CustomEvent<{ value: string }>) => this.send("edit", { codigo: row.codigo, text: String(e.detail.value ?? "") })}
              @blur=${() => this.send("commit", { codigo: row.codigo })}
              @keydown=${(e: KeyboardEvent) => this.onKeydown(e, row.codigo)}
            ></tx-textarea>
            <div class="pie">
              <span class="descripcion-error">${error}</span>
              <span class="contador">${row.descripcion.length} / ${MAX_DESCRIPTION}</span>
            </div>
          </div>
          <div class="estado" aria-live="polite">${this.renderEstado(row)}</div>
        </div>
      </td>
    </tr>`;
  }

  protected render() {
    if (this.total === 0) {
      return html`<tx-card class="empty">
        Aún no hay hallazgos: se llenan solos cuando se consulta un cargue en el portal de Mutual Ser.
      </tx-card>`;
    }
    if (this.rows.length === 0) {
      return html`<tx-card class="empty">Ningún hallazgo coincide con la búsqueda.</tx-card>`;
    }
    return html`<tx-card padding="none">
      <div class="scroll">
        <table>
          <thead>
            <tr>
              <th class="codigo-col">Código</th>
              <th class="detalle-col">Detalle</th>
              <th class="glosa-col">¿Genera glosa?</th>
              <th class="devolucion-col">¿Devuelve la factura?</th>
              <th>Descripción</th>
            </tr>
          </thead>
          <tbody>
            ${repeat(this.rows, (row) => row.codigo, (row) => this.renderRow(row))}
          </tbody>
        </table>
      </div>
    </tx-card>`;
  }
}

if (!customElements.get("tx-hallazgos-tabla")) customElements.define("tx-hallazgos-tabla", HallazgosTabla);

declare global {
  interface HTMLElementTagNameMap {
    "tx-hallazgos-tabla": HallazgosTabla;
  }
}
