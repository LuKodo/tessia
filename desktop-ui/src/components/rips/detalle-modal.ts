import { LitElement, css, html, nothing } from "lit";
import type { PropertyValues } from "lit";
import { api } from "../../api";
import type { InvoiceTracking, TrackingRow } from "../../types/api";
import { notesOf, plazoInfo, stepperSteps } from "../../utils/detalle-model";
import { errorMessage } from "../../utils/error-message";
import { whenText } from "../../utils/rips-format";
import "../tx-badge";
import "../tx-button";
import "../tx-icon";
import "../tx-modal";
import "./detalle-card";
import "./detalle-convenio";
import "./detalle-etapas";
import "./detalle-field";
import "./rips-estado-badge";

/**
 * Invoice detail. The slim table row is already in memory (table polling), so it opens at once:
 * open while `row` is set. The heavy fields the list omits (convenio lines, estado detail) are
 * fetched on demand from GET /api/facturas/{factura}/seguimiento when the modal opens, and again
 * when the live row's estado changes. Fires `close` when the user asks to close (Escape, backdrop,
 * X, Cerrar); the owner closes it by clearing `row`. Fields with no source here (Zonaser block,
 * MEG case, direccionamiento...) are omitted, as before.
 */
export class DetalleModal extends LitElement {
  static properties = { row: { attribute: false }, detail: { state: true }, loading: { state: true }, error: { state: true } };

  static styles = css`
    :host {
      display: contents;
    }
    .eyebrow {
      display: block;
      font-size: 0.75rem;
      text-transform: uppercase;
      color: var(--tx-text-muted);
    }
    .factura {
      display: block;
      font-size: 1.5rem;
      line-height: 1.25;
      font-weight: 700;
    }
    .badge-actions {
      margin-right: var(--tx-space-3);
      transform: scale(1.25);
      transform-origin: right;
    }
    .body {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-3);
    }
    .split {
      display: grid;
      grid-template-columns: 1fr;
      gap: var(--tx-space-3);
      margin-top: var(--tx-space-2);
      padding-top: var(--tx-space-2);
      border-top: 1px solid var(--tx-border);
    }
    @media (min-width: 640px) {
      .split {
        grid-template-columns: 1fr 1fr;
      }
    }
    .wrap {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-2);
    }
    .wrap.loose {
      gap: var(--tx-space-3);
    }
    .plazo-date {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    tx-badge {
      max-width: 100%;
      white-space: normal;
    }
    tx-badge.alert {
      text-transform: capitalize;
    }
    .stepper {
      display: flex;
      flex-wrap: wrap;
      align-items: flex-start;
      row-gap: var(--tx-space-3);
      padding-bottom: 0.25rem;
    }
    .step {
      display: flex;
      min-width: 0;
      flex: 1 1 84px;
      flex-direction: column;
      align-items: center;
      gap: 0.25rem;
    }
    .step-dot {
      display: grid;
      place-items: center;
      width: 1.75rem;
      height: 1.75rem;
      border-radius: 9999px;
      background: var(--tx-bg-muted);
      color: var(--tx-error);
      font-size: 1rem;
    }
    .step-dot.ok {
      background: var(--tx-success);
      color: white;
    }
    .step-label {
      font-size: 0.75rem;
      line-height: 1.2;
      text-align: center;
    }
    .footer {
      display: flex;
      width: 100%;
      justify-content: flex-end;
    }
  `;

  row: TrackingRow | null = null;
  detail: InvoiceTracking | null = null;
  loading = false;
  error = "";

  // Kept so the content does not blank out during the close.
  private lastRow: TrackingRow | null = null;
  // Identity of the invoice and the version of its state the detail was requested for.
  private invoiceKey = "";
  private versionKey = "";
  private requestId = 0;

  protected willUpdate(changed: PropertyValues<this>): void {
    if (!changed.has("row") || !this.row) return;
    this.lastRow = this.row;
    const row = this.row;
    const invoiceKey = (row.cargue || "") + "/" + row.factura;
    const versionKey = invoiceKey + "|" + (row.estado || "") + "|" + (row.actualizado_en || "");
    if (invoiceKey !== this.invoiceKey) {
      this.invoiceKey = invoiceKey;
      this.detail = null; // another invoice: never show the previous one's lines
    }
    if (versionKey === this.versionKey) return;
    this.versionKey = versionKey;
    void this.loadDetail(row);
  }

  private retry = () => {
    if (this.lastRow) void this.loadDetail(this.lastRow);
  };

