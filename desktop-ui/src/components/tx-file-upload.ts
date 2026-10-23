import { LitElement, css, html, nothing } from "lit";
import { ifDefined } from "lit/directives/if-defined.js";
import { emit, srOnly, type ControlColor, type ControlSize, type FieldControl } from "./tx-form-field";

/** True when `file` satisfies an `accept` string (".csv", "image/*", "text/csv", comma separated). */
function matchesAccept(file: File, accept?: string): boolean {
  const tokens = (accept ?? "")
    .split(",")
    .map((t) => t.trim().toLowerCase())
    .filter(Boolean);
  if (tokens.length === 0) return true;
  const name = file.name.toLowerCase();
  const type = file.type.toLowerCase();
  return tokens.some((t) => (t.startsWith(".") ? name.endsWith(t) : t.endsWith("/*") ? type.startsWith(t.slice(0, -1)) : type === t));
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

/**
 * File picker with a drop zone (`variant="area"`, default) or a compact button (`variant="button"`).
 * The selected files are listed under it and can be removed one by one. Fires `tx-change` with
 * `detail: { files: File[] }` whenever the selection changes; a new pick or drop replaces the selection
 * (only the first file unless `multiple`), and a page resets it by assigning `files = []`.
 */
export class TxFileUpload extends LitElement implements FieldControl {
  static properties = {
    files: { attribute: false },
    accept: {},
    multiple: { type: Boolean, reflect: true },
    variant: { reflect: true },
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
    dragging: { state: true },
  };

  static shadowRootOptions = { ...LitElement.shadowRootOptions, delegatesFocus: true };
  static styles = [
    srOnly,
    css`
      :host {
        display: block;
        font-family: var(--tx-font-sans);
        font-size: var(--tx-text-sm);
        --_accent: var(--tx-primary);
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
      .zone {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: var(--tx-space-2);
        box-sizing: border-box;
        width: 100%;
        margin: 0;
        padding: var(--tx-space-4);
        border: 1px dashed var(--tx-border-strong);
        border-radius: calc(var(--tx-radius) * 1.5);
        background: var(--tx-bg);
        color: var(--tx-text-muted);
        font: inherit;
        text-align: center;
        cursor: pointer;
        transition:
          background 0.12s ease,
          border-color 0.12s ease;
      }
      .zone.area {
        flex-direction: column;
        min-height: 7rem;
      }
      .zone.button {
        display: inline-flex;
        width: auto;
        min-height: 2rem;
        padding: 0 0.75rem;
        border-style: solid;
        color: var(--tx-text-highlighted);
        gap: 0.5rem;
      }
      .zone:hover:not(:disabled),
      .zone.dragging {
        background: var(--tx-bg-muted);
        border-color: var(--_accent);
      }
      .zone.invalid {
        border-color: var(--tx-error);
      }
      .zone:disabled {
        opacity: 0.75;
        cursor: not-allowed;
      }
      .zone:focus-visible {
        outline: var(--tx-focus-ring);
        outline-color: var(--_accent);
        outline-offset: 2px;
      }
      .zone svg {
        flex: none;
        width: 1.5rem;
        height: 1.5rem;
        color: var(--tx-text-dimmed);
      }
      .zone.button svg {
        width: 1rem;
        height: 1rem;
      }
      .text {
        display: flex;
        flex-direction: column;
        min-width: 0;
      }
      .zone.button .text {
        flex-direction: row;
        align-items: baseline;
        gap: 0.5rem;
      }
      .label {
        font-weight: 500;
        color: var(--tx-text-highlighted);
      }
      .description {
        font-size: 0.75rem;
        color: var(--tx-text-muted);
      }
      .files {
        display: flex;
        flex-direction: column;
        gap: var(--tx-space-1);
        margin: var(--tx-space-2) 0 0;
        padding: 0;
        list-style: none;
      }
      .files[hidden] {
        display: none;
      }
      .files li {
        display: flex;
        align-items: center;
        gap: var(--tx-space-2);
        padding: 0.25rem 0.5rem;
        border-radius: var(--tx-radius);
        background: var(--tx-bg-muted);
        color: var(--tx-text-highlighted);
      }
      .name {
        flex: 1;
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .size {
        flex: none;
        font-size: 0.75rem;
        color: var(--tx-text-muted);
      }
      .files button {
        display: grid;
        place-items: center;
        flex: none;
        width: 1.5rem;
        height: 1.5rem;
        padding: 0;
        border: 0;
        border-radius: var(--tx-radius);
        background: transparent;
        color: var(--tx-text-muted);
        cursor: pointer;
      }
      .files button:hover {
        background: var(--tx-bg-elevated);
        color: var(--tx-text-highlighted);
      }
      .files button:focus-visible {
        outline: var(--tx-focus-ring);
        outline-color: var(--_accent);
      }
      .files button svg {
        width: 0.875rem;
        height: 0.875rem;
      }
      @media (prefers-reduced-motion: reduce) {
        .zone {
          transition: none;
        }
      }
    `,
  ];

  files: File[] = [];
  accept?: string;
  multiple = false;
  variant: "area" | "button" = "area";
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
  dragging = false;

  private get picker() {
    return this.renderRoot.querySelector<HTMLInputElement>('input[type="file"]');
  }

  focus(options?: FocusOptions) {
    this.renderRoot.querySelector<HTMLElement>(".zone")?.focus(options);
  }

  /** Replaces the selection and tells the page. */
  private setFiles(next: File[]) {
    this.files = this.multiple ? next : next.slice(0, 1);
    emit(this, "tx-change", { files: [...this.files] });
  }

  protected updated() {
    // The native input keeps its own file list: empty it when the page cleared the selection.
    const picker = this.picker;
    if (picker && this.files.length === 0 && picker.value) picker.value = "";
  }

  private onPick(e: Event) {
    const input = e.target as HTMLInputElement;
    const picked = Array.from(input.files ?? []);
    if (picked.length === 0) return;
    input.value = ""; // lets the same file be picked again after removing it
    this.setFiles(picked);
  }

  private onDragOver(e: DragEvent) {
    if (this.disabled) return;
    e.preventDefault();
    if (e.dataTransfer) e.dataTransfer.dropEffect = "copy";
    this.dragging = true;
  }

  private onDragLeave() {
    this.dragging = false;
  }

  private onDrop(e: DragEvent) {
    e.preventDefault();
    this.dragging = false;
    if (this.disabled) return;
    const dropped = Array.from(e.dataTransfer?.files ?? []).filter((f) => matchesAccept(f, this.accept));
    if (dropped.length > 0) this.setFiles(dropped);
  }

  private removeFile(index: number) {
    this.setFiles(this.files.filter((_, i) => i !== index));
  }

  render() {
    const isButton = this.variant === "button";
    const fallback = isButton ? (this.multiple ? "Elegir archivos" : "Elegir archivo") : this.multiple ? "Arrastra archivos o haz clic" : "Arrastra un archivo o haz clic";
    return html`
      <div role="group" aria-label=${ifDefined(this.accessibleName || this.fieldLabel || undefined)}>
        <input
          type="file"
          hidden
          tabindex="-1"
          name=${ifDefined(this.name)}
          accept=${ifDefined(this.accept)}
          ?multiple=${this.multiple}
          ?disabled=${this.disabled}
          ?required=${this.required}
          @change=${this.onPick}
        />
        <button
          type="button"
          class="zone ${isButton ? "button" : "area"} ${this.dragging ? "dragging" : ""} ${this.invalid ? "invalid" : ""}"
          ?disabled=${this.disabled}
          aria-describedby=${this.fieldDescription ? "desc" : nothing}
          @click=${() => this.picker?.click()}
          @dragenter=${this.onDragOver}
          @dragover=${this.onDragOver}
          @dragleave=${this.onDragLeave}
          @drop=${this.onDrop}
        >
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M12 16V4" />
            <path d="m7 9 5-5 5 5" />
            <path d="M4 16v3a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-3" />
          </svg>
          <span class="text">
            <span class="label"><slot name="label">${this.label ?? fallback}</slot></span>
            <span class="description"><slot name="description">${this.description}</slot></span>
          </span>
        </button>
        ${this.fieldDescription ? html`<span id="desc" class="sr-only">${this.fieldDescription}</span>` : nothing}
        <ul class="files" aria-live="polite" ?hidden=${this.files.length === 0}>
          ${this.files.map(
            (file, i) => html`<li>
              <span class="name">${file.name}</span>
              <span class="size">${formatSize(file.size)}</span>
              <button type="button" aria-label="Quitar ${file.name}" ?disabled=${this.disabled} @click=${() => this.removeFile(i)}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" aria-hidden="true"><path d="M6 6l12 12M18 6 6 18" /></svg>
              </button>
            </li>`,
          )}
        </ul>
      </div>
    `;
  }
}

if (!customElements.get("tx-file-upload")) customElements.define("tx-file-upload", TxFileUpload);

declare global {
  interface HTMLElementTagNameMap {
    "tx-file-upload": TxFileUpload;
  }
}
