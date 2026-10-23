import { LitElement, css, html } from "lit";
import { api } from "../api";
import { apiBase } from "../config";
import { PollController } from "../controllers/poll-controller";
import type { ServiceHealth } from "../types/api";
import { summarizeHealth } from "../utils/health";
import type { HealthState } from "../utils/health";
import "./tx-badge";
import "./tx-tooltip";

const STYLE: Record<HealthState, { color: string; icon: string }> = {
  ok: { color: "success", icon: "check-circle" },
  degraded: { color: "warning", icon: "warning-circle" },
  unreachable: { color: "error", icon: "wifi-xmark" },
  unknown: { color: "neutral", icon: "help-circle" },
};

/** One-badge summary of GET /api/health for the navbar (polled every 5 s); the tooltip carries the detail. */
export class TxHealthIndicator extends LitElement {
  static styles = css`
    :host {
      display: inline-flex;
    }
  `;

  private health = new PollController<ServiceHealth[]>(this, () => api.health(), 5000, { pauseWhenHidden: true });

  render() {
    const summary = summarizeHealth(this.health.data, this.health.error, { apiBase, origin: window.location.origin });
    const style = STYLE[summary.state];
    return html`<tx-tooltip text=${summary.detail} ?disabled=${!summary.detail} side="bottom">
      <tx-badge color=${style.color} icon=${style.icon} variant="subtle" size="md" data-testid="health-indicator" data-state=${summary.state}
        >${summary.label}</tx-badge
      >
    </tx-tooltip>`;
  }
}

if (!customElements.get("tx-health-indicator")) customElements.define("tx-health-indicator", TxHealthIndicator);
