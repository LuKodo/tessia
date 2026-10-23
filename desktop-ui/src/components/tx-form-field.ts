import { LitElement, css, html, nothing } from "lit";

export type ControlColor = "primary" | "neutral" | "success" | "warning" | "error" | "info";
export type ControlVariant = "outline" | "soft" | "subtle" | "ghost";
export type ControlSize = "xs" | "sm" | "md" | "lg" | "xl";

/** What `<tx-form-field>` pushes into a slotted control so label, help and error reach assistive tech. */
export interface FieldControl {
  fieldLabel?: string;
  fieldDescription?: string;
  invalid?: boolean;
  required?: boolean;
}

/** Dispatches a bubbling, composed CustomEvent so pages can listen on the host or any ancestor. */
export function emit<T>(host: HTMLElement, name: string, detail: T): boolean {
  return host.dispatchEvent(new CustomEvent<T>(name, { detail, bubbles: true, composed: true }));
}

/** Visually hidden text that stays in the accessibility tree (descriptions referenced by aria-describedby). */
export const srOnly = css`
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip: rect(0 0 0 0);
    white-space: nowrap;
    border: 0;
  }
`;

/** `color` and `size` resolved to component-local custom properties, shared by every form control. */
export const colorSizeStyles = css`
  :host {
    --_accent: var(--tx-primary);
    --_h: 2rem;
    --_fs: var(--tx-text-sm);
    --_px: 0.625rem;
  }
  :host([hidden]) {
    display: none;
  }
  :host([color="neutral"]) {
    --_accent: var(--tx-bg-inverted);
  }
  :host([color="success"]) {
    --_accent: var(--tx-success);
  }
  :host([color="warning"]) {
    --_accent: var(--tx-warning);
  }
  :host([color="error"]) {
    --_accent: var(--tx-error);
  }
  :host([color="info"]) {
    --_accent: var(--tx-info);
  }
  :host([size="xs"]) {
    --_h: 1.5rem;
    --_fs: 0.75rem;
    --_px: 0.5rem;
  }
  :host([size="sm"]) {
    --_h: 1.75rem;
    --_fs: 0.75rem;
    --_px: 0.625rem;
  }
  :host([size="lg"]) {
    --_h: 2.25rem;
    --_px: 0.75rem;
  }
  :host([size="xl"]) {
    --_h: 2.5rem;
    --_fs: var(--tx-text-base);
    --_px: 0.75rem;
  }
`;

/** The framed box of text-like controls (input, textarea, select, select menu). */
export const boxStyles = [
  srOnly,
  colorSizeStyles,
  css`
    :host {
      display: inline-block;
      vertical-align: top;
      min-width: 0;
      --_bg: var(--tx-bg);
      --_ring: var(--tx-border-strong);
      --_ring-w: 1px;
    }
    :host([block]) {
      display: block;
    }
    :host([variant="soft"]) {
      --_bg: var(--tx-bg-muted);
      --_ring-w: 0px;
    }
    :host([variant="subtle"]) {
      --_bg: var(--tx-bg-muted);
    }
    :host([variant="ghost"]) {
      --_bg: transparent;
      --_ring-w: 0px;
    }
    .box {
      position: relative;
      display: flex;
      align-items: center;
      gap: 0.5rem;
      box-sizing: border-box;
      width: 100%;
      min-height: var(--_h);
      padding: 0 var(--_px);
      border-radius: calc(var(--tx-radius) * 1.5);
      background: var(--_bg);
      color: var(--tx-text-highlighted);
      font-family: var(--tx-font-sans);
      font-size: var(--_fs);
      line-height: 1.25rem;
      box-shadow: inset 0 0 0 var(--_ring-w) var(--_ring);
      transition: box-shadow 0.12s ease;
    }
    :host([variant="ghost"]) .box:hover:not(.disabled) {
      background: var(--tx-bg-muted);
    }
    .box:focus-within {
      --_ring: var(--_accent);
      --_ring-w: 2px;
    }
    .box.invalid {
      --_accent: var(--tx-error);
      --_ring: var(--tx-error);
      --_ring-w: 1px;
    }
    .box.invalid:focus-within {
      --_ring-w: 2px;
    }
    .box.disabled {
      opacity: 0.75;
      cursor: not-allowed;
    }
    .control {
      flex: 1;
      min-width: 0;
      width: 100%;
      margin: 0;
      padding: 0;
      border: 0;
      border-radius: 0;
      background: transparent;
      color: inherit;
      font: inherit;
      outline: none;
      text-overflow: ellipsis;
    }
    .control::placeholder {
      color: var(--tx-text-dimmed);
    }
    .control:disabled {
      cursor: not-allowed;
    }
    ::slotted([slot="leading"]),
    ::slotted([slot="trailing"]) {
      flex: none;
      color: var(--tx-text-dimmed);
    }
    @media (forced-colors: active) {
      .box:focus-within {
        outline: 2px solid Highlight;
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .box {
        transition: none;
      }
    }
  `,
];

