import { LitElement, css, html } from "lit";
import "../components/historial";

const TITLE = "Ejecuciones — Tessia";

/**
 * /ejecuciones: history of the finished processes. Starting a process and following the one in
 * progress live on the Dashboard (the process panel, new cargue and console components).
 */
export class TxPageEjecuciones extends LitElement {
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
    p {
      margin: 0;
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
  `;

  private previousTitle: string | null = null;

  connectedCallback(): void {
    super.connectedCallback();
    this.previousTitle = document.title;
    document.title = TITLE;
  }

  disconnectedCallback(): void {
    // Only undo our own title: another page may have set its own already.
    if (this.previousTitle !== null && document.title === TITLE) document.title = this.previousTitle;
    this.previousTitle = null;
    super.disconnectedCallback();
  }

  render() {
    return html`
      <div>
        <h2>Ejecuciones</h2>
        <p>Historial de procesos ya terminados. El progreso de un proceso en curso se ve en el Dashboard.</p>
      </div>
      <tx-historial></tx-historial>
    `;
  }
}

if (!customElements.get("tx-page-ejecuciones")) customElements.define("tx-page-ejecuciones", TxPageEjecuciones);
