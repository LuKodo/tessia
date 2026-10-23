import { LitElement, css, html } from "lit";

let uid = 0;

/**
 * Tooltip around its default slot (the trigger). Shows on hover and on keyboard focus, hides on
 * leave, blur and Escape.
 *
 * Properties: `text` (whitespace-pre-line, so `\n` breaks lines), `side` (top|bottom|left|right,
 * default top), `delay` (ms before opening, default 0), `disabled`.
 *
 * Accessibility: the element keeps a `role="tooltip"` span as a light-DOM child (slotted into the
 * bubble) and adds its id to the `aria-describedby` of every trigger child, because an id cannot be
 * referenced across a shadow boundary. The bubble is `position: fixed`, placed from the trigger
 * rectangle when it opens, so it is not clipped by `overflow` ancestors. The host is inline-flex so
 * the rectangle hugs the trigger.
 */
export class TxTooltip extends LitElement {
  static properties = {
    text: { type: String },
    side: { type: String },
    delay: { type: Number },
    disabled: { type: Boolean, reflect: true },
    shown: { state: true },
    x: { state: true },
    y: { state: true },
  };

  declare text: string;
  declare side: "top" | "bottom" | "left" | "right";
  declare delay: number;
  declare disabled: boolean;
  declare shown: boolean;
  declare x: number;
  declare y: number;

  static styles = css`
    :host {
      display: inline-flex;
    }
    .bubble {
      position: fixed;
      left: var(--tx-tip-x, 0);
      top: var(--tx-tip-y, 0);
      z-index: 1000;
      box-sizing: border-box;
      max-width: min(20rem, calc(100vw - 1rem));
      padding: var(--tx-space-1) var(--tx-space-2);
      border-radius: var(--tx-radius);
      background: var(--tx-bg-inverted);
      color: var(--tx-text-inverted);
      font-size: var(--tx-text-sm);
      line-height: 1.35;
      white-space: pre-line;
      pointer-events: none;
      visibility: hidden;
      opacity: 0;
    }
    .bubble[data-open] {
      visibility: visible;
      opacity: 1;
    }
    @media (prefers-reduced-motion: no-preference) {
      .bubble {
        transition: opacity 100ms ease-out;
      }
    }
    .bubble[data-side="top"] {
      transform: translate(-50%, calc(-100% - 6px));
    }
    .bubble[data-side="bottom"] {
      transform: translate(-50%, 6px);
    }
    .bubble[data-side="left"] {
      transform: translate(calc(-100% - 6px), -50%);
    }
    .bubble[data-side="right"] {
      transform: translate(6px, -50%);
    }
  `;

  private tipEl: HTMLSpanElement = document.createElement("span");
  private timer: ReturnType<typeof setTimeout> | undefined;
  private observer = new MutationObserver(() => this.syncTriggers());

  constructor() {
    super();
    this.text = "";
    this.side = "top";
    this.delay = 0;
    this.disabled = false;
    this.shown = false;
    this.x = 0;
    this.y = 0;
    this.tipEl.id = `tx-tooltip-${++uid}`;
    this.tipEl.setAttribute("role", "tooltip");
    this.tipEl.slot = "tip";
    this.addEventListener("mouseenter", this.onEnter);
    this.addEventListener("mouseleave", this.hide);
    this.addEventListener("focusin", this.onEnter);
    this.addEventListener("focusout", this.hide);
  }

  connectedCallback(): void {
    super.connectedCallback();
    if (this.tipEl.parentNode !== this) this.append(this.tipEl);
    document.addEventListener("keydown", this.onKeydown);
    this.observer.observe(this, { childList: true });
    this.syncTriggers();
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    document.removeEventListener("keydown", this.onKeydown);
    this.observer.disconnect();
    clearTimeout(this.timer);
    this.shown = false;
  }

  protected willUpdate(): void {
    if (this.tipEl.textContent !== this.text) this.tipEl.textContent = this.text;
  }

  protected updated(changed: Map<string, unknown>): void {
    if (changed.has("text")) this.syncTriggers();
  }

  /** Adds (or removes, when there is no text) the tooltip id in each trigger's aria-describedby. */
  private syncTriggers(): void {
    for (const child of this.children) {
      if (child === this.tipEl) continue;
      const ids = (child.getAttribute("aria-describedby") ?? "").split(/\s+/).filter((id) => id && id !== this.tipEl.id);
      if (this.text) ids.push(this.tipEl.id);
      if (ids.length) child.setAttribute("aria-describedby", ids.join(" "));
      else child.removeAttribute("aria-describedby");
    }
  }

  private onEnter = (): void => {
    if (this.disabled || !this.text) return;
    clearTimeout(this.timer);
    const open = () => {
      const rect = this.getBoundingClientRect();
      const horizontal = this.side === "top" || this.side === "bottom";
      this.x = horizontal ? rect.left + rect.width / 2 : this.side === "left" ? rect.left : rect.right;
      this.y = horizontal ? (this.side === "top" ? rect.top : rect.bottom) : rect.top + rect.height / 2;
      this.shown = true;
    };
    if (this.delay > 0) this.timer = setTimeout(open, this.delay);
    else open();
  };

  private hide = (): void => {
    clearTimeout(this.timer);
    this.shown = false;
  };

  private onKeydown = (event: KeyboardEvent): void => {
    if (event.key === "Escape" && this.shown) this.hide();
  };

  render() {
    return html`<slot></slot
      ><span
        class="bubble"
        data-side=${this.side}
        ?data-open=${this.shown && !this.disabled && this.text !== ""}
        style="--tx-tip-x: ${this.x}px; --tx-tip-y: ${this.y}px"
        ><slot name="tip"></slot
      ></span>`;
  }
}

if (!customElements.get("tx-tooltip")) customElements.define("tx-tooltip", TxTooltip);
