import { LitElement, css, html, svg, nothing } from "lit";
import { DialogsController } from "../controllers/dialogs-controller";
import "./tx-modal";

const WARNING = svg`<path d="M12 3 2.5 20h19L12 3Z"/><path d="M12 10v4.5"/><path d="M12 17.5v.01"/>`;
const ERROR = svg`<circle cx="12" cy="12" r="9.5"/><path d="m9 9 6 6M15 9l-6 6"/>`;

/**
 * Renders the pending confirm and error dialogs of the shared dialogs service (port of
 * AppDialogs.vue). Mount one instance in the app shell. The confirmation is not dismissible:
 * Escape and backdrop clicks keep it open and only its two buttons answer it.
 */
export class TxAppDialogs extends LitElement {
  private dialogs = new DialogsController(this);

  static styles = css`
    .content {
      display: flex;
      flex-direction: column;
      align-items: center;
      gap: var(--tx-space-2);
      padding: 2rem var(--tx-space-6) var(--tx-space-6);
      text-align: center;
    }
    svg {
      width: 3.5rem;
      height: 3.5rem;
      fill: none;
      stroke: currentColor;
      stroke-width: 1.5;
      stroke-linecap: round;
      stroke-linejoin: round;
    }
    svg.warning {
      color: var(--tx-warning);
    }
    svg.error {
      color: var(--tx-error);
    }
    h2 {
      margin: var(--tx-space-1) 0 0;
      font-size: var(--tx-text-xl);
      font-weight: 600;
      color: var(--tx-text-highlighted);
    }
    p {
      margin: 0;
      color: var(--tx-text);
      white-space: pre-line;
    }
    .buttons {
      display: flex;
      justify-content: center;
      gap: var(--tx-space-2);
      margin-top: var(--tx-space-4);
    }
    button {
      padding: var(--tx-space-2) var(--tx-space-4);
      border: 1px solid transparent;
      border-radius: var(--tx-radius);
      font: inherit;
      font-weight: 500;
      cursor: pointer;
    }
    button:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
    button.primary {
      background: var(--tx-primary);
      color: var(--tx-text-inverted);
    }
    button.subtle {
      background: var(--tx-bg-muted);
      border-color: var(--tx-border-strong);
      color: var(--tx-text-highlighted);
    }
    button:hover {
      filter: brightness(0.95);
    }
  `;

  render() {
    const confirm = this.dialogs.confirm;
    const error = this.dialogs.error;
    return html`
      <tx-modal data-kind="confirm" bare size="sm" .open=${confirm !== null} .dismissible=${false} .label=${confirm?.title ?? ""}>
        ${confirm
          ? html`<div class="content" data-testid="confirm-dialog">
              <svg class="warning" viewBox="0 0 24 24" aria-hidden="true">${WARNING}</svg>
              <h2>${confirm.title}</h2>
              <p>${confirm.text}</p>
              <div class="buttons">
                <button class="subtle" type="button" autofocus data-testid="confirm-cancel" @click=${() => this.dialogs.answerConfirm(false)}>
                  Cancelar
                </button>
                <button class="primary" type="button" data-testid="confirm-accept" @click=${() => this.dialogs.answerConfirm(true)}>
                  ${confirm.confirmText}
                </button>
              </div>
            </div>`
          : nothing}
      </tx-modal>
      <tx-modal
        data-kind="error"
        bare
        size="sm"
        .open=${error !== null}
        .label=${error?.title ?? ""}
        @tx-open-change=${() => this.dialogs.dismissError()}
      >
        ${error
          ? html`<div class="content" data-testid="error-dialog">
              <svg class="error" viewBox="0 0 24 24" aria-hidden="true">${ERROR}</svg>
              <h2>${error.title}</h2>
              <p>${error.text}</p>
              <div class="buttons">
                <button class="primary" type="button" autofocus data-testid="error-ok" @click=${() => this.dialogs.dismissError()}>OK</button>
              </div>
            </div>`
          : nothing}
      </tx-modal>
    `;
  }
}

if (!customElements.get("tx-app-dialogs")) customElements.define("tx-app-dialogs", TxAppDialogs);
