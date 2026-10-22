import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import type { NotaTecnicaStatus } from "../types/api";
import { errorMessage } from "../utils/error-message";
import { fmtTime } from "../utils/format";
import { notaTecnicaFilas, notaTecnicaVersion, notaTecnicaWarning, relativeAge } from "../utils/health-model";
import type { UiMessage } from "../utils/message-class";
import "./tx-button";
import "./tx-card";

const READ_ONLY_HINT =
  "Catálogo de solo lectura publicado en Redis: esta aplicación solo lo consulta. Para cambiarlo, publique una nueva versión en Redis.";
const MISSING_HINT =
  "No hay una Nota Técnica disponible: Redis no responde o no tiene una versión publicada (nt:actual). Los cargues no podrán prepararse hasta que exista.";

/**
 * Card of the Nota Técnica published in Redis (read-only): version, rows, generated/published dates
 * and the bases the catalog lacks. Talks to `GET /api/nota-tecnica` through `src/api.ts`; there is
 * no upload.
 */
export class TxNotaTecnica extends LitElement {
  static properties = {
    info: { state: true },
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
    .meta {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .warning {
      margin-top: var(--tx-space-2);
      font-size: var(--tx-text-sm);
      font-weight: 700;
      color: var(--tx-warning);
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

  info: NotaTecnicaStatus | null = null;
  status = "verificando…";
  msg: UiMessage = { text: READ_ONLY_HINT, kind: "" };

  connectedCallback(): void {
    super.connectedCallback();
    void this.refresh();
  }

  private async refresh(): Promise<void> {
    try {
      const info = await api.notaTecnica();
      this.info = info;
      this.status = info.exists
        ? [notaTecnicaVersion(info) && `Versión ${notaTecnicaVersion(info)}`, notaTecnicaFilas(info)].filter(Boolean).join(" · ") || "Publicada"
        : "Sin Nota Técnica";
      this.msg = { text: info.exists ? READ_ONLY_HINT : MISSING_HINT, kind: "" };
    } catch (e) {
      this.info = null;
      this.status = "No disponible";
      this.msg = { text: errorMessage(e), kind: "error" };
    }
  }

  private dates(info: NotaTecnicaStatus): string {
    const published = fmtTime(info.publicado_en);
    const age = published ? relativeAge(info.publicado_en) : "";
    return [
      info.generado_en && `Generada: ${info.generado_en}`,
      published && `Publicada: ${published}${age ? ` · ${age}` : ""}`,
    ]
      .filter(Boolean)
      .join(" · ");
  }

  render() {
    const info = this.info?.exists ? this.info : null;
    const dates = info ? this.dates(info) : "";
    const warning = notaTecnicaWarning(info);
    return html`
      <tx-card>
        <h2>Nota técnica (Redis)</h2>
        <div class="row">
          <div>
            <div class="caption">Versión vigente</div>
            <div class="current" data-testid="nota-status">${this.status}</div>
            ${dates ? html`<div class="meta" data-testid="nota-fechas">${dates}</div>` : nothing}
          </div>
          <tx-button color="neutral" variant="outline" data-testid="nota-actualizar" @click=${this.refresh}>Actualizar</tx-button>
        </div>
        ${warning ? html`<div class="warning" data-testid="nota-bases">${warning}</div>` : nothing}
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
