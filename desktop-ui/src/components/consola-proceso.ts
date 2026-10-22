import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import { JobPollController } from "../controllers/job-poll-controller";
import { dialogs } from "../utils/dialogs";
import { errorMessage } from "../utils/error-message";
import { statChips } from "../utils/history-stats";
import { formatDuration } from "../utils/job-alert";
import "./convenio-progreso";
import "./fases-proceso";
import "./tx-badge";
import "./tx-button";
import "./tx-card";
import "./tx-icon";

type StateColor = "warning" | "success" | "neutral" | "error";
interface JobState {
  text: string;
  color: StateColor;
  running: boolean;
}

/**
 * Card with the state of the current job. Its `salida` is a short synthetic log, not a console stream:
 * the full verbose output stays in the terminal of `tessia serve`. The card owns its job poll (5 s
 * while a job runs, see JobPollController). `default-open` opens the log on first render; the header
 * (kind, state, elapsed time), the convenio strip and the per-phase cards (queued / in progress / done
 * per pipeline phase, kept as a summary once the job ends) are visible even while collapsed.
 */
export class TxConsolaProceso extends LitElement {
  static properties = {
    defaultOpen: { type: Boolean, attribute: "default-open" },
    open: { state: true },
  };

  static styles = css`
    :host {
      display: block;
    }
    .head {
      display: flex;
      align-items: center;
      gap: var(--tx-space-2);
    }
    h2 {
      flex: 1;
      margin: 0;
      font-size: var(--tx-text-base);
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    h2 button {
      display: inline-flex;
      align-items: center;
      gap: var(--tx-space-1);
      padding: 0;
      border: 0;
      background: none;
      color: inherit;
      font: inherit;
      text-align: left;
      cursor: pointer;
    }
    h2 button:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
    .kind {
      font-size: var(--tx-text-sm);
      font-weight: 400;
      color: var(--tx-text-muted);
    }
    .chevron {
      font-size: 1.25rem;
      transition: transform 0.2s ease;
    }
    .chevron.open {
      transform: rotate(180deg);
    }
    .elapsed {
      display: flex;
      align-items: center;
      gap: var(--tx-space-1);
      font-size: 0.75rem;
      white-space: nowrap;
      color: var(--tx-text-muted);
    }
    .spin {
      animation: spin 1s linear infinite;
    }
    @keyframes spin {
      to {
        transform: rotate(360deg);
      }
    }
    tx-convenio-progreso,
    tx-fases-proceso,
    .chips {
      margin-top: var(--tx-space-3);
    }
    .chips {
      display: flex;
      flex-wrap: wrap;
      gap: var(--tx-space-1);
    }
    #console-body[hidden] {
      display: none;
    }
    pre {
      margin: var(--tx-space-2) 0 0;
      padding: var(--tx-space-2) var(--tx-space-3);
      border-radius: var(--tx-radius);
      background: var(--tx-bg-elevated);
      font: 0.8125rem var(--tx-font-mono);
      white-space: pre-wrap;
    }
    @media (prefers-reduced-motion: reduce) {
      .spin {
        animation: none;
      }
      .chevron {
        transition: none;
      }
    }
  `;

  private poll = new JobPollController(this, api);
  defaultOpen = false;
  open = false;
  private opened = false;

  protected willUpdate(): void {
    // The initial expanded state; afterwards only the header button changes it.
    if (!this.opened) {
      this.opened = true;
      this.open = this.defaultOpen;
    }
  }

  private async cancelar(): Promise<void> {
    if (!(await dialogs.confirmAction("¿Cancelar el proceso en curso? Lo que ya se escribió (Medicar, DB, disco) queda hecho; solo se detiene lo que falta."))) return;
    try {
      await api.cancelJob();
    } catch (e) {
      dialogs.notifyError(errorMessage(e));
    }
  }

  // Compact state shown in the header even while collapsed. The server sets error to
  // "cancelado por el operador" when the operator cancels (same match as the finish alert).
  private jobState(): JobState | null {
    const job = this.poll.data;
    if (!job?.id) return null;
    if (job.corriendo) return { text: "en curso", color: "warning", running: true };
    if (!job.error) return { text: "terminado", color: "success", running: false };
    return /cancelad[oa]|cancell?ed/i.test(job.error)
      ? { text: "cancelado", color: "neutral", running: false }
      : { text: "con error", color: "error", running: false };
  }

  render() {
    const job = this.poll.data;
    const state = this.jobState();
    const kind = job?.id && state ? ` — ${job.tipo} (${state.text})` : "";
    // Elapsed time: while running it moves with each poll (not on a local ticking clock), so the
    // header never looks like a live stream.
    const elapsed = job?.id ? formatDuration(job.iniciado_en, job.corriendo ? new Date(this.poll.polledAt).toISOString() : job.terminado_en) : "";
    // The invoice chips and the convenio strip are end-of-step summaries: while they are still
    // moving the header badge and the log are all the card shows.
    const chips = job && !job.corriendo ? statChips(job.estadisticas) : [];
    const convenio = job?.convenio && job.convenio.estado !== "preparando" ? job.convenio : null;
    // Live per-phase cards: absent until the pipeline reports its first phase.
    const fases = job?.id && job.fases?.length ? job.fases : null;

    return html`
      <tx-card>
        <div class="head">
          <h2>
            <button type="button" aria-expanded=${this.open ? "true" : "false"} aria-controls="console-body" @click=${() => (this.open = !this.open)}>
              <span>Registro del proceso<span class="kind">${kind}</span></span>
              <tx-icon class="chevron ${this.open ? "open" : ""}" name="nav-arrow-down"></tx-icon>
            </button>
          </h2>
          ${elapsed
            ? html`<span class="elapsed"><tx-icon name="clock"></tx-icon><span data-testid="job-elapsed">${elapsed}</span></span>`
            : nothing}
          ${state
            ? html`<tx-badge size="md" variant="subtle" color=${state.color} data-testid="job-state">
                ${state.running ? html`<tx-icon slot="leading" class="spin" name="refresh"></tx-icon>` : nothing}${state.text}
              </tx-badge>`
            : nothing}
          ${job?.cancelable
            ? html`<tx-button size="sm" variant="outline" data-testid="cancel-job" @click=${() => void this.cancelar()}>Cancelar</tx-button>`
            : nothing}
        </div>

        ${convenio ? html`<tx-convenio-progreso .convenio=${convenio}></tx-convenio-progreso>` : nothing}
        ${chips.length
          ? html`<div class="chips">${chips.map((c) => html`<tx-badge size="sm" variant="subtle" color=${c.color} title=${c.hint}>${c.label}: ${c.value}</tx-badge>`)}</div>`
          : nothing}
        ${fases ? html`<tx-fases-proceso .fases=${fases} .radicadas=${job?.radicadas}></tx-fases-proceso>` : nothing}

        <div id="console-body" ?hidden=${!this.open}>
          <pre data-testid="console-output">${job?.id ? job.salida : "sin actividad"}</pre>
        </div>
      </tx-card>
    `;
  }
}

if (!customElements.get("tx-consola-proceso")) customElements.define("tx-consola-proceso", TxConsolaProceso);
