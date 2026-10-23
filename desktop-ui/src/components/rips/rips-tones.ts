import { css } from "lit";
import type { DotColor, KpiColor } from "../../utils/rips-constants";
import { DOT_COLOR } from "../../utils/rips-constants";
import { RIPS_ICONS } from "../../utils/rips-icons";
import type { RipsIconKey } from "../../utils/rips-icons";

/** Registry key of a `tx-icon` for an `RIPS_ICONS` entry (`i-iconoir-check` -> `check`). */
export function ripsIcon(key: RipsIconKey): string {
  return RIPS_ICONS[key].replace(/^i-iconoir-/, "");
}

/** Registry key of a `tx-icon` for an iconoir class such as the ones portal-model returns. */
export function iconName(iconoirClass: string): string {
  return iconoirClass.replace(/^i-iconoir-/, "");
}

/** Tone of an artifact / radicación dot or step; anything unknown is pending (neutral). */
export function dotToneOf(status: string): DotColor {
  return (DOT_COLOR as Record<string, DotColor>)[status] ?? "neutral";
}

/**
 * Hue of each KPI card as `light-dark(<600 shade>, <400 shade>)`; the old app used the Tailwind
 * palette, so the literal values are the same ones.
 */
export const KPI_HUE: Record<KpiColor, string> = {
  cyan: "light-dark(oklch(60.9% 0.126 221.7), oklch(78.9% 0.154 211.5))",
  neutral: "light-dark(var(--tx-taupe-600), var(--tx-taupe-400))",
  green: "light-dark(oklch(62.7% 0.194 149.2), oklch(79.2% 0.209 151.7))",
  red: "light-dark(oklch(57.7% 0.245 27.3), oklch(70.4% 0.191 22.2))",
  blue: "light-dark(oklch(54.6% 0.245 262.9), oklch(70.7% 0.165 254.6))",
  pink: "light-dark(oklch(59.2% 0.249 0.6), oklch(71.8% 0.202 349.8))",
  orange: "light-dark(oklch(64.6% 0.222 41.1), oklch(75% 0.183 55.9))",
  purple: "light-dark(oklch(55.8% 0.288 302.3), oklch(71.4% 0.203 305.5))",
  violet: "light-dark(oklch(54.1% 0.281 293), oklch(70.2% 0.183 293.5))",
};

/** Background of a dot or step per tone; the glyph is always white. */
export const dotToneStyles = css`
  .tone-success {
    background: var(--tx-success);
  }
  .tone-error {
    background: var(--tx-error);
  }
  .tone-warning {
    background: var(--tx-warning);
  }
  .tone-neutral {
    background: var(--tx-taupe-400);
  }
`;

/** Spinner animation shared by the loading states. */
export const spinStyles = css`
  @keyframes rips-spin {
    to {
      transform: rotate(360deg);
    }
  }
  .spin {
    animation: rips-spin 1s linear infinite;
  }
  @media (prefers-reduced-motion: reduce) {
    .spin {
      animation: none;
    }
  }
`;

/** Series colors of the cargue chart, shared by the bars and the legend (same values as the old SVG). */
export const chartPalette = css`
  :host {
    --seg-filed: #2f9e44;
    --seg-managed: #1c7ed6;
    --seg-manual: #f08c00;
    --seg-excluded: #868e96;
  }
`;
