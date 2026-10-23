import { LitElement, css, html, svg } from "lit";

export interface AccordionItem {
  value: string;
  label: string;
  disabled?: boolean;
}

const CHEVRON = svg`<path d="m6 9 6 6 6-6"/>`;
const booleanDefaultTrue = { fromAttribute: (v: string | null) => v !== "false" };

let uid = 0;

/**
 * Disclosure list. Each item is a heading with a button trigger (`aria-expanded`/`aria-controls`)
 * and a labelled region.
 *
 * Properties: `items` (`{value, label, disabled?}[]`, set from JS), `type` (single|multiple, default
 * single), `collapsible` (single mode only: may the open item be closed, default true), `value`
 * (array of open item values; the array is never mutated, a new one is assigned on change).
 *
 * Slots: the body of an item goes in a child with `slot="<item value>"`; a child with
 * `slot="label-<item value>"` replaces the label text inside the trigger (the label is its fallback).
 *
 * Events (bubbling, composed): `tx-change` {value: string[]} with the open values after a toggle.
 * Keys on a trigger: ArrowDown/ArrowUp (wrapping), Home, End move focus between enabled triggers.
 */
export class TxAccordion extends LitElement {
  static properties = {
    items: { attribute: false },
    type: { type: String },
    collapsible: { converter: booleanDefaultTrue },
    value: { attribute: false },
  };

  declare items: AccordionItem[];
  declare type: "single" | "multiple";
  declare collapsible: boolean;
  declare value: string[];

  static styles = css`
    :host {
      display: block;
    }
    .item {
      border-bottom: 1px solid var(--tx-border);
    }
    h3 {
      margin: 0;
      font: inherit;
    }
    button.trigger {
      display: flex;
      align-items: flex-start;
      gap: var(--tx-space-3);
      width: 100%;
      padding: var(--tx-space-3) var(--tx-space-1);
      border: 0;
      background: transparent;
      color: var(--tx-text-highlighted);
      font: inherit;
      font-weight: 500;
      text-align: left;
      cursor: pointer;
    }
    button.trigger:disabled {
      opacity: 0.5;
      cursor: not-allowed;
    }
    button.trigger:focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: -2px;
    }
    .label {
      flex: 1;
      min-width: 0;
    }
    svg {
      flex: none;
      width: 1.25rem;
      height: 1.25rem;
      margin-top: 0.1rem;
      fill: none;
      stroke: var(--tx-text-muted);
      stroke-width: 1.75;
      stroke-linecap: round;
      stroke-linejoin: round;
    }
    @media (prefers-reduced-motion: no-preference) {
      svg {
        transition: transform 150ms ease-out;
      }
    }
    button[aria-expanded="true"] svg {
      transform: rotate(180deg);
    }
    [role="region"] {
      padding: 0 var(--tx-space-1) var(--tx-space-3);
    }
  `;

  private uid = ++uid;

  constructor() {
    super();
    this.items = [];
    this.type = "single";
    this.collapsible = true;
    this.value = [];
  }

  private toggle(item: AccordionItem): void {
    if (item.disabled) return;
    const open = this.value.includes(item.value);
    let next: string[];
    if (this.type === "multiple") next = open ? this.value.filter((v) => v !== item.value) : [...this.value, item.value];
    else if (open) next = this.collapsible ? [] : [item.value];
    else next = [item.value];
    if (next.length === this.value.length && next.every((v, i) => v === this.value[i])) return;
    this.value = next;
    this.dispatchEvent(new CustomEvent("tx-change", { detail: { value: next }, bubbles: true, composed: true }));
  }

  private onKeydown(event: KeyboardEvent): void {
    const triggers = [...this.renderRoot.querySelectorAll<HTMLButtonElement>("button.trigger:not(:disabled)")];
    if (triggers.length === 0) return;
    const at = triggers.findIndex((t) => t === (event.target as Element).closest("button.trigger"));
    let next: number;
    switch (event.key) {
      case "ArrowDown":
        next = (at + 1) % triggers.length;
        break;
      case "ArrowUp":
        next = (at - 1 + triggers.length) % triggers.length;
        break;
      case "Home":
        next = 0;
        break;
      case "End":
        next = triggers.length - 1;
        break;
      default:
        return;
    }
    event.preventDefault();
    triggers[next]!.focus();
  }

  render() {
    const base = `tx-accordion-${this.uid}`;
    return html`<div @keydown=${this.onKeydown}>
      ${this.items.map((item, i) => {
        const open = this.value.includes(item.value);
        return html`<div class="item">
          <h3>
            <button
              type="button"
              class="trigger"
              id="${base}-trigger-${i}"
              aria-expanded=${open ? "true" : "false"}
              aria-controls="${base}-region-${i}"
              ?disabled=${item.disabled}
              @click=${() => this.toggle(item)}
            >
              <span class="label"><slot name="label-${item.value}">${item.label}</slot></span>
              <svg viewBox="0 0 24 24" aria-hidden="true">${CHEVRON}</svg>
            </button>
          </h3>
          <div role="region" id="${base}-region-${i}" aria-labelledby="${base}-trigger-${i}" ?hidden=${!open}>
            <slot name=${item.value}></slot>
          </div>
        </div>`;
      })}
    </div>`;
  }
}

if (!customElements.get("tx-accordion")) customElements.define("tx-accordion", TxAccordion);
