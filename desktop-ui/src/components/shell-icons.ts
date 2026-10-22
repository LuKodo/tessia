import { svg, html, type SVGTemplateResult, type TemplateResult } from "lit";

// Minimal stroke icon set (24x24, currentColor). The shell uses these; primitives grow the set later.
const PATHS: Record<string, SVGTemplateResult> = {
  dashboard: svg`<rect x="3" y="3" width="7" height="7" rx="1"/><rect x="14" y="3" width="7" height="7" rx="1"/><rect x="3" y="14" width="7" height="7" rx="1"/><rect x="14" y="14" width="7" height="7" rx="1"/>`,
  history: svg`<path d="M3 12a9 9 0 1 0 3-6.7L3 8"/><path d="M3 3v5h5"/><path d="M12 7v5l3 2"/>`,
  review: svg`<rect x="5" y="4" width="14" height="17" rx="2"/><path d="M9 4V3h6v1"/><path d="m9 13 2 2 4-4"/>`,
  reports: svg`<path d="M4 20h16"/><rect x="5" y="11" width="3" height="6" rx="0.5"/><rect x="10.5" y="6" width="3" height="11" rx="0.5"/><rect x="16" y="13" width="3" height="4" rx="0.5"/>`,
  findings: svg`<path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h4"/><path d="M14 3l5 5v3"/><path d="M14 3v5h5"/><circle cx="16.5" cy="16.5" r="3"/><path d="m21 21-2.4-2.4"/>`,
  settings: svg`<path d="M4 7h9"/><path d="M17 7h3"/><circle cx="15" cy="7" r="2"/><path d="M4 17h3"/><path d="M11 17h9"/><circle cx="9" cy="17" r="2"/>`,
  menu: svg`<path d="M4 6h16M4 12h16M4 18h16"/>`,
  sun: svg`<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.9 4.9l1.4 1.4M17.7 17.7l1.4 1.4M2 12h2M20 12h2M4.9 19.1l1.4-1.4M17.7 6.3l1.4-1.4"/>`,
  moon: svg`<path d="M20 14.5A8 8 0 0 1 9.5 4 8 8 0 1 0 20 14.5Z"/>`,
  close: svg`<path d="M6 6l12 12M18 6 6 18"/>`,
};

export type IconName = keyof typeof PATHS;

/** Decorative inline SVG; the surrounding control carries the accessible name. */
export function icon(name: string, size = 20): TemplateResult {
  return html`<svg
    width=${size}
    height=${size}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.75"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    ${PATHS[name]}
  </svg>`;
}
