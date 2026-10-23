import { LitElement, css, html } from "lit";
import { DOT_LABEL } from "../../utils/rips-constants";
import { dotToneOf, dotToneStyles } from "./rips-tones";

/** The four numbered radicación steps of a row, each toned by its status. */
export class RipsSteps extends LitElement {
  static properties = { list: { attribute: false } };

  static styles = [
    dotToneStyles,
    css`
      :host {
        display: block;
      }
      .steps {
        display: flex;
        flex-wrap: nowrap;
        justify-content: center;
        gap: 3px;
      }
      .step {
        display: grid;
        place-items: center;
        width: 17px;
        height: 17px;
        border-radius: 9999px;
        color: white;
        font-size: 10px;
        font-weight: 700;
      }
    `,
  ];

  list: readonly string[] | null | undefined = [];

  render() {
    return html`<div class="steps">
      ${(this.list || []).map((s, i) => {
        const name = `Paso ${i + 1}: ${(DOT_LABEL as Record<string, string>)[s] || s}`;
        return html`<span class="step tone-${dotToneOf(s)}" title=${name} aria-label=${name}>${i + 1}</span>`;
      })}
    </div>`;
  }
}

if (!customElements.get("rips-steps")) customElements.define("rips-steps", RipsSteps);
