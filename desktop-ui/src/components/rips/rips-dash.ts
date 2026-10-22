import { html } from "lit";
import type { TemplateResult } from "lit";

/**
 * Cell value, or a dimmed "-" when it is empty. A template function and not an element: a table
 * renders it for every cell of every row. The host styles `.dash` (muted color).
 */
export function dash(value?: string | number | null): TemplateResult | string | number {
  return value ? value : html`<span class="dash">-</span>`;
}
