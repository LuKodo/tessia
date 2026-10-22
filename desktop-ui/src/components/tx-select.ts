import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { boxStyles, emit, type ControlColor, type ControlSize, type ControlVariant, type FieldControl } from "./tx-form-field";

export type SelectValue = string | number;

export interface SelectItem {
  value: SelectValue;
  label: string;
  disabled?: boolean;
}

/**
 * Single choice from `items`, rendered with a native `<select>` (platform picker, keyboard and
 * accessibility included). Fires `tx-input` then `tx-change` with `detail: { value }`, where `value`
 * keeps the type of the chosen item's `value`. For a searchable list use `<tx-select-menu>`.
 */
export class TxSelect extends LitElement implements FieldControl {
  static properties = {
    items: { attribute: false },
    value: { attribute: false },
    placeholder: {},
    name: {},
    disabled: { type: Boolean, reflect: true },
    required: { type: Boolean },
    invalid: { type: Boolean, reflect: true },
    size: { reflect: true },
    variant: { reflect: true },
    color: { reflect: true },
    block: { type: Boolean, reflect: true },
    accessibleName: { attribute: "aria-label" },
    fieldLabel: { attribute: false },
    fieldDescription: { attribute: false },
  };

  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };
  static styles = [
    boxStyles,
    css`
      select.control {
        appearance: none;
        cursor: pointer;
        text-overflow: ellipsis;
      }
      select.control:disabled {
        cursor: not-allowed;
      }
      option {
        background: var(--tx-bg-elevated);
        color: var(--tx-text-highlighted);
      }
      .chevron {
        flex: none;
        width: 1rem;
        height: 1rem;
        color: var(--tx-text-dimmed);
        pointer-events: none;
      }
    `,
  ];

  items: Array<SelectItem | string | number> = [];
  value?: SelectValue;
  placeholder?: string;
  name?: string;
  disabled = false;
  required = false;
  invalid = false;
  size: ControlSize = "md";
  variant: ControlVariant = "outline";
  color: ControlColor = "primary";
  block = false;
  accessibleName?: string;
  fieldLabel?: string;
  fieldDescription?: string;

  private get field() {
    return this.renderRoot.querySelector<HTMLSelectElement>("select");
  }

  private normalized(): SelectItem[] {
    return this.items.map((item) => (typeof item === "object" ? item : { value: item, label: String(item) }));
  }

  focus(options?: FocusOptions) {
    this.field?.focus(options);
  }

  protected updated() {
    const select = this.field;
    const current = this.value == null ? "" : String(this.value);
    if (select && select.value !== current) select.value = current;
  }

  private onChange(e: Event) {
    const raw = (e.target as HTMLSelectElement).value;
    const item = this.normalized().find((i) => String(i.value) === raw);
    this.value = item ? item.value : raw;
    emit(this, "tx-input", { value: this.value });
    emit(this, "tx-change", { value: this.value });
  }

  render() {
    const invalid = this.invalid || this.color === "error";
    const current = this.value == null ? "" : String(this.value);
    return html`
      <div class="box ${invalid ? "invalid" : ""} ${this.disabled ? "disabled" : ""}">
        <select
          class="control"
          name=${ifDefined(this.name)}
          ?disabled=${this.disabled}
          ?required=${this.required}
          aria-label=${ifDefined(this.accessibleName || this.fieldLabel || undefined)}
          aria-invalid=${invalid ? "true" : nothing}
          aria-describedby=${this.fieldDescription ? "desc" : nothing}
          @change=${this.onChange}
        >
          ${this.placeholder ? html`<option value="" disabled hidden .selected=${current === ""}>${this.placeholder}</option>` : nothing}
          ${this.normalized().map(
            (item) => html`<option value=${String(item.value)} ?disabled=${item.disabled} .selected=${String(item.value) === current}>${item.label}</option>`,
          )}
        </select>
        <svg class="chevron" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="m6 9 6 6 6-6" />
        </svg>
        ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
      </div>
    `;
  }
}

if (!customElements.get("tx-select")) customElements.define("tx-select", TxSelect);

declare global {
  interface HTMLElementTagNameMap {
    "tx-select": TxSelect;
  }
}
