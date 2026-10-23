import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import { errorMessage } from "../utils/error-message";
import { notaTecnicaDetail } from "../utils/health-model";
import type { UiMessage } from "../utils/message-class";
import "./tx-button";
import "./tx-card";

const READ_ONLY_HINT =
  "Catálogo de solo lectura publicado en Redis: esta aplicación solo lo consulta. Para cambiarlo, publique una nueva versión en Redis.";
const MISSING_HINT =
  "No hay una Nota Técnica disponible: Redis no responde o no tiene una versión publicada (nt:actual). Los cargues no podrán prepararse hasta que exista.";

/**
 * Card of the Nota Técnica published in Redis (read-only): version and row count. Talks to `GET /api/nota-tecnica` through `src/api.ts`; there is
 * no upload.
 */
export class TxNotaTecnica extends LitElement {
  static properties = {
    status: { state: true },
    msg: { state: true },
  };

  static styles = css`
    :host {
      display: block;
      font-family: var(--tx-font-sans);
    }
    h2 {
      margin: 0 0 var(--tx-space-2);
      font-size: var(--tx-text-base);
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    .row {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-end;
      gap: var(--tx-space-3);
    }
    .caption {
      font-size: 0.75rem;
      font-weight: 700;
      text-transform: uppercase;
      color: var(--tx-text-muted);
    }
    .current {
      font-size: var(--tx-text-sm);
      color: var(--tx-text);
    }
    .msg {
      margin-top: var(--tx-space-2);
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .msg.ok {
      color: var(--tx-success);
    }
    .msg.error {
      color: var(--tx-error);
    }
  `;

  status = "verificando…";
  msg: UiMessage = { text: READ_ONLY_HINT, kind: "" };

  connectedCallback(): void {
    super.connectedCallback();
    void this.refresh();
  }

  private async refresh(): Promise<void> {
    try {
      const info = await api.notaTecnica();
      this.status = info.exists ? notaTecnicaDetail(info) : "Sin Nota Técnica";
      this.msg = { text: info.exists ? READ_ONLY_HINT : MISSING_HINT, kind: "" };
    } catch (e) {
      this.status = "No disponible";
      this.msg = { text: errorMessage(e), kind: "error" };
    }
  }

  render() {
    return html`
      <tx-card>
        <h2>Nota técnica (Redis)</h2>
        <div class="row">
          <div>
            <div class="caption">Publicada en Redis</div>
            <div class="current" data-testid="nota-status">${this.status}</div>
          </div>
          <tx-button color="neutral" variant="outline" data-testid="nota-actualizar" @click=${this.refresh}>Actualizar</tx-button>
        </div>
        ${this.msg.text
          ? html`<div class="msg ${this.msg.kind}" data-testid="nota-msg">${this.msg.text}</div>`
          : nothing}
      </tx-card>
    `;
  }
}

if (!customElements.get("tx-nota-tecnica")) customElements.define("tx-nota-tecnica", TxNotaTecnica);

declare global {
  interface HTMLElementTagNameMap {
    "tx-nota-tecnica": TxNotaTecnica;
  }
}
