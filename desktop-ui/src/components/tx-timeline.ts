import { LitElement, css, html, nothing } from "lit";
import { icon } from "./shell-icons";

export interface TimelineItem {
  title: string;
  description?: string;
  /** Shown as a `<time>` next to the title; an ISO string keeps `datetime` meaningful. */
  date?: string;
  /** Name from the icon set in `icons.ts`, drawn inside the dot. */
  icon?: string;
  color?: string;
}

const COLORS = ["primary", "neutral", "success", "warning", "error", "info"];
const DOT: Record<string, string> = { xs: "0.5rem", sm: "0.625rem", md: "0.875rem", lg: "1.25rem", xl: "1.75rem" };

/**
 * Vertical timeline: a dot per entry joined by a line.
 *
 * Properties: `items` (`{title, description?, date?, icon?, color?}[]`, set from JS), `color` (default
 * color of the dots without their own, default neutral), `size` (xs..xl, reflected, default md).
 *
 * Slots: `title-<index>` and `description-<index>` (zero based) replace the text of an entry; the
 * item fields are their fallback.
 */
export class TxTimeline extends LitElement {
  static properties = {
    items: { attribute: false },
    color: { type: String },
    size: { type: String, reflect: true },
  };

  declare items: TimelineItem[];
  declare color: string;
  declare size: string;

  static styles = css`
    :host {
      display: block;
      --_c-primary: var(--tx-primary);
      --_c-neutral: var(--tx-text-dimmed);
      --_c-success: var(--tx-success);
      --_c-warning: var(--tx-warning);
      --_c-error: var(--tx-error);
      --_c-info: var(--tx-info);
    }
    ol {
      margin: 0;
      padding: 0;
      list-style: none;
    }
    li {
      position: relative;
      display: grid;
      grid-template-columns: var(--_dot, 0.875rem) 1fr;
      column-gap: var(--tx-space-3);
      padding-bottom: var(--tx-space-4);
    }
    li[data-last] {
      padding-bottom: 0;
    }
    .dot {
      display: grid;
      place-items: center;
      width: var(--_dot, 0.875rem);
      height: var(--_dot, 0.875rem);
      margin-top: 0.2rem;
      border-radius: 50%;
      background: var(--_dot-color);
      color: var(--tx-text-inverted);
    }
    .dot svg {
      width: 70%;
      height: 70%;
    }
    li:not([data-last])::before {
      content: "";
      position: absolute;
      left: calc(var(--_dot, 0.875rem) / 2 - 1px);
      top: calc(0.2rem + var(--_dot, 0.875rem));
      bottom: 0;
      width: 2px;
      background: var(--tx-border-strong);
    }
    .head {
      display: flex;
      flex-wrap: wrap;
      align-items: baseline;
      gap: var(--tx-space-1) var(--tx-space-3);
    }
    .title {
      font-weight: 600;
      color: var(--tx-text-highlighted);
    }
    time {
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
    .description {
      margin-top: 0.125rem;
      font-size: var(--tx-text-sm);
      color: var(--tx-text-muted);
    }
  `;

  // Slotted overrides decide whether an empty description row renders; track the host's children.
  private observer = new MutationObserver(() => this.requestUpdate());

  constructor() {
    super();
    this.items = [];
    this.color = "neutral";
    this.size = "md";
  }

  connectedCallback(): void {
    super.connectedCallback();
    this.observer.observe(this, { childList: true });
  }

  disconnectedCallback(): void {
    super.disconnectedCallback();
    this.observer.disconnect();
  }

  private slotted(name: string): boolean {
    return [...this.children].some((child) => child.getAttribute("slot") === name);
  }

  render() {
    return html`<ol style="--_dot: ${DOT[this.size] ?? DOT.md}">
      ${this.items.map((item, i) => {
        const color = COLORS.includes(item.color ?? "") ? item.color! : this.color;
        const hasDescription = !!item.description || this.slotted(`description-${i}`);
        return html`<li ?data-last=${i === this.items.length - 1}>
          <span class="dot" data-color=${color} style="--_dot-color: var(--_c-${color}, var(--_c-neutral))" aria-hidden="true">
            ${item.icon ? icon(item.icon, 12) : nothing}
          </span>
          <div>
            <div class="head">
              <span class="title"><slot name="title-${i}">${item.title}</slot></span>
              ${item.date ? html`<time datetime=${item.date}>${item.date}</time>` : nothing}
            </div>
            ${hasDescription ? html`<div class="description"><slot name="description-${i}">${item.description}</slot></div>` : nothing}
          </div>
        </li>`;
      })}
    </ol>`;
  }
}

if (!customElements.get("tx-timeline")) customElements.define("tx-timeline", TxTimeline);
