import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import "./tx-checkbox";
import { colorSizeStyles, emit, srOnly, type ControlColor, type ControlSize, type FieldControl } from "./tx-form-field";

export interface CheckboxGroupItem {
  value: string;
  label?: string;
  description?: string;
  disabled?: boolean;
}

/**
 * A set of checkboxes bound to one `value: string[]`, like Nuxt UI's UCheckboxGroup: checking appends
 * the item's value, unchecking removes it. Fires one `tx-input` and one `tx-change` per toggle with
 * `detail: { value: string[] }` (a new array); the inner checkboxes' own events are not exposed.
 */
export class TxCheckboxGroup extends LitElement implements FieldControl {
  static properties = {
    items: { attribute: false },
    value: { attribute: false },
    legend: {},
    orientation: { reflect: true },
    disabled: { type: Boolean, reflect: true },
    required: { type: Boolean },
    invalid: { type: Boolean, reflect: true },
    size: { reflect: true },
    color: { reflect: true },
    accessibleName: { attribute: "aria-label" },
    fieldLabel: { attribute: false },
    fieldDescription: { attribute: false },
  };

  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };
  static styles = [
    srOnly,
    colorSizeStyles,
    css`
      :host {
        display: block;
      }
      fieldset {
        display: flex;
        flex-direction: column;
        gap: var(--tx-space-2);
        min-width: 0;
        margin: 0;
        padding: 0;
        border: 0;
      }
      fieldset.horizontal {
        flex-direction: row;
        flex-wrap: wrap;
        gap: var(--tx-space-2) var(--tx-space-4);
      }
      legend {
        padding: 0;
        margin-bottom: var(--tx-space-2);
        font-family: var(--tx-font-sans);
        font-size: var(--tx-text-sm);
        font-weight: 500;
        color: var(--tx-text-highlighted);
      }
    `,
  ];

  items: Array<string | CheckboxGroupItem> = [];
  value: string[] = [];
  legend?: string;
  orientation: "vertical" | "horizontal" = "vertical";
  disabled = false;
  required = false;
  invalid = false;
  size: ControlSize = "md";
  color: ControlColor = "primary";
  accessibleName?: string;
  fieldLabel?: string;
  fieldDescription?: string;

  private normalized(): CheckboxGroupItem[] {
    return this.items.map((item) => (typeof item === "string" ? { value: item } : item));
  }

  focus(options?: FocusOptions) {
    this.renderRoot.querySelector<HTMLElement>("tx-checkbox:not([disabled])")?.focus(options);
  }

  private onToggle(item: CheckboxGroupItem, e: Event) {
    e.stopPropagation();
    if (this.disabled || item.disabled) return;
    const checked = (e as CustomEvent<{ checked: boolean }>).detail.checked;
    const rest = (this.value ?? []).filter((v) => v !== item.value);
    this.value = checked ? [...rest, item.value] : rest;
    emit(this, "tx-input", { value: this.value });
    emit(this, "tx-change", { value: this.value });
  }

  render() {
    const selected = new Set(this.value ?? []);
    return html`
      <fieldset
        class=${this.orientation === "horizontal" ? "horizontal" : ""}
        aria-label=${ifDefined((!this.legend && (this.accessibleName || this.fieldLabel)) || undefined)}
        aria-describedby=${this.fieldDescription ? "desc" : nothing}
      >
        ${this.legend ? html`<legend>${this.legend}</legend>` : nothing}
        ${this.normalized().map(
          (item) => html`<tx-checkbox
            .checked=${selected.has(item.value)}
            .label=${item.label ?? item.value}
            .description=${item.description}
            .disabled=${this.disabled || Boolean(item.disabled)}
            .invalid=${this.invalid}
            .color=${this.color}
            .size=${this.size}
            @tx-input=${(e: Event) => e.stopPropagation()}
            @tx-change=${(e: Event) => this.onToggle(item, e)}
          ></tx-checkbox>`,
        )}
      </fieldset>
      ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
    `;
  }
}

if (!customElements.get("tx-checkbox-group")) customElements.define("tx-checkbox-group", TxCheckboxGroup);

declare global {
  interface HTMLElementTagNameMap {
    "tx-checkbox-group": TxCheckboxGroup;
  }
}
