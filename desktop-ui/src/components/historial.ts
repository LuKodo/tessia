import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import { PollController } from "../controllers/poll-controller";
import type { JobHistoryEntry, JobsHistory } from "../types/api";
import { fmtTime } from "../utils/format";
import { statChips } from "../utils/history-stats";
import { formatDuration } from "../utils/job-alert";
import { statusColor } from "../utils/status-color";
import "./tx-accordion";
import "./tx-badge";
import "./tx-card";

/**
 * Card with the finished jobs (GET /api/jobs/history, polled every 5 s). Each entry is an accordion
 * item showing outcome, times, duration and the invoice stat chips; opening it shows the error (if
 * any) and the final output. Several entries can be open at once and stay open across polls.
 */
/**
 * One unique accordion key per entry. Job ids repeat across server restarts (the counter used to
 * start again at job-1), so the id alone is not enough: add the start time, then a suffix for any
 * remaining duplicate.
 */
export function entryKeys(entries: readonly JobHistoryEntry[]): string[] {
  const seen = new Map<string, number>();
  return entries.map((h) => {
    const base = `${h.id}@${h.iniciado_en}`;
    const n = seen.get(base) ?? 0;
    seen.set(base, n + 1);
    return n === 0 ? base : `${base}#${n}`;
  });
}

export class TxHistorial extends LitElement {
  static properties = { open: { state: true } };

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
    .empty {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .list {
      max-height: 360px;
      overflow-y: auto;
    }
    .entry {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-1);
      min-width: 0;
      flex: 1;
      text-align: left;
    }
    .line {
      display: flex;
      flex-wrap: wrap;
      align-items: center;
      gap: var(--tx-space-1) var(--tx-space-3);
    }
    .tipo {
      font-size: var(--tx-text-sm);
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    .times {
      gap: var(--tx-space-1) var(--tx-space-4);
      font-size: var(--tx-text-sm);
      font-weight: 400;
      color: var(--tx-text);
    }
    .times .muted {
      color: var(--tx-text-muted);
    }
    .chips {
      display: flex;
      flex-wrap: wrap;
      gap: var(--tx-space-1);
    }
    .error {
      margin-bottom: var(--tx-space-2);
      font-size: var(--tx-text-sm);
      color: var(--tx-error);
    }
    pre {
      margin: 0;
      padding: var(--tx-space-2) var(--tx-space-3);
      border-radius: var(--tx-radius);
      background: var(--tx-bg-elevated);
      font: 0.8125rem var(--tx-font-mono);
      white-space: pre-wrap;
    }
  `;

  // `null` until the first job finished (Go nil slice), so the empty state covers both.
  private history = new PollController<JobsHistory>(this, () => api.jobsHistory(), 5000, { pauseWhenHidden: true });
  open: string[] = [];

  private onToggle(event: CustomEvent<{ value: string[] }>): void {
    if (event.target !== event.currentTarget) return; // ignore tx-change bubbling from inside an entry
    this.open = event.detail.value;
  }

  private entry(h: JobHistoryEntry, key: string) {
    const chips = statChips(h.estadisticas);
    const duration = formatDuration(h.iniciado_en, h.terminado_en);
    const outcome = h.error ? "Terminado con error" : "Terminado";
    return html`
      <span slot="label-${key}" class="entry" data-testid="history-entry">
        <span class="line">
          <span class="tipo">${h.tipo}</span>
          <tx-badge size="md" variant="subtle" color=${statusColor(outcome)}>${outcome}</tx-badge>
        </span>
        <span class="line times">
          <span><span class="muted">Inicio:</span> <strong>${fmtTime(h.iniciado_en) || "—"}</strong></span>
          <span><span class="muted">Fin:</span> <strong>${fmtTime(h.terminado_en) || "—"}</strong></span>
          ${duration ? html`<span><span class="muted">Duración:</span> ${duration}</span>` : nothing}
        </span>
        ${chips.length
          ? html`<span class="chips"
              >${chips.map((c) => html`<tx-badge size="sm" variant="subtle" color=${c.color} title=${c.hint}>${c.label}: ${c.value}</tx-badge>`)}</span
            >`
          : nothing}
      </span>
      ${this.open.includes(key)
        ? html`<div slot=${key}>
            ${h.error ? html`<div class="error">${h.error}</div>` : nothing}
            <pre>${h.salida_final || "(sin salida)"}</pre>
          </div>`
        : nothing}
    `;
  }

  render() {
    const entries = this.history.data;
    const keys = entryKeys(entries ?? []);
    return html`
      <tx-card>
        <h2>Historial de procesos</h2>
        ${!entries || !entries.length
          ? html`<div class="empty" data-testid="history-empty">sin historial todavía</div>`
          : html`<div class="list">
              <tx-accordion
                type="multiple"
                .items=${keys.map((value, i) => ({ value, label: entries[i]!.tipo }))}
                .value=${this.open}
                @tx-change=${this.onToggle}
                >${entries.map((h, i) => this.entry(h, keys[i]!))}</tx-accordion
              >
            </div>`}
      </tx-card>
    `;
  }
}

if (!customElements.get("tx-historial")) customElements.define("tx-historial", TxHistorial);
