import { LitElement, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { live } from "lit/directives/live.js";
import { boxStyles, emit, type ControlColor, type ControlSize, type ControlVariant, type FieldControl } from "./tx-form-field";

/**
 * Single-line text control (text, number, search, password, date, ...). `value` is always a string;
 * pages convert numbers themselves. Fires `tx-input` on every edit and `tx-change` on commit, both with
 * `detail: { value }`. Native `focus`, `blur`, `keydown` and `input` events still reach the host.
 */
export class TxInput extends LitElement implements FieldControl {
  static properties = {
    type: { reflect: true },
    value: {},
    placeholder: {},
    name: {},
    disabled: { type: Boolean, reflect: true },
    readonly: { type: Boolean },
    required: { type: Boolean },
    invalid: { type: Boolean, reflect: true },
    min: {},
    max: {},
    step: {},
    maxlength: { type: Number },
    inputmode: {},
    autocomplete: {},
    size: { reflect: true },
    variant: { reflect: true },
    color: { reflect: true },
    block: { type: Boolean, reflect: true },
    accessibleName: { attribute: "aria-label" },
    fieldLabel: { attribute: false },
    fieldDescription: { attribute: false },
  };

  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };
  static styles = boxStyles;

  type = "text";
  value = "";
  placeholder?: string;
  name?: string;
  disabled = false;
  readonly = false;
  required = false;
  invalid = false;
  min?: string | number;
  max?: string | number;
  step?: string | number;
  maxlength?: number;
  inputmode?: string;
  autocomplete?: string;
  size: ControlSize = "md";
  variant: ControlVariant = "outline";
  color: ControlColor = "primary";
  block = false;
  accessibleName?: string;
  fieldLabel?: string;
  fieldDescription?: string;

  private get field() {
    return this.renderRoot.querySelector<HTMLInputElement>("input");
  }

  focus(options?: FocusOptions) {
    this.field?.focus(options);
  }

  select() {
    this.field?.select();
  }

  private onInput(e: Event) {
    this.value = (e.target as HTMLInputElement).value;
    emit(this, "tx-input", { value: this.value });
  }

  private onChange() {
    emit(this, "tx-change", { value: this.value });
  }

  render() {
    const invalid = this.invalid || this.color === "error";
    return html`
      <div class="box ${invalid ? "invalid" : ""} ${this.disabled ? "disabled" : ""}">
        <slot name="leading"></slot>
        <input
          class="control"
          type=${this.type}
          name=${ifDefined(this.name)}
          .value=${live(this.value ?? "")}
          placeholder=${ifDefined(this.placeholder)}
          min=${ifDefined(this.min)}
          max=${ifDefined(this.max)}
          step=${ifDefined(this.step)}
          maxlength=${ifDefined(this.maxlength)}
          inputmode=${ifDefined(this.inputmode)}
          autocomplete=${ifDefined(this.autocomplete)}
          ?disabled=${this.disabled}
          ?readonly=${this.readonly}
          ?required=${this.required}
          aria-label=${ifDefined(this.accessibleName || this.fieldLabel || undefined)}
          aria-invalid=${invalid ? "true" : nothing}
          aria-describedby=${this.fieldDescription ? "desc" : nothing}
          @input=${this.onInput}
          @change=${this.onChange}
        />
        <slot name="trailing"></slot>
        ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
      </div>
    `;
  }
}

if (!customElements.get("tx-input")) customElements.define("tx-input", TxInput);

declare global {
  interface HTMLElementTagNameMap {
    "tx-input": TxInput;
  }
}
