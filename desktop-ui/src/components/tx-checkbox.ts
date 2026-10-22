import { LitElement, css, html, nothing, svg } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { live } from "lit/directives/live.js";
import { colorSizeStyles, emit, srOnly, type ControlColor, type ControlSize, type FieldControl } from "./tx-form-field";

/**
 * Checkbox backed by a visually hidden native input (keyboard, label click and form semantics for free).
 * Fires `tx-input` and then `tx-change` with `detail: { value, checked }` (both the new state) on toggle.
 * The label can be plain text (`label`) or rich markup in the `label` slot; `description` has a slot too.
 */
export class TxCheckbox extends LitElement implements FieldControl {
  static properties = {
    checked: { type: Boolean, reflect: true },
    indeterminate: { type: Boolean, reflect: true },
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
        --_box: 1rem;
      }
      :host([size="xs"]) {
        --_box: 0.75rem;
      }
      :host([size="sm"]) {
        --_box: 0.875rem;
      }
      :host([size="lg"]) {
        --_box: 1.125rem;
      }
      :host([size="xl"]) {
        --_box: 1.25rem;
      }
      .wrap {
        position: relative;
        display: flex;
        align-items: flex-start;
        font-family: var(--tx-font-sans);
        font-size: var(--_fs);
        line-height: 1.25rem;
        color: var(--tx-text);
        cursor: pointer;
      }
      .wrap.disabled {
        opacity: 0.75;
        cursor: not-allowed;
      }
      input {
        position: absolute;
        width: var(--_box);
        height: var(--_box);
        margin: 0;
        opacity: 0;
        cursor: inherit;
      }
      .box {
        flex: none;
        display: grid;
        place-items: center;
        box-sizing: border-box;
        width: var(--_box);
        height: var(--_box);
        margin-top: calc((1.25rem - var(--_box)) / 2);
        border-radius: var(--tx-radius);
        background: var(--tx-bg);
        color: var(--tx-text-inverted);
        box-shadow: inset 0 0 0 1px var(--tx-border-strong);
      }
      .box svg {
        grid-area: 1 / 1;
        width: 100%;
        height: 100%;
        opacity: 0;
      }
      input:checked + .box,
      input:indeterminate + .box {
        background: var(--_accent);
        box-shadow: none;
      }
      input:checked + .box .check,
      input:indeterminate + .box .dash {
        opacity: 1;
      }
      input:indeterminate + .box .check {
        opacity: 0;
      }
      .invalid .box {
        box-shadow: inset 0 0 0 1px var(--tx-error);
      }
      input:focus-visible + .box {
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
        color: var(--tx-text-muted);
      }
      @media (forced-colors: active) {
        input:focus-visible + .box {
          outline-color: Highlight;
        }
      }
    `,
  ];

  checked = false;
  indeterminate = false;
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
    this.indeterminate = false;
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
          name=${ifDefined(this.name)}
          .checked=${live(this.checked)}
          .indeterminate=${this.indeterminate}
          ?disabled=${this.disabled}
          ?required=${this.required}
          aria-label=${ifDefined(name || undefined)}
          aria-invalid=${this.invalid ? "true" : nothing}
          aria-describedby=${this.fieldDescription ? "desc" : nothing}
          @change=${this.onChange}
        />
        <span class="box" aria-hidden="true">
          <svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            ${svg`<path class="check" d="M3.5 8.5 6.5 11.5 12.5 4.5"/><path class="dash" d="M4 8h8"/>`}
          </svg>
        </span>
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

if (!customElements.get("tx-checkbox")) customElements.define("tx-checkbox", TxCheckbox);

declare global {
  interface HTMLElementTagNameMap {
    "tx-checkbox": TxCheckbox;
  }
}
