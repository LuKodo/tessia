import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import { PollController } from "../controllers/poll-controller";
import type { Cargue } from "../types/api";
import { dialogs } from "../utils/dialogs";
import { errorMessage } from "../utils/error-message";
import { messageClass, type UiMessage } from "../utils/message-class";
import { PHASES } from "../utils/phases";
import { statusColor } from "../utils/status-color";
import "./tx-badge";
import "./tx-button";
import "./tx-card";
import "./tx-form-field";
import "./tx-input";
import "./tx-select";
import "./tx-select-menu";

// "(completo)" travels as a sentinel value and maps back to "" (kept from the old select, which
// rejected an empty item value).
const ALL_PHASES = "completo";
const PHASE_OPTIONS = [{ value: ALL_PHASES, label: "(completo)" }, ...PHASES.map(([value, label]) => ({ value, label }))];

/**
 * "Ejecutar proceso" card: pick a cargue (polled every 5 s), an optional phase range and a limit, then
 * run the whole process or a section after the operator confirms. The card owns its cargue list and
 * selection; nothing is shared with sibling cards.
 */
export class TxProcesoPanel extends LitElement {
  static properties = {
    selected: { state: true },
    from: { state: true },
    to: { state: true },
    limit: { state: true },
    status: { state: true },
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
      align-items: flex-start;
      gap: var(--tx-space-3);
    }
    .row > tx-form-field {
      flex: 1 1 160px;
      min-width: 140px;
    }
    .row > .cargue {
      flex-basis: 260px;
      min-width: 220px;
    }
    .row > .limit {
      flex-basis: 320px;
      min-width: 260px;
    }
    .row > tx-button,
    .row > .status {
      margin-top: 1.625rem;
    }
    .status {
      display: flex;
      align-items: center;
      min-height: 2.5rem;
      font-size: var(--tx-text-sm);
    }
    .summary {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-2);
      margin-top: var(--tx-space-3);
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
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

  // This panel owns its cargue list and selection.
  private cargues = new PollController<Cargue[]>(this, () => api.cargues(), 5000, { pauseWhenHidden: true });
  selected: string | null = null;
  from = "";
  to = "";
  // Raw text of the limit input; `Number(text) || 0` is what the API receives.
  limit = "0";
  status: UiMessage = { text: "", kind: "" };

  // Keep the selection valid: default to the first cargue, and fall back to it if the selected
  // one disappears from the list.
  protected willUpdate(): void {
    const list = this.cargues.data;
    const first = list?.[0];
    if (!first) return;
    if (this.selected === null || !list.some((c) => c.id === this.selected)) this.selected = first.id;
  }

  private get current(): Cargue | undefined {
    return (this.cargues.data || []).find((c) => c.id === this.selected);
  }

  private async runPipeline(fromPhase: string, toPhase: string): Promise<void> {
    if (!this.selected) {
      this.status = { text: "selecciona un cargue primero", kind: "error" };
      return;
    }
    const limit = Number(this.limit) || 0;
    if (!Number.isSafeInteger(limit) || limit < 0) {
      this.status = { text: "El límite debe ser un entero mayor o igual a 0.", kind: "error" };
      return;
    }
    const rango = fromPhase || toPhase ? `las fases ${fromPhase || "(inicio)"} → ${toPhase || "(fin)"}` : "el proceso COMPLETO";
    const preparation = this.current?.estado === "SIN_PREPARAR" ? " Primero se preparará el convenio desde su CSV." : "";
    const cargue = this.selected;
    if (!(await dialogs.confirmAction(`¿Ejecutar ${rango} sobre "${cargue}"?${preparation} Esto escribe de verdad (Medicar, SISPRO, Siesa, MySQL, disco).`))) return;
    this.status = { text: "iniciando…", kind: "" };
    try {
      // Capitalized keys: the Go request struct has no json tags.
      await api.runJob({ Cargue: cargue, From: fromPhase, To: toPhase, Limit: limit });
      this.status = { text: "en curso — sigue el progreso justo abajo", kind: "ok" };
    } catch (e) {
      this.status = { text: errorMessage(e), kind: "error" };
    }
  }

  private summary(current: Cargue) {
    const detail =
      current.estado === "SIN_PREPARAR"
        ? "Pendiente de preparación; iniciar el proceso prepara el CSV primero"
        : `${current.facturas} preparada(s) · ${current.excluidas} excluida(s) · ${current.revision_manual} en revisión manual`;
    return html`
      <div class="summary">
        <tx-badge size="md" variant="subtle" color=${statusColor(current.estado)}>${current.estado}</tx-badge>
        <span>${current.fecha_inicio || ""}${current.fecha_fin ? ` → ${current.fecha_fin}` : ""} · ${detail}</span>
        <tx-button
          size="sm"
          variant="link"
          data-testid="open-informe"
          href=${`/revision-manual?cargue=${encodeURIComponent(this.selected || "")}`}
          ?disabled=${!this.selected}
          >Informe de revisión manual</tx-button
        >
      </div>
    `;
  }

  render() {
    const list = this.cargues.data;
    const options = (list || []).map((c) => ({ value: c.id, label: c.nombre || c.id }));
    const current = this.current;
    return html`
      <tx-card>
        <h2>Ejecutar proceso</h2>
        <div class="row">
          <tx-form-field class="cargue" label="Cargue">
            <tx-select-menu
              block
              data-testid="cargue-select"
              value-key="value"
              label-key="label"
              placeholder=${list ? "sin cargues en paths.automatic" : "cargando…"}
              .items=${options}
              .value=${this.selected ?? undefined}
              @tx-change=${(e: CustomEvent<{ value?: string }>) => (this.selected = e.detail.value ?? null)}
            ></tx-select-menu>
          </tx-form-field>
          <tx-form-field label="Desde fase">
            <tx-select
              block
              data-testid="from-phase"
              .items=${PHASE_OPTIONS}
              .value=${this.from || ALL_PHASES}
              @tx-change=${(e: CustomEvent<{ value: string }>) => (this.from = e.detail.value === ALL_PHASES ? "" : e.detail.value)}
            ></tx-select>
          </tx-form-field>
          <tx-form-field label="Hasta fase">
            <tx-select
              block
              data-testid="to-phase"
              .items=${PHASE_OPTIONS}
              .value=${this.to || ALL_PHASES}
              @tx-change=${(e: CustomEvent<{ value: string }>) => (this.to = e.detail.value === ALL_PHASES ? "" : e.detail.value)}
            ></tx-select>
          </tx-form-field>
          <tx-form-field
            class="limit"
            label="Límite de facturas (0 = todas)"
            help="Cuenta solo facturas preparadas: las excluidas se reemplazan por las siguientes."
          >
            <tx-input
              block
              type="number"
              min="0"
              step="1"
              data-testid="limit"
              .value=${this.limit}
              @tx-input=${(e: CustomEvent<{ value: string }>) => (this.limit = e.detail.value)}
            ></tx-input>
          </tx-form-field>
          <tx-button data-testid="run-full" @click=${() => void this.runPipeline("", "")}>Proceso completo</tx-button>
          <tx-button variant="outline" data-testid="run-section" @click=${() => void this.runPipeline(this.from, this.to)}>Ejecutar sección</tx-button>
          ${this.status.text
            ? html`<span class="status ${messageClass(this.status.kind)}" data-testid="run-status">${this.status.text}</span>`
            : nothing}
        </div>
        ${current ? this.summary(current) : nothing}
      </tx-card>
    `;
  }
}

if (!customElements.get("tx-proceso-panel")) customElements.define("tx-proceso-panel", TxProcesoPanel);
