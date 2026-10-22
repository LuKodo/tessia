import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import "../components/nota-tecnica";
import "../components/tx-alert";
import "../components/tx-button";
import "../components/tx-card";
import "../components/tx-checkbox-group";
import "../components/tx-form-field";
import "../components/tx-input";
import { ParametersFormController, type ParametersFormFields } from "../controllers/parameters-form-controller";
import { WAVE_SIZE_HINT, WAVE_SIZE_LABEL } from "../utils/parameters-model";

type NumericField = "waveSize" | "derechosPercent" | "derechosMinSample" | "cuvPercent" | "cuvMinSample";

interface NumberFieldSpec {
  field: NumericField;
  testid: string;
  label: string;
  help: string;
  min: string;
  max?: string;
}

// Labels are in English on purpose: the old Parameters screen is not translated.
const NUMBER_FIELDS: readonly NumberFieldSpec[] = [
  { field: "waveSize", testid: "wave-size", label: WAVE_SIZE_LABEL, help: WAVE_SIZE_HINT, min: "0" },
  {
    field: "derechosPercent",
    testid: "derechos-percent",
    label: "Rights validator down alert (% of affiliates)",
    help: "derechos validar fails when more than this percent of the queried affiliates could not be validated because the validator was unavailable. 0 disables the alert.",
    min: "0",
    max: "100",
  },
  {
    field: "derechosMinSample",
    testid: "derechos-min-sample",
    label: "Rights validator alert: minimum affiliates queried",
    help: "The alert only applies when at least this many affiliates were queried, so one failure in a tiny run does not trip it.",
    min: "0",
  },
  {
    field: "cuvPercent",
    testid: "cuv-percent",
    label: "SISPRO login failure alert (% of invoices)",
    help: "The cuv phase fails when more than this percent of the invoices sent could not log in to SISPRO (no token). 0 disables the alert.",
    min: "0",
    max: "100",
  },
  {
    field: "cuvMinSample",
    testid: "cuv-min-sample",
    label: "SISPRO login alert: minimum invoices sent",
    help: "The alert only applies when at least this many invoices were sent, so one failure in a tiny run does not trip it.",
    min: "0",
  },
];

/** Parameters page: global preparation rules form plus the read-only nota técnica card. */
export class PageParametros extends LitElement {
  static styles = css`
    :host {
      display: block;
    }
    .page {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-4);
      max-width: 48rem;
      margin: 0 auto;
    }
    h2 {
      margin: 0;
      font-size: 1.5rem;
      font-weight: 600;
      color: var(--tx-text-highlighted);
    }
    .lead {
      margin: 0;
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    form {
      display: flex;
      flex-direction: column;
      gap: 1.25rem;
    }
    tx-input {
      width: 100%;
    }
    .actions {
      display: flex;
      justify-content: flex-end;
    }
  `;

  private form = new ParametersFormController(this, api);

  connectedCallback(): void {
    super.connectedCallback();
    document.title = "Parámetros — Tessia";
  }

  private onSubmit(e: Event): void {
    e.preventDefault();
    void this.form.save();
  }

  // The native <input> lives in tx-input's shadow root, so the browser's implicit submission never
  // reaches this <form>: submit on Enter by hand, as a plain form would.
  private onKeydown(e: KeyboardEvent): void {
    if (e.key !== "Enter" || e.isComposing) return;
    if ((e.target as Element | null)?.localName !== "tx-input") return;
    e.preventDefault();
    this.renderRoot.querySelector("form")?.requestSubmit();
  }

  private bindInput(field: Exclude<keyof ParametersFormFields, "selected">) {
    return (e: CustomEvent<{ value: string }>) => this.form.set(field, e.detail.value);
  }

  render() {
    const f = this.form;
    const busy = f.loading || f.saving;
    return html`
      <div class="page">
        <div>
          <h2>Parameters</h2>
          <p class="lead">Configure the global rules used by convenio preparar.</p>
        </div>

        ${f.error
          ? html`<tx-alert color="error" icon="warning-circle" heading="Unable to load or save parameters" description=${f.error} data-testid="form-error"></tx-alert>`
          : nothing}
        ${f.message ? html`<tx-alert color="success" icon="check-circle" heading=${f.message} data-testid="form-message"></tx-alert>` : nothing}
        ${!f.configured && !f.loading
          ? html`<tx-alert
              color="info"
              icon="info-circle"
              heading="Not saved yet. Existing preparation defaults remain active until you save these parameters."
              data-testid="not-configured"
            ></tx-alert>`
          : nothing}

        <tx-card>
          <form novalidate @submit=${this.onSubmit} @keydown=${this.onKeydown}>
            <tx-form-field label="Maximum invoice amount (COP)" help="Invoices above this amount will not be prepared automatically.">
              <tx-input
                .value=${String(f.amount)}
                inputmode="decimal"
                block
                ?disabled=${busy}
                data-testid="amount"
                @tx-input=${this.bindInput("amount")}
              ></tx-input>
            </tx-form-field>

            <tx-form-field
              label="Automatic-preparation subplans"
              description="Select the subplans prepared automatically. Matching ignores case, accents, and repeated spaces."
            >
              <tx-checkbox-group
                .items=${f.options}
                .value=${f.selected}
                ?disabled=${busy}
                data-testid="subplans"
                @tx-change=${(e: CustomEvent<{ value: string[] }>) => f.set("selected", e.detail.value)}
              ></tx-checkbox-group>
            </tx-form-field>

            ${NUMBER_FIELDS.map(
              (spec) => html`<tx-form-field label=${spec.label} help=${spec.help}>
                <tx-input
                  type="number"
                  block
                  min=${spec.min}
                  max=${spec.max ?? nothing}
                  .value=${String(f[spec.field])}
                  ?disabled=${busy}
                  data-testid=${spec.testid}
                  @tx-input=${this.bindInput(spec.field)}
                ></tx-input>
              </tx-form-field>`,
            )}

            <div class="actions">
              <tx-button type="submit" color="primary" ?loading=${f.saving} ?disabled=${f.loading} data-testid="save">Save parameters</tx-button>
            </div>
          </form>
        </tx-card>

        <tx-nota-tecnica></tx-nota-tecnica>
      </div>
    `;
  }
}

if (!customElements.get("tx-page-parametros")) customElements.define("tx-page-parametros", PageParametros);

declare global {
  interface HTMLElementTagNameMap {
    "tx-page-parametros": PageParametros;
  }
}
