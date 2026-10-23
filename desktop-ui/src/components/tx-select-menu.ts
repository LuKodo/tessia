import { LitElement, css, html, nothing, type PropertyValues } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { live } from "lit/directives/live.js";
import { boxStyles, emit, type ControlColor, type ControlSize, type ControlVariant, type FieldControl } from "./tx-form-field";

interface MenuOption {
  value: unknown;
  label: string;
  disabled: boolean;
  raw: unknown;
}

/** Lowercase and strip accents so "preparacion" finds "Preparación". */
const fold = (text: string) => text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

/**
 * Searchable single select following the ARIA 1.2 editable combobox pattern: a text input that filters
 * the listbox as you type (Arrows move, Enter selects, Escape closes, Tab leaves). `items` are objects
 * read through `value-key` / `label-key` (plus an optional `disabled` flag) or plain strings. Fires
 * `tx-input` then `tx-change` with `detail: { value, item }` when the selection changes.
 */
export class TxSelectMenu extends LitElement implements FieldControl {
  static properties = {
    items: { attribute: false },
    value: { attribute: false },
    valueKey: { attribute: "value-key" },
    labelKey: { attribute: "label-key" },
    placeholder: {},
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
    open: { state: true },
    query: { state: true },
    dirty: { state: true },
    active: { state: true },
  };

  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };
  static styles = [
    boxStyles,
    css`
      .anchor {
        position: relative;
      }
      input.control {
        cursor: pointer;
      }
      input.control:disabled {
        cursor: not-allowed;
      }
      .chevron {
        flex: none;
        display: grid;
        place-items: center;
        width: 1rem;
        height: 1rem;
        color: var(--tx-text-dimmed);
        cursor: pointer;
      }
      .chevron svg {
        width: 100%;
        height: 100%;
        transition: transform 0.12s ease;
      }
      .open .chevron svg {
        transform: rotate(180deg);
      }
      .menu {
        position: absolute;
        z-index: 30;
        top: calc(100% + 0.25rem);
        left: 0;
        right: 0;
        box-sizing: border-box;
        max-height: 15rem;
        margin: 0;
        padding: 0.25rem;
        overflow-y: auto;
        list-style: none;
        border-radius: calc(var(--tx-radius) * 1.5);
        background: var(--tx-bg-elevated);
        box-shadow:
          0 0 0 1px var(--tx-border),
          0 6px 16px light-dark(rgb(0 0 0 / 0.12), rgb(0 0 0 / 0.5));
      }
      .menu[hidden] {
        display: none;
      }
      .option {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: 0.5rem;
        padding: 0.375rem 0.5rem;
        border-radius: var(--tx-radius);
        font-size: var(--_fs);
        color: var(--tx-text-highlighted);
        cursor: pointer;
      }
      .option[data-active="true"] {
        background: var(--tx-bg-muted);
      }
      .option[aria-selected="true"] {
        font-weight: 600;
        color: var(--_accent);
      }
      .option[aria-disabled="true"] {
        opacity: 0.5;
        cursor: not-allowed;
      }
      .option svg {
        flex: none;
        width: 1rem;
        height: 1rem;
      }
      .empty {
        padding: 0.5rem;
        font-size: var(--_fs);
        color: var(--tx-text-muted);
        text-align: center;
      }
      @media (prefers-reduced-motion: reduce) {
        .chevron svg {
          transition: none;
        }
      }
    `,
  ];

  items: unknown[] = [];
  value?: unknown;
  valueKey = "value";
  labelKey = "label";
  placeholder?: string;
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
  open = false;
  query = "";
  /** True once the user typed since opening: the input then shows the query instead of the selection. */
  dirty = false;
  /** Index of the highlighted option in the visible list, -1 for none. */
  active = -1;

  private onDocumentPress = (e: Event) => {
    if (this.open && !e.composedPath().includes(this)) this.close();
  };

  connectedCallback() {
    super.connectedCallback();
    document.addEventListener("pointerdown", this.onDocumentPress, true);
  }

  disconnectedCallback() {
    document.removeEventListener("pointerdown", this.onDocumentPress, true);
    super.disconnectedCallback();
  }

  focus(options?: FocusOptions) {
    this.renderRoot.querySelector<HTMLInputElement>("input")?.focus(options);
  }

  private options(): MenuOption[] {
    return this.items.map((raw) => {
      if (raw !== null && typeof raw === "object") {
        const item = raw as Record<string, unknown>;
        const value = item[this.valueKey];
        return { value, label: String(item[this.labelKey] ?? value ?? ""), disabled: Boolean(item.disabled), raw };
      }
      return { value: raw, label: String(raw), disabled: false, raw };
    });
  }

  private visible(): MenuOption[] {
    const all = this.options();
    const needle = fold(this.query.trim());
    return this.dirty && needle ? all.filter((o) => fold(o.label).includes(needle)) : all;
  }

  private selectedLabel() {
    return this.options().find((o) => o.value === this.value)?.label ?? "";
  }

  /** Next enabled index from `from` moving by `step`, or `from` itself when none is left that way. */
  private step(list: MenuOption[], from: number, step: 1 | -1): number {
    for (let i = from + step; i >= 0 && i < list.length; i += step) if (!list[i]!.disabled) return i;
    return from;
  }

  private firstEnabled(list: MenuOption[]) {
    return list.findIndex((o) => !o.disabled);
  }

  private openMenu() {
    if (this.disabled || this.open) return;
    const list = this.options();
    const selected = list.findIndex((o) => o.value === this.value && !o.disabled);
    this.query = "";
    this.dirty = false;
    this.active = selected >= 0 ? selected : this.firstEnabled(list);
    this.open = true;
  }

  private close() {
    this.open = false;
    this.dirty = false;
    this.query = "";
    this.active = -1;
  }

  private choose(option: MenuOption) {
    if (option.disabled) return;
    const changed = option.value !== this.value;
    this.value = option.value;
    this.close();
    if (changed) {
      emit(this, "tx-input", { value: option.value, item: option.raw });
      emit(this, "tx-change", { value: option.value, item: option.raw });
    }
  }

  private onInput(e: Event) {
    e.stopPropagation(); // the raw input event would read as an edit of the value to page listeners
    if (this.disabled) return;
    this.open = true;
    this.dirty = true;
    this.query = (e.target as HTMLInputElement).value;
    this.active = this.firstEnabled(this.visible());
  }

  private onKeydown(e: KeyboardEvent) {
    const list = this.visible();
    switch (e.key) {
      case "ArrowDown":
      case "ArrowUp":
        e.preventDefault();
        if (!this.open) this.openMenu();
        else this.active = this.active < 0 ? this.firstEnabled(list) : this.step(list, this.active, e.key === "ArrowDown" ? 1 : -1);
        break;
      case "Enter": {
        const option = this.open ? list[this.active] : undefined;
        if (option) {
          e.preventDefault();
          this.choose(option);
        }
        break;
      }
      case "Escape":
        if (this.open) {
          e.preventDefault();
          e.stopPropagation();
          this.close();
        }
        break;
      case "Tab":
        this.close();
        break;
    }
  }

  /** A press on the box padding behaves like a press on the input. */
  private onBoxClick(e: Event) {
    if (e.target !== e.currentTarget || this.disabled) return;
    this.focus();
    this.openMenu();
  }

  private toggle() {
    if (this.disabled) return;
    if (this.open) this.close();
    else {
      this.focus();
      this.openMenu();
    }
  }

  protected updated(changed: PropertyValues<this>) {
    if (this.open && changed.has("active")) {
      this.renderRoot.querySelector<HTMLElement>('[data-active="true"]')?.scrollIntoView?.({ block: "nearest" });
    }
  }

  render() {
    const invalid = this.invalid || this.color === "error";
    const list = this.open ? this.visible() : [];
    const shown = this.open && this.dirty ? this.query : this.selectedLabel();
    return html`
      <div class="anchor ${this.open ? "open" : ""}">
        <div class="box ${invalid ? "invalid" : ""} ${this.disabled ? "disabled" : ""}" @click=${this.onBoxClick}>
          <input
            class="control"
            type="text"
            role="combobox"
            autocomplete="off"
            spellcheck="false"
            .value=${live(shown)}
            placeholder=${ifDefined(this.placeholder)}
            ?disabled=${this.disabled}
            ?required=${this.required}
            aria-expanded=${this.open ? "true" : "false"}
            aria-haspopup="listbox"
            aria-autocomplete="list"
            aria-controls="listbox"
            aria-activedescendant=${this.open && this.active >= 0 ? `opt-${this.active}` : nothing}
            aria-label=${ifDefined(this.accessibleName || this.fieldLabel || undefined)}
            aria-invalid=${invalid ? "true" : nothing}
            aria-describedby=${this.fieldDescription ? "desc" : nothing}
            @click=${this.openMenu}
            @input=${this.onInput}
            @keydown=${this.onKeydown}
            @blur=${this.close}
          />
          <span class="chevron" aria-hidden="true" @mousedown=${(e: Event) => e.preventDefault()} @click=${this.toggle}>
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="m6 9 6 6 6-6" />
            </svg>
          </span>
        </div>
        <ul id="listbox" class="menu" role="listbox" ?hidden=${!this.open} @mousedown=${(e: Event) => e.preventDefault()}>
          ${this.open && list.length === 0 ? html`<li class="empty" role="presentation">Sin resultados</li>` : nothing}
          ${list.map((option, i) => {
            const selected = option.value === this.value;
            return html`<li
              id="opt-${i}"
              class="option"
              role="option"
              aria-selected=${selected ? "true" : "false"}
              aria-disabled=${option.disabled ? "true" : nothing}
              data-active=${i === this.active ? "true" : nothing}
              @mousedown=${(e: Event) => e.preventDefault()}
              @mousemove=${() => !option.disabled && (this.active = i)}
              @click=${() => this.choose(option)}
            >
              <span>${option.label}</span>
              ${selected
                ? html`<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="m5 12.5 4.5 4.5L19 7.5" /></svg>`
                : nothing}
            </li>`;
          })}
        </ul>
        ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
      </div>
    `;
  }
}

if (!customElements.get("tx-select-menu")) customElements.define("tx-select-menu", TxSelectMenu);

declare global {
  interface HTMLElementTagNameMap {
    "tx-select-menu": TxSelectMenu;
  }
}
