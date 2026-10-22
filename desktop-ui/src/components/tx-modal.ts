import { LitElement, css, html, nothing } from "lit";
import { icon } from "./shell-icons";

const SIZES: Record<string, string> = { xs: "22rem", sm: "27.5rem", md: "32rem", lg: "48rem", xl: "56rem" };

// Several modals can be open at once (confirm over a detail): the page unlocks with the last one.
let locks = 0;
let previousOverflow = "";
function lockScroll(): void {
  if (locks++ === 0) {
    previousOverflow = document.documentElement.style.overflow;
    document.documentElement.style.overflow = "hidden";
  }
}
function unlockScroll(): void {
  if (locks > 0 && --locks === 0) document.documentElement.style.overflow = previousOverflow;
}

/** The focused element even when it lives inside nested shadow roots. */
function deepActiveElement(): HTMLElement | null {
  let el = document.activeElement as HTMLElement | null;
  while (el?.shadowRoot?.activeElement) el = el.shadowRoot.activeElement as HTMLElement;
  return el;
}

let uid = 0;

/**
 * Modal dialog on top of a native `<dialog>` opened with `showModal()`: the browser provides the
 * top layer, the inert page behind it (the focus trap) and the backdrop. The element adds the
 * Escape/backdrop handling, focus restore, scroll lock and the slots.
 *
 * Properties: `open`, `title`, `description`, `label` (aria-label when there is no visible title),
 * `size` (xs|sm|md|lg|xl: 22, 27.5, 32, 48, 56rem), `width` (any CSS length, wins over `size`),
 * `dismissible` (default true: Escape, backdrop and the close button request a close),
 * `bare` (content only: no header, close button or footer).
 *
 * Slots: default or `body`, `title`, `description`, `actions` (header, before the close button),
 * `footer` (rendered only when something is slotted).
 *
 * Events (bubbling, composed): `tx-open-change` {open: false}, cancelable, fired when the user asks
 * to close; if nobody calls `preventDefault()` the element closes itself (`open = false`) and then
 * fires `tx-close`. Changing `open` from code never fires events.
 */
export class TxModal extends LitElement {
  static properties = {
    open: { type: Boolean, reflect: true },
    title: { type: String },
    description: { type: String },
    label: { type: String },
    size: { type: String },
    width: { type: String },
    dismissible: { converter: { fromAttribute: (v: string | null) => v !== "false" } },
    bare: { type: Boolean },
  };

  declare open: boolean;
  declare title: string;
  declare description: string;
  declare label: string;
  declare size: string;
  declare width: string;
  declare dismissible: boolean;
  declare bare: boolean;

  static styles = css`
    :host {
      display: contents;
    }
    dialog {
      box-sizing: border-box;
      width: min(var(--tx-modal-width, 32rem), calc(100vw - 2rem));
      max-height: calc(100dvh - 2rem);
      padding: 0;
      border: 1px solid var(--tx-border);
      border-radius: var(--tx-radius-lg);
      background: var(--tx-bg-elevated);
      color: var(--tx-text);
      box-shadow: 0 10px 30px light-dark(rgb(0 0 0 / 0.15), rgb(0 0 0 / 0.6));
      overflow: hidden;
    }
    dialog:not([open]) {
      display: none;
    }
    dialog[open] {
      display: block;
    }
    dialog::backdrop {
      background: rgb(0 0 0 / 0.5);
    }
    @media (prefers-reduced-motion: no-preference) {
      dialog[open] {
        animation: tx-modal-in 140ms ease-out;
      }
      dialog[open]::backdrop {
        animation: tx-modal-fade 140ms ease-out;
      }
    }
    @keyframes tx-modal-in {
      from {
        opacity: 0;
        transform: translateY(0.5rem) scale(0.98);
      }
    }
    @keyframes tx-modal-fade {
      from {
        opacity: 0;
      }
    }
    .panel {
      display: flex;
      flex-direction: column;
      max-height: calc(100dvh - 2rem);
    }
    header {
      display: flex;
      align-items: flex-start;
      gap: var(--tx-space-3);
      padding: var(--tx-space-4) var(--tx-space-6);
      border-bottom: 1px solid var(--tx-border);
    }
    .heading {
      flex: 1;
      min-width: 0;
    }
    .title {
      font-size: var(--tx-text-lg);
      font-weight: 600;
      color: var(--tx-text-highlighted);
    }
    .description {
      margin-top: var(--tx-space-1);
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .actions {
      display: flex;
      align-items: center;
    }
    button.close {
      display: inline-grid;
      place-items: center;
      width: 2rem;
      height: 2rem;
      padding: 0;
      border: 0;
      border-radius: var(--tx-radius);
      background: transparent;
      color: var(--tx-text-muted);
      cursor: pointer;
    }
    button.close:hover {
      background: var(--tx-bg-muted);
      color: var(--tx-text-highlighted);
    }
    button.close:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 1px;
    }
    .body {
      flex: 1;
      min-height: 0;
      overflow: auto;
      padding: var(--tx-space-4) var(--tx-space-6);
    }
    .bare .body {
      padding: 0;
    }
    footer {
      padding: var(--tx-space-4) var(--tx-space-6);
      border-top: 1px solid var(--tx-border);
    }
  `;

