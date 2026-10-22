import { LitElement, css, html, nothing } from "lit";
import type { PropertyValues } from "lit";
import { api } from "../../api";
import type { PortalView } from "../../types/api";
import { errorMessage } from "../../utils/error-message";
import "../tx-alert";
import "../tx-button";
import "../tx-icon";
import "../tx-modal";
import "./portal-contenido";
import { ripsIcon, spinStyles } from "./rips-tones";

/**
 * Result of the Mutual Ser portal for one invoice. Fetches every time it opens (and when the invoice
 * changes while open). Events: `loaded` after a successful fetch/refresh (so the dashboard re-polls
 * the rows: the table icon may have changed) and `close` when the user asks to close; the owner
 * closes it by clearing `open`.
 */
export class PortalModal extends LitElement {
  static properties = {
    open: { type: Boolean },
    factura: { type: String },
    loading: { state: true },
    refreshing: { state: true },
    error: { state: true },
    payload: { state: true },
  };

  static styles = [
    spinStyles,
    css`
      :host {
        display: contents;
      }
      .loading {
        display: flex;
        align-items: center;
        gap: var(--tx-space-3);
        padding: 1.5rem 0;
        font-size: var(--tx-text-sm);
      }
      .loading tx-icon {
        font-size: 1.5rem;
        color: var(--tx-text-muted);
      }
      .request-error {
        display: block;
        margin-bottom: var(--tx-space-3);
      }
      .footer {
        display: flex;
        width: 100%;
        align-items: center;
        gap: var(--tx-space-3);
      }
      .grow {
        flex: 1;
      }
    `,
  ];

  open = false;
  factura = "";
  loading = false;
  refreshing = false;
  error = "";
  payload: PortalView | null = null;

  // One request at a time matters: a counter drops responses overtaken by a newer one
  // (invoice switched, dialog reopened, refresh pressed meanwhile).
  private requestId = 0;

  protected willUpdate(changed: PropertyValues<this>): void {
    if ((changed.has("open") || changed.has("factura")) && this.open && this.factura) void this.load();
  }

  disconnectedCallback(): void {
    this.requestId++; // anything still in flight is dropped
    super.disconnectedCallback();
  }

  private async run(call: () => Promise<PortalView>, flag: "loading" | "refreshing") {
    const id = ++this.requestId;
    this[flag] = true;
    this.error = "";
    try {
      const body = await call();
      if (id !== this.requestId) return;
      this.payload = body;
      this.dispatchEvent(new CustomEvent("loaded"));
    } catch (e) {
      if (id === this.requestId) this.error = "No se pudo consultar el portal: " + errorMessage(e);
    } finally {
      if (id === this.requestId) {
        this.loading = false;
        this.refreshing = false;
      }
    }
  }

  private load() {
    this.payload = null;
    this.refreshing = false;
    const factura = this.factura;
    return this.run(() => api.portal(factura), "loading");
  }

  private refresh() {
    const factura = this.factura;
    return this.run(() => api.refreshPortal(factura), "refreshing");
  }

  private close = (e?: Event) => {
    e?.preventDefault(); // the owner closes the modal by clearing `open`
    this.dispatchEvent(new CustomEvent("close"));
  };

  render() {
    return html`<tx-modal
      size="xl"
      title="Resultado del portal de Mutual Ser"
      description=${this.factura}
      .open=${this.open}
      @tx-open-change=${this.close}
    >
      <div slot="body">
        ${this.loading
          ? html`<div class="loading">
              <tx-icon class="spin" name="refresh"></tx-icon>
              <span>Consultando el portal de Mutual Ser… (puede tardar unos segundos)</span>
            </div>`
          : html`${this.error
              ? html`<tx-alert class="request-error" color="error" variant="subtle" role="alert" icon=${ripsIcon("circleX")} description=${this.error}></tx-alert>`
              : nothing}
            ${this.payload ? html`<portal-contenido .payload=${this.payload}></portal-contenido>` : nothing}`}
      </div>
      <div slot="footer" class="footer">
        <tx-button
          color="neutral"
          variant="outline"
          ?loading=${this.refreshing}
          ?disabled=${!this.factura || this.loading || this.refreshing}
          @click=${() => this.refresh()}
          >Actualizar desde el portal</tx-button
        >
        <div class="grow"></div>
        <tx-button @click=${() => this.close()}>Cerrar</tx-button>
      </div>
    </tx-modal>`;
  }
}

if (!customElements.get("portal-modal")) customElements.define("portal-modal", PortalModal);
