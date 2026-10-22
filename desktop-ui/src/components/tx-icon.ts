import { LitElement, css, html, nothing } from "lit";
import { unsafeSVG } from "lit/directives/unsafe-svg.js";
import { ICONS } from "./icons/icons";

const SIZES: Record<string, string> = { xs: "0.75rem", sm: "1rem", md: "1.25rem", lg: "1.5rem", xl: "1.75rem" };
const warned = new Set<string>();

/** `xs..xl` token, bare number (pixels) or any CSS length. */
function cssSize(size: string): string {
  const token = SIZES[size];
  if (token) return token;
  return /^\d+(\.\d+)?$/.test(size) ? `${size}px` : size;
}

/**
 * Inline SVG icon from the bundled iconoir subset.
 * Attributes: `name` (registry key, e.g. `check-circle`), `size` (token, pixels or CSS length; default 1em),
 * `label` (turns it into an `img` with that accessible name; decorative otherwise).
 */
export class TxIcon extends LitElement {
  static properties = {
    name: { type: String, reflect: true },
    size: { type: String, reflect: true },
    label: { type: String },
  };

  static styles = css`
    :host {
      display: inline-flex;
      flex: none;
      width: var(--tx-icon-size, 1em);
      height: var(--tx-icon-size, 1em);
      vertical-align: middle;
      color: inherit;
    }
    svg {
      width: 100%;
      height: 100%;
    }
  `;

  name = "";
  size = "";
  label = "";

  protected willUpdate(): void {
    if (this.size) this.style.setProperty("--tx-icon-size", cssSize(this.size));
    else this.style.removeProperty("--tx-icon-size");
    if (this.label) {
      this.setAttribute("role", "img");
      this.setAttribute("aria-label", this.label);
    } else if (this.getAttribute("role") === "img") {
      this.removeAttribute("role");
      this.removeAttribute("aria-label");
    }
  }

  protected render() {
    const body = ICONS[this.name];
    if (!body) {
      if (this.name && !warned.has(this.name)) {
        warned.add(this.name);
        console.warn(`[tx-icon] unknown icon "${this.name}"`);
      }
      return nothing;
    }
    return html`<svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">${unsafeSVG(body)}</svg>`;
  }
}

if (!customElements.get("tx-icon")) customElements.define("tx-icon", TxIcon);
