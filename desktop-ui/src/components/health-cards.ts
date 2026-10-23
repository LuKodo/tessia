import { LitElement, css, html, nothing } from "lit";
import type { TemplateResult } from "lit";
import { api } from "../api";
import { apiBase } from "../config";
import { PollController } from "../controllers/poll-controller";
import type { NotaTecnicaStatus, ServiceHealth } from "../types/api";
import { connectivityHint } from "../utils/connectivity-hint";
import { errorMessage } from "../utils/error-message";
import { healthDetail, healthState, notaTecnicaDetail, notaTecnicaState } from "../utils/health-model";
import type { HealthCardState } from "../utils/health-model";
import "./tx-avatar";
import "./tx-badge";
import "./tx-card";
import "./tx-skeleton";

// health-model returns icon keys; the registry names live here.
const ICON = { ok: "check-circle", down: "xmark-circle", unknown: "help-circle" };

/** One card per service of GET /api/health (polled every 5 s) plus the nota técnica card (every 60 s). */
export class TxHealthCards extends LitElement {
  static styles = css`
    :host {
      display: grid;
      grid-template-columns: 1fr;
      gap: var(--tx-space-3);
      padding: var(--tx-space-4) var(--tx-space-4) 0;
    }
    @media (min-width: 640px) {
      :host {
        grid-template-columns: repeat(2, 1fr);
      }
    }
    @media (min-width: 1024px) {
      :host {
        grid-template-columns: repeat(3, 1fr);
      }
    }
    tx-card {
      border-left: 4px solid var(--tx-border-strong);
    }
    tx-card[data-state="ok"] {
      border-left-color: var(--tx-success);
    }
    tx-card[data-state="down"] {
      border-left-color: var(--tx-error);
    }
    .row {
      display: flex;
      align-items: flex-start;
      gap: var(--tx-space-3);
    }
    .row.center {
      align-items: center;
    }
    tx-avatar {
      flex: none;
    }
    .main {
      min-width: 0;
      flex: 1;
    }
    .head {
      display: flex;
      align-items: center;
      justify-content: space-between;
      gap: var(--tx-space-2);
    }
    .name {
      font-weight: 700;
      overflow-wrap: break-word;
      color: var(--tx-text-highlighted);
    }
    tx-badge {
      flex: none;
    }
    .detail {
      margin-top: 0.25rem;
      font-size: 0.75rem;
      overflow-wrap: break-word;
      color: var(--tx-text-muted);
    }
    .detail.plain {
      margin: 0;
    }
    .skeleton-line {
      flex: 1;
      height: 1.25rem;
    }
  `;

  // A failed tick keeps the last known cards: PollController does not clear `data` on error.
  private services = new PollController<ServiceHealth[]>(this, () => api.health(), 5000, { pauseWhenHidden: true });
  // The Nota Técnica published in Redis changes rarely, so a slow poll is enough.
  private nota = new PollController<NotaTecnicaStatus>(this, () => api.notaTecnica(), 60000, { pauseWhenHidden: true });

  private card(name: string, state: HealthCardState, detail: string): TemplateResult {
    const color = state.color;
    return html`<tx-card padding="sm" data-state=${state.key} data-testid="health-card">
      <div class="row">
        <tx-avatar size="xl" color=${color} icon=${ICON[state.icon]}></tx-avatar>
        <div class="main">
          <div class="head">
            <div class="name">${name}</div>
            ${state.label ? html`<tx-badge size="sm" variant="subtle" color=${color}>${state.label}</tx-badge>` : nothing}
          </div>
          <div class="detail">${detail}</div>
        </div>
      </div>
    </tx-card>`;
  }

  private placeholder(): TemplateResult {
    return html`<tx-card padding="sm">
      <div class="row center"><tx-skeleton circle></tx-skeleton><tx-skeleton class="skeleton-line"></tx-skeleton></div>
    </tx-card>`;
  }

  render() {
    const services = this.services.data;
    const error = this.services.error;
    const nota = this.nota.data;
    const notaError = this.nota.error;
    const hint = connectivityHint(error, apiBase, window.location.origin);
    return html`
      ${(services || []).map((s) => this.card(s.nombre, healthState(s), healthDetail(s)))}
      <!-- First load: neutral placeholders until the first /api/health answer. -->
      ${!services && !error ? [1, 2, 3, 4, 5].map(() => this.placeholder()) : nothing}
      <!-- No answer yet and the first request failed: show why, in the failing style. -->
      ${!services && error
        ? html`<tx-card padding="sm" data-state="down" data-testid="health-card">
            <div class="row">
              <tx-avatar size="xl" color="error" icon=${ICON.down}></tx-avatar>
              <div class="main">
                <div class="detail plain">${errorMessage(error)}</div>
                ${hint ? html`<div class="detail" data-testid="connectivity-hint">${hint}</div>` : nothing}
              </div>
            </div>
          </tx-card>`
        : nothing}
      <!-- Nota técnica: informational card (read-only Redis catalog: version and rows). -->
      ${nota
        ? this.card("Nota técnica", notaTecnicaState(nota), notaTecnicaDetail(nota))
        : !notaError
          ? this.placeholder()
          : html`<tx-card padding="sm" data-state="down" data-testid="health-card">
              <div class="row">
                <tx-avatar size="xl" color="error" icon=${ICON.down}></tx-avatar>
                <div class="main">
                  <div class="name">Nota técnica</div>
                  <div class="detail">${errorMessage(notaError)}</div>
                </div>
              </div>
            </tx-card>`}
    `;
  }
}

if (!customElements.get("tx-health-cards")) customElements.define("tx-health-cards", TxHealthCards);
