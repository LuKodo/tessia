import { LitElement, css, html, svg, nothing } from "lit";
import { DOT_LABEL } from "../../utils/rips-constants";
import { dotToneOf, dotToneStyles } from "./rips-tones";

const CHECK = svg`<path d="m5 13 4 4L19 7" />`;
const CROSS = svg`<path d="M6 6l12 12M18 6 6 18" />`;

/**
 * Status dot of an artifact column: green check (ok), red cross (error), amber (en curso) or grey
 * (pendiente). The state is a native tooltip (`title`) and the accessible name: a table lists
 * hundreds of dots, so no tooltip element is created per dot.
 */
export class RipsDot extends LitElement {
  static properties = { status: { type: String }, label: { type: String } };

  static styles = [
    dotToneStyles,
    css`
      :host {
        display: inline-flex;
        vertical-align: middle;
      }
      .dot {
        display: inline-grid;
        place-items: center;
        width: 0.875rem;
        height: 0.875rem;
        border-radius: 9999px;
        color: white;
      }
      svg {
        width: 0.625rem;
        height: 0.625rem;
        fill: none;
        stroke: currentColor;
        stroke-width: 3;
        stroke-linecap: round;
        stroke-linejoin: round;
      }
    `,
  ];

  status = "";
  label = "";

  render() {
    const stateLabel = (DOT_LABEL as Record<string, string>)[this.status] || this.status;
    const name = `${this.label}: ${stateLabel}`;
    const glyph = this.status === "ok" ? CHECK : this.status === "error" ? CROSS : null;
    return html`<span class="dot tone-${dotToneOf(this.status)}" role="img" title=${name} aria-label=${name}
      >${glyph ? html`<svg viewBox="0 0 24 24" aria-hidden="true">${glyph}</svg>` : nothing}</span
    >`;
  }
}

if (!customElements.get("rips-dot")) customElements.define("rips-dot", RipsDot);