/** True for custom elements of this design system (the controls that understand `FieldControl`). */
const isTxControl = (el: Element) => el.localName.startsWith("tx-");
const isNativeControl = (el: Element) => ["input", "select", "textarea"].includes(el.localName);

/**
 * Label, description, hint, help and error around one control. Shadow DOM stops `for`/`aria-describedby`
 * ids from crossing to the slotted control, so the field pushes `fieldLabel`, `fieldDescription`,
 * `invalid` and `required` into `tx-*` controls (they render the ARIA wiring inside their own root) and
 * sets the equivalent attributes on native controls.
 */
export class TxFormField extends LitElement {
  static properties = {
    label: {},
    description: {},
    hint: {},
    help: {},
    error: {},
    required: { type: Boolean, reflect: true },
  };

  static styles = css`
    :host {
      display: block;
      min-width: 0;
      font-family: var(--tx-font-sans);
    }
    :host([hidden]) {
      display: none;
    }
    .top {
      display: flex;
      align-items: baseline;
      justify-content: space-between;
      gap: var(--tx-space-2);
    }
    label {
      font-size: var(--tx-text-sm);
      font-weight: 500;
      color: var(--tx-text-highlighted);
      cursor: default;
    }
    .required {
      margin-left: 0.125rem;
      color: var(--tx-error);
    }
    .hint,
    .description,
    .help {
      font-size: 0.75rem;
      color: var(--tx-text-muted);
    }
    .description {
      margin: 0;
    }
    .control {
      margin-top: var(--tx-space-1);
    }
    .help,
    .error {
      margin: var(--tx-space-1) 0 0;
      font-size: 0.75rem;
    }
    .error {
      color: var(--tx-error);
    }
  `;

  label?: string;
  description?: string;
  hint?: string;
  help?: string;
  /** Error message; any truthy value marks the control invalid, and the message replaces the help text. */
  error?: string | boolean;
  required = false;

  /** Controls whose `invalid`/`required` we set, so we only undo what we did. */
  private invalidSet = new WeakSet<Element>();
  private requiredSet = new WeakSet<Element>();

  private get errorText() {
    return typeof this.error === "string" ? this.error : "";
  }

  private controls(): HTMLElement[] {
    const slot = this.renderRoot?.querySelector<HTMLSlotElement>("slot:not([name])");
    return slot ? (slot.assignedElements({ flatten: true }) as HTMLElement[]) : [];
  }

  private wire() {
    const invalid = Boolean(this.error);
    const message = [this.errorText || this.help, this.description].filter(Boolean).join(". ");
    for (const el of this.controls()) {
      if (isTxControl(el)) {
        const control = el as HTMLElement & FieldControl;
        control.fieldLabel = this.label || undefined;
        control.fieldDescription = message;
        if (invalid) {
          control.invalid = true;
          this.invalidSet.add(el);
        } else if (this.invalidSet.has(el)) {
          control.invalid = false;
          this.invalidSet.delete(el);
        }
        if (this.required) {
          control.required = true;
          this.requiredSet.add(el);
        } else if (this.requiredSet.has(el)) {
          control.required = false;
          this.requiredSet.delete(el);
        }
      } else if (isNativeControl(el)) {
        if (this.label && !el.hasAttribute("aria-label")) el.setAttribute("aria-label", this.label);
        if (invalid) {
          el.setAttribute("aria-invalid", "true");
          this.invalidSet.add(el);
        } else if (this.invalidSet.has(el)) {
          el.removeAttribute("aria-invalid");
          this.invalidSet.delete(el);
        }
        if (this.required) {
          el.setAttribute("aria-required", "true");
          this.requiredSet.add(el);
        } else if (this.requiredSet.has(el)) {
          el.removeAttribute("aria-required");
          this.requiredSet.delete(el);
        }
      }
    }
  }

  protected updated() {
    this.wire();
  }

  private focusControl() {
    const first = this.controls()[0];
    first?.focus();
  }

  render() {
    const showTop = this.label || this.hint;
    return html`
      ${showTop
        ? html`<div class="top">
            <label @click=${this.focusControl}
              ><slot name="label">${this.label}</slot>${this.required ? html`<span class="required" aria-hidden="true">*</span>` : nothing}</label
            >
            <span class="hint"><slot name="hint">${this.hint}</slot></span>
          </div>`
        : nothing}
      ${this.description ? html`<p class="description"><slot name="description">${this.description}</slot></p>` : nothing}
      <div class="control"><slot @slotchange=${() => this.wire()}></slot></div>
      ${this.errorText
        ? html`<p class="error" role="alert">${this.errorText}</p>`
        : this.help
          ? html`<p class="help"><slot name="help">${this.help}</slot></p>`
          : nothing}
    `;
  }
}

if (!customElements.get("tx-form-field")) customElements.define("tx-form-field", TxFormField);

declare global {
  interface HTMLElementTagNameMap {
    "tx-form-field": TxFormField;
  }
}
