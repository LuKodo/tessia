import { LitElement, css, html, nothing } from "lit";
import { api } from "../api";
import { JobAlertController } from "../controllers/job-alert-controller";
import { getRouter } from "../router/router";
import type { AlertOutcome } from "../utils/job-alert";
import "./tx-alert";
import "./tx-button";

const STYLE: Record<AlertOutcome, { color: "success" | "error" | "warning"; icon: string }> = {
  ok: { color: "success", icon: "check-circle" },
  error: { color: "error", icon: "warning-circle" },
  cancelado: { color: "warning", icon: "square" },
  interrumpido: { color: "warning", icon: "warning-circle" },
};

/**
 * Global "a process finished" alert. It lives in the app shell, outside the page outlet, so it shows on
 * every page. It stays until the operator closes it (a finish is never missed); "Ver log" jumps to the
 * history page and closes it.
 */
export class TxJobAlertSnackbar extends LitElement {
  static styles = css`
    :host {
      position: fixed;
      right: var(--tx-space-4);
      bottom: var(--tx-space-4);
      z-index: 50;
      width: min(24rem, calc(100vw - 2 * var(--tx-space-4)));
    }
    tx-alert {
      box-shadow: var(--tx-shadow);
    }
  `;

  private job = new JobAlertController(this, api);

  private openLogs(): void {
    getRouter().navigate("/ejecuciones");
    this.job.dismiss();
  }

  render() {
    const alert = this.job.alert;
    if (!alert) return nothing;
    const style = STYLE[alert.outcome];
    return html`
      <tx-alert
        variant="subtle"
        closable
        color=${style.color}
        icon=${style.icon}
        heading=${alert.message}
        description=${alert.durationText ? `Duración: ${alert.durationText}` : ""}
        @close=${() => this.job.dismiss()}
      >
        <tx-button slot="actions" size="xs" variant="outline" color="neutral" @click=${() => this.openLogs()}>Ver log</tx-button>
      </tx-alert>
    `;
  }
}

if (!customElements.get("tx-job-alert-snackbar")) customElements.define("tx-job-alert-snackbar", TxJobAlertSnackbar);
