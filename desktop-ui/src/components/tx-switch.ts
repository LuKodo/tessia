import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { live } from "lit/directives/live.js";
import { colorSizeStyles, emit, srOnly, type ControlColor, type ControlSize, type FieldControl } from "./tx-form-field";

/**
 * On/off toggle backed by a visually hidden native checkbox with `role="switch"`.
 * Fires `tx-input` and then `tx-change` with `detail: { value, checked }` (both the new state).
 */
export class TxSwitch extends LitElement implements FieldControl {
  static properties = {
    checked: { type: Boolean, reflect: true },
    label: {},
    description: {},
    name: {},
    disabled: { type: Boolean, reflect: true },
    required: { type: Boolean },
    invalid: { type: Boolean, reflect: true },
    size: { reflect: true },
    color: { reflect: true },
    accessibleName: { attribute: "aria-label" },
    fieldLabel: { attribute: false },
    fieldDescription: { attribute: false },
    hasLabelSlot: { state: true },
    hasDescriptionSlot: { state: true },
  };

  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };
  static styles = [
    srOnly,
    colorSizeStyles,
    css`
      :host {
        display: inline-block;
        vertical-align: top;
        --_w: 2.25rem;
        --_t: 1.25rem;
      }
      :host([size="xs"]) {
        --_w: 1.75rem;
        --_t: 1rem;
      }
      :host([size="sm"]) {
        --_w: 2rem;
        --_t: 1.125rem;
      }
      :host([size="lg"]) {
        --_w: 2.5rem;
        --_t: 1.375rem;
      }
      :host([size="xl"]) {
        --_w: 2.75rem;
        --_t: 1.5rem;
      }
      .wrap {
        position: relative;
        display: flex;
        align-items: flex-start;
        font-family: var(--tx-font-sans);
        font-size: var(--_fs);
        line-height: var(--_t);
        color: var(--tx-text);
        cursor: pointer;
      }
      .wrap.disabled {
        opacity: 0.75;
        cursor: not-allowed;
      }
      input {
        position: absolute;
        width: var(--_w);
        height: var(--_t);
        margin: 0;
        opacity: 0;
        cursor: inherit;
      }
      .track {
        flex: none;
        box-sizing: border-box;
        width: var(--_w);
        height: var(--_t);
        padding: 0.125rem;
        border-radius: 999px;
        background: var(--tx-border-strong);
        transition: background 0.15s ease;
      }
      .thumb {
        display: block;
        width: calc(var(--_t) - 0.25rem);
        height: calc(var(--_t) - 0.25rem);
        border-radius: 50%;
        background: var(--tx-bg-elevated);
        box-shadow: var(--tx-shadow);
        transition: transform 0.15s ease;
      }
      input:checked + .track {
        background: var(--_accent);
      }
      input:checked + .track .thumb {
        transform: translateX(calc(var(--_w) - var(--_t)));
      }
      .invalid .track {
        box-shadow: 0 0 0 1px var(--tx-error);
      }
      input:focus-visible + .track {
        outline: var(--tx-focus-ring);
        outline-color: var(--_accent);
        outline-offset: 2px;
      }
      .text {
        display: flex;
        flex-direction: column;
        min-width: 0;
        margin-left: 0.5rem;
      }
      .text[hidden] {
        display: none;
      }
      .label {
        font-weight: 500;
        color: var(--tx-text-highlighted);
      }
      .description {
        line-height: 1.25rem;
        color: var(--tx-text-muted);
      }
      @media (forced-colors: active) {
        input:focus-visible + .track {
          outline-color: Highlight;
        }
      }
      @media (prefers-reduced-motion: reduce) {
        .track,
        .thumb {
          transition: none;
        }
      }
    `,
  ];

  checked = false;
  label?: string;
  description?: string;
  name?: string;
  disabled = false;
  required = false;
  invalid = false;
  size: ControlSize = "md";
  color: ControlColor = "primary";
  accessibleName?: string;
  fieldLabel?: string;
  fieldDescription?: string;
  hasLabelSlot = false;
  hasDescriptionSlot = false;

  private get field() {
    return this.renderRoot.querySelector<HTMLInputElement>("input");
  }

  focus(options?: FocusOptions) {
    this.field?.focus(options);
  }

  private onChange(e: Event) {
    this.checked = (e.target as HTMLInputElement).checked;
    const detail = { value: this.checked, checked: this.checked };
    emit(this, "tx-input", detail);
    emit(this, "tx-change", detail);
  }

  private hasNodes = (e: Event) => (e.target as HTMLSlotElement).assignedNodes().length > 0;

  render() {
    const showText = Boolean(this.label || this.description || this.hasLabelSlot || this.hasDescriptionSlot);
    const name = this.accessibleName || (this.label || this.hasLabelSlot ? undefined : this.fieldLabel);
    return html`
      <label class="wrap ${this.invalid ? "invalid" : ""} ${this.disabled ? "disabled" : ""}">
        <input
          type="checkbox"
          role="switch"
          name=${ifDefined(this.name)}
          .checked=${live(this.checked)}
          ?disabled=${this.disabled}
          ?required=${this.required}
          aria-label=${ifDefined(name || undefined)}
          aria-invalid=${this.invalid ? "true" : nothing}
          aria-describedby=${this.fieldDescription ? "desc" : nothing}
          @change=${this.onChange}
        />
        <span class="track" aria-hidden="true"><span class="thumb"></span></span>
        <span class="text" ?hidden=${!showText}>
          <span class="label"><slot name="label" @slotchange=${(e: Event) => (this.hasLabelSlot = this.hasNodes(e))}>${this.label}</slot></span>
          <span class="description"
            ><slot name="description" @slotchange=${(e: Event) => (this.hasDescriptionSlot = this.hasNodes(e))}>${this.description}</slot></span
          >
        </span>
      </label>
      ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
    `;
  }
}

if (!customElements.get("tx-switch")) customElements.define("tx-switch", TxSwitch);

declare global {
  interface HTMLElementTagNameMap {
    "tx-switch": TxSwitch;
  }
}
