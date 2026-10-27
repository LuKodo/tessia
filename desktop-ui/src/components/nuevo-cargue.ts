import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import { dialogs } from "../utils/dialogs";
import { errorMessage } from "../utils/error-message";
import { messageClass, type UiMessage } from "../utils/message-class";
import "./tx-button";
import "./tx-card";
import "./tx-file-upload";
import "./tx-form-field";
import "./tx-input";
import "./tx-separator";

/**
 * "Nuevo cargue (convenio descargar)" card: download a Medicar date range after the operator confirms,
 * or upload a manual CSV/Excel file (POST /api/convenio/upload), which never contacts Medicar and gets its own name.
 */
export class TxNuevoCargue extends LitElement {
  static properties = {
    desde: { state: true },
    hasta: { state: true },
    msg: { state: true },
    csv: { state: true },
    uploadMsg: { state: true },
  };

  static styles = css`
    :host {
      display: block;
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
    .row > tx-form-field {
      flex: 1 1 180px;
      min-width: 160px;
    }
    .row > tx-file-upload {
      flex: 1 1 220px;
      min-width: 200px;
    }
    .msg {
      margin-top: var(--tx-space-2);
      font-size: var(--tx-text-sm);
    }
    tx-separator {
      margin: var(--tx-space-3) 0;
    }
    .text-error {
      color: var(--tx-error);
    }
    .text-success {
      color: var(--tx-success);
    }
    .text-muted {
      color: var(--tx-text-muted);
    }
  `;

  desde = "";
  hasta = "";
  msg: UiMessage = { text: "", kind: "" };
  csv: File | null = null;
  uploadMsg: UiMessage = { text: "", kind: "" };

  private async descargar(): Promise<void> {
    if (!this.desde || !this.hasta) {
      this.msg = { text: "elige ambas fechas", kind: "error" };
      return;
    }
    const { desde, hasta } = this;
    if (!(await dialogs.confirmAction(`¿Descargar el convenio de Medicar del ${desde} al ${hasta}? Esto consulta Medicar de verdad.`))) return;
    this.msg = { text: "iniciando descarga…", kind: "" };
    try {
      // Capitalized keys: the Go request struct has no json tags.
      await api.download({ Desde: desde, Hasta: hasta });
      this.msg = { text: "descarga en curso — sigue el progreso justo abajo", kind: "ok" };
    } catch (e) {
      this.msg = { text: errorMessage(e), kind: "error" };
    }
  }

  private async subirCSV(): Promise<void> {
    if (!this.csv) {
      this.uploadMsg = { text: "elige un archivo CSV o Excel", kind: "error" };
      return;
    }
    this.uploadMsg = { text: "subiendo archivo…", kind: "" };
    try {
      const res = await api.uploadConvenio({ csv: this.csv });
      this.csv = null;
      this.uploadMsg = { text: `Cargue "${res.cargue}" creado (${res.file_name}). Aparecerá como SIN_PREPARAR.`, kind: "ok" };
    } catch (e) {
      this.uploadMsg = { text: errorMessage(e), kind: "error" };
    }
  }

  private message(m: UiMessage, testid: string) {
    return m.text ? html`<div class="msg ${messageClass(m.kind)}" data-testid=${testid}>${m.text}</div>` : nothing;
  }

  render() {
    return html`
      <tx-card>
        <h2>Nuevo cargue (convenio descargar)</h2>
        <div class="row">
          <tx-form-field label="Desde">
            <tx-input block type="date" data-testid="desde" .value=${this.desde} @tx-input=${(e: CustomEvent<{ value: string }>) => (this.desde = e.detail.value)}></tx-input>
          </tx-form-field>
          <tx-form-field label="Hasta">
            <tx-input block type="date" data-testid="hasta" .value=${this.hasta} @tx-input=${(e: CustomEvent<{ value: string }>) => (this.hasta = e.detail.value)}></tx-input>
          </tx-form-field>
          <tx-button data-testid="descargar" @click=${() => void this.descargar()}>Descargar de Medicar</tx-button>
        </div>
        ${this.message(this.msg, "download-msg")}

        <tx-separator label="o subir un CSV o Excel manual (sin fechas, no consulta Medicar)"></tx-separator>
        <div class="row">
          <tx-file-upload
            data-testid="csv-file"
            variant="button"
            label="Archivo de convenio"
            description="Elegir archivo (.csv o .xlsx)"
            accept=".csv,.xlsx"
            .files=${this.csv ? [this.csv] : []}
            @tx-change=${(e: CustomEvent<{ files: File[] }>) => (this.csv = e.detail.files[0] ?? null)}
          ></tx-file-upload>
          <tx-button variant="outline" data-testid="subir-csv" @click=${() => void this.subirCSV()}>Subir archivo</tx-button>
        </div>
        ${this.message(this.uploadMsg, "upload-msg")}
      </tx-card>
    `;
  }
}

if (!customElements.get("tx-nuevo-cargue")) customElements.define("tx-nuevo-cargue", TxNuevoCargue);