  private opener: HTMLElement | null = null;
  private locked = false;
  private uid = ++uid;

  constructor() {
    super();
    this.open = false;
    this.title = "";
    this.description = "";
    this.label = "";
    this.size = "md";
    this.width = "";
    this.dismissible = true;
    this.bare = false;
  }

  // Slotted `title`/`description`/`footer` decide which regions render; re-render when they come or go.
  private observer = new MutationObserver(() => this.requestUpdate());

  connectedCallback(): void {
    super.connectedCallback();
    this.observer.observe(this, { childList: true });
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.observer.disconnect();
    this.release();
  }

  protected updated(changed: Map<string, unknown>): void {
    if (!changed.has("open")) return;
    const dialog = this.renderRoot.querySelector("dialog");
    if (!dialog) return;
    if (this.open) {
      if (!dialog.open) {
        this.opener = deepActiveElement();
        dialog.showModal();
      }
      if (!this.locked) {
        this.locked = true;
        lockScroll();
      }
      const wanted = this.querySelector<HTMLElement>("[autofocus]");
      wanted?.focus();
    } else {
      if (dialog.open) dialog.close();
      this.release();
      const back = this.opener;
      this.opener = null;
      if (back?.isConnected) back.focus();
    }
  }

  private release(): void {
    if (!this.locked) return;
    this.locked = false;
    unlockScroll();
  }

  /** The user asked to close (Escape, backdrop, close button): let the parent veto, else close. */
  private requestClose(): void {
    if (!this.dismissible) return;
    const accepted = this.dispatchEvent(
      new CustomEvent("tx-open-change", { detail: { open: false }, bubbles: true, composed: true, cancelable: true }),
    );
    if (!accepted) return;
    this.open = false;
    this.dispatchEvent(new CustomEvent("tx-close", { bubbles: true, composed: true }));
  }

  private onCancel = (event: Event): void => {
    event.preventDefault(); // the element decides: the native close would desync `open`
    this.requestClose();
  };

  private onDialogClick = (event: MouseEvent): void => {
    // The panel fills the dialog, so a click that targets the dialog itself landed on the backdrop.
    if (event.target === event.currentTarget) this.requestClose();
  };

  private slotted(name: string): boolean {
    return [...this.children].some((child) => child.getAttribute("slot") === name);
  }

  render() {
    const titleId = `tx-modal-title-${this.uid}`;
    const descId = `tx-modal-desc-${this.uid}`;
    const hasTitle = !this.bare && (this.title !== "" || this.slotted("title"));
    const hasDescription = !this.bare && (this.description !== "" || this.slotted("description"));
    const width = this.width || SIZES[this.size] || SIZES.md!;
    return html`
      <dialog
        title=""
        style="--tx-modal-width: ${width}"
        aria-modal="true"
        aria-labelledby=${hasTitle ? titleId : nothing}
        aria-label=${hasTitle ? nothing : this.label || this.title || nothing}
        aria-describedby=${hasDescription ? descId : nothing}
        @cancel=${this.onCancel}
        @click=${this.onDialogClick}
      >
        <div class="panel ${this.bare ? "bare" : ""}">
          ${this.bare
            ? nothing
            : html`<header>
                <div class="heading">
                  <div class="title" id=${titleId}><slot name="title">${this.title}</slot></div>
                  <div class="description" id=${descId}>
                    <slot name="description">${this.description}</slot>
                  </div>
                </div>
                <div class="actions">
                  <slot name="actions"></slot>
                  ${this.dismissible
                    ? html`<button class="close" type="button" aria-label="Cerrar" @click=${() => this.requestClose()}>
                        ${icon("close", 18)}
                      </button>`
                    : nothing}
                </div>
              </header>`}
          <div class="body"><slot name="body"><slot></slot></slot></div>
          ${this.bare || !this.slotted("footer")
            ? nothing
            : html`<footer><slot name="footer"></slot></footer>`}
        </div>
      </dialog>
    `;
  }
}

if (!customElements.get("tx-modal")) customElements.define("tx-modal", TxModal);
