import { LitElement, css } from "lit";

/**
 * Loading placeholder block. Size it from outside (`width`, `height`, `flex`, `border-radius`);
 * the default is a one-line bar. `circle` makes it round (avatars). Decorative: `aria-hidden` unless the
 * consumer set it; the container should announce the loading state (e.g. `aria-busy`).
 */
export class TxSkeleton extends LitElement {
  static properties = { circle: { type: Boolean, reflect: true } };

  static styles = css`
    :host {
      display: block;
      min-height: 1rem;
      border-radius: var(--tx-radius);
      background: var(--tx-bg-muted);
      animation: pulse 2s cubic-bezier(0.4, 0, 0.6, 1) infinite;
    }
    :host([circle]) {
      width: 2.5rem;
      height: 2.5rem;
      border-radius: 9999px;
    }
    @keyframes pulse {
      50% {
        opacity: 0.5;
      }
    }
    @media (prefers-reduced-motion: reduce) {
      :host {
        animation: none;
      }
    }
  `;

  circle = false;

  connectedCallback(): void {
    super.connectedCallback();
    if (!this.hasAttribute("aria-hidden")) this.setAttribute("aria-hidden", "true");
  }
}

if (!customElements.get("tx-skeleton")) customElements.define("tx-skeleton", TxSkeleton);