  // A newer request supersedes this one; the detail on screen stays while a refresh runs.
  private async loadDetail(row: TrackingRow) {
    const id = ++this.requestId;
    this.loading = !this.detail;
    this.error = "";
    try {
      const detail = await api.seguimientoFactura(row.factura, row.cargue || undefined);
      if (id === this.requestId) this.detail = detail;
    } catch (e) {
      if (id === this.requestId) this.error = errorMessage(e);
    } finally {
      if (id === this.requestId) this.loading = false;
    }
  }

  private close = (e?: Event) => {
    e?.preventDefault(); // the owner closes the modal by clearing `row`
    this.dispatchEvent(new CustomEvent("close"));
  };

  render() {
    const live = this.row || this.lastRow;
    // The live row (kept fresh by the table polls) plus the whole detail text and the lines of the
    // detail; until it arrives the list's short estado detail stands in for the whole one.
    const shown: InvoiceTracking | null = live && { ...live, estado_detalle: this.detail?.estado_detalle ?? live.estado_detalle, lineas_convenio: this.detail?.lineas_convenio };
    return html`<tx-modal size="lg" .open=${!!this.row} @tx-open-change=${this.close}>${shown ? this.content(shown) : nothing}</tx-modal>`;
  }

  private content(shown: InvoiceTracking) {
    const notes = notesOf(shown);
    const steps = stepperSteps(shown);
    const plazo = plazoInfo(shown);
    return html`
      <div slot="title"><span class="eyebrow">Factura</span><span class="factura">${shown.factura}</span></div>
      <div slot="actions" class="badge-actions"><rips-estado-badge .row=${shown} .showDetail=${false}></rips-estado-badge></div>
      <div slot="body" class="body">
        <detalle-card>
          <detalle-field label="Lote / Cargue">${shown.cargue_nombre || shown.cargue || "-"}</detalle-field>
          <div class="split">
            <detalle-field label="Vigencia del cargue">${shown.fecha_inicio_cargue || "-"} → ${shown.fecha_fin_cargue || "-"}</detalle-field>
            <detalle-field label="Última actualización">${whenText(shown.actualizado_en) || "-"}</detalle-field>
          </div>
        </detalle-card>

        ${notes.ajuste || notes.credito
          ? html`<detalle-card title="NOTAS">
              <div class="wrap">
                ${notes.ajuste ? html`<tx-badge size="md" variant="subtle" color="warning">Nota Ajuste</tx-badge>` : nothing}
                ${notes.credito ? html`<tx-badge size="md" variant="subtle" color="warning">Nota Crédito</tx-badge>` : nothing}
              </div>
            </detalle-card>`
          : nothing}

        <detalle-card title="PLAZO: 22 días hábiles para completar la radicación">
          <div class="wrap loose">
            ${plazo.kind !== "vencida" && plazo.kind !== "completa"
              ? html`<span class="plazo-date">Fecha de factura: ${plazo.fecha || "-"}</span>`
              : nothing}
            ${plazo.badge
              ? html`<tx-badge size="md" variant=${plazo.badge.color === "neutral" ? "subtle" : "solid"} color=${plazo.badge.color}>${plazo.badge.text}</tx-badge>`
              : nothing}
          </div>
        </detalle-card>

        <detalle-card title="PIPELINE DE PROCESAMIENTO">
          <div class="stepper">
            ${steps.map(
              (s) => html`<div class="step">
                <span class="step-dot ${s.ok ? "ok" : ""}" data-testid="stepper-dot"><tx-icon name=${s.ok ? "check" : "xmark"}></tx-icon></span>
                <span class="step-label">${s.label}</span>
              </div>`,
            )}
          </div>
        </detalle-card>

        <detalle-convenio .row=${shown} .loading=${this.loading} .error=${this.error} @retry=${this.retry}></detalle-convenio>
        <detalle-etapas .row=${shown}></detalle-etapas>

        ${(shown.alertas || []).length > 0
          ? html`<detalle-card title="ALERTAS">
              <div class="wrap">
                ${(shown.alertas || []).map((a) => html`<tx-badge class="alert" size="md" variant="outline" color="warning">${a.replace(/_/g, " ")}</tx-badge>`)}
              </div>
            </detalle-card>`
          : nothing}
      </div>
      <div slot="footer" class="footer"><tx-button color="neutral" variant="outline" size="sm" @click=${() => this.close()}>Cerrar</tx-button></div>
    `;
  }
}

if (!customElements.get("detalle-modal")) customElements.define("detalle-modal", DetalleModal);
