import { LitElement, css, html } from "lit";
import "../components/health-cards";
import "../components/consola-proceso";
import "../components/nuevo-cargue";
import "../components/proceso-panel";
import "../components/rips/rips-dashboard";
import "../components/tx-tabs";

const TITLE = "Dashboard — Tessia";

// "Operar" (health cards + new cargue / process panel / process log) and "Analizar" (RIPS dashboard).
const TABS = [
  { label: "Operar", value: "operate" },
  { label: "Analizar", value: "analyze" },
];

/**
 * `/`: the operator's dashboard. Each tab mounts its content only while selected (as the old
 * `UTabs` did), so the polls of the hidden tab stop.
 */
export class TxPageDashboard extends LitElement {
  static properties = { tab: { state: true } };

  static styles = css`
    :host {
      display: block;
    }
    .operate {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-3);
      padding: var(--tx-space-4);
    }
  `;

  tab = "operate";

  private previousTitle: string | null = null;

  connectedCallback(): void {
    super.connectedCallback();
    this.previousTitle = document.title;
    document.title = TITLE;
  }

  disconnectedCallback(): void {
    // Only undo our own title: another page may have set its own already.
    if (this.previousTitle !== null && document.title === TITLE) document.title = this.previousTitle;
    this.previousTitle = null;
    super.disconnectedCallback();
  }

  // `tx-change` bubbles from every control inside a tab too (the cargue select, the filters...):
  // only the event of the tabs element itself is a tab change.
  private onTabChange = (e: Event) => {
    if (e.target !== e.currentTarget) return;
    this.tab = (e as CustomEvent<{ value: string }>).detail.value;
  };

  render() {
    return html`<tx-tabs .items=${TABS} .value=${this.tab} @tx-change=${this.onTabChange}>
      <div slot="operate">
        ${this.tab === "operate"
          ? html`<tx-health-cards></tx-health-cards>
              <div class="operate">
                <tx-nuevo-cargue></tx-nuevo-cargue>
                <tx-proceso-panel></tx-proceso-panel>
                <tx-consola-proceso default-open></tx-consola-proceso>
              </div>`
          : null}
      </div>
      <div slot="analyze">${this.tab === "analyze" ? html`<rips-dashboard></rips-dashboard>` : null}</div>
    </tx-tabs>`;
  }
}

if (!customElements.get("tx-page-dashboard")) customElements.define("tx-page-dashboard", TxPageDashboard);
