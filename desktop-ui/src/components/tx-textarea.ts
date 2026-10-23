import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { live } from "lit/directives/live.js";
import { boxStyles, emit, type ControlColor, type ControlSize, type ControlVariant, type FieldControl } from "./tx-form-field";

/**
 * Multi-line text control. With `autoresize` it grows with its content up to `maxrows`. Same events as
 * `<tx-input>` (`tx-input` per edit, `tx-change` on commit, `detail: { value }`); native `blur` and
 * `keydown` reach the host so pages can commit on blur or ctrl/cmd+enter.
 */
export class TxTextarea extends LitElement implements FieldControl {
  static properties = {
    value: {},
    placeholder: {},
    name: {},
    rows: { type: Number },
    maxrows: { type: Number },
    autoresize: { type: Boolean, reflect: true },
    maxlength: { type: Number },
    disabled: { type: Boolean, reflect: true },
    readonly: { type: Boolean },
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
      .box {
        align-items: flex-start;
        padding-top: 0.375rem;
        padding-bottom: 0.375rem;
      }
      textarea.control {
        display: block;
        resize: vertical;
        line-height: 1.25rem;
      }
      :host([autoresize]) textarea.control {
        resize: none;
        overflow-y: auto;
      }
    `,
  ];

  value = "";
  placeholder?: string;
  name?: string;
  rows = 3;
  /** Upper bound of the autoresize growth, in rows; unbounded when unset. */
  maxrows?: number;
  autoresize = false;
  maxlength?: number;
  disabled = false;
  readonly = false;
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
    return this.renderRoot.querySelector<HTMLTextAreaElement>("textarea");
  }

  focus(options?: FocusOptions) {
    this.field?.focus(options);
  }

  select() {
    this.field?.select();
  }

  /** Sizes the textarea to its content, capped at `maxrows` lines. */
  private resize() {
    const area = this.field;
    if (!area || !this.autoresize) return;
    const cs = getComputedStyle(area);
    const num = (v: string) => parseFloat(v) || 0;
    const chrome = num(cs.paddingTop) + num(cs.paddingBottom) + num(cs.borderTopWidth) + num(cs.borderBottomWidth);
    area.style.height = "auto";
    let height = area.scrollHeight + num(cs.borderTopWidth) + num(cs.borderBottomWidth);
    if (this.maxrows) height = Math.min(height, (num(cs.lineHeight) || 20) * this.maxrows + chrome);
    area.style.height = `${height}px`;
  }

  protected updated() {
    this.resize();
  }

  private onInput(e: Event) {
    this.value = (e.target as HTMLTextAreaElement).value;
    this.resize();
    emit(this, "tx-input", { value: this.value });
  }

  private onChange() {
    emit(this, "tx-change", { value: this.value });
  }

  render() {
    const invalid = this.invalid || this.color === "error";
    return html`
      <div class="box ${invalid ? "invalid" : ""} ${this.disabled ? "disabled" : ""}">
        <textarea
          class="control"
          name=${ifDefined(this.name)}
          rows=${this.rows}
          .value=${live(this.value ?? "")}
          placeholder=${ifDefined(this.placeholder)}
          maxlength=${ifDefined(this.maxlength)}
          ?disabled=${this.disabled}
          ?readonly=${this.readonly}
          ?required=${this.required}
          aria-label=${ifDefined(this.accessibleName || this.fieldLabel || undefined)}
          aria-invalid=${invalid ? "true" : nothing}
          aria-describedby=${this.fieldDescription ? "desc" : nothing}
          @input=${this.onInput}
          @change=${this.onChange}
        ></textarea>
        ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
      </div>
    `;
  }
}

if (!customElements.get("tx-textarea")) customElements.define("tx-textarea", TxTextarea);

declare global {
  interface HTMLElementTagNameMap {
    "tx-textarea": TxTextarea;
  }
}
