/**
 * Icon registry of the RIPS dashboard (port of rips/icons.js). The keys used by the KPI cards
 * and the text filters come first; the rest are used directly by the components.
 *
 * Every value is a LITERAL `i-iconoir-*` class: the icon bundle only includes names that appear
 * as literal strings under `app/`, so they must not be built dynamically.
 */
export const RIPS_ICONS = {
  list: "i-iconoir-list",
  circleMinus: "i-iconoir-minus-circle",
  circleCheck: "i-iconoir-check-circle",
  circleX: "i-iconoir-xmark-circle",
  hourglass: "i-iconoir-hourglass",
  fileAlert: "i-iconoir-page-edit",
  send: "i-iconoir-send",
  receipt: "i-iconoir-page-minus",
  shieldX: "i-iconoir-shield-xmark",
  fileText: "i-iconoir-page",
  stack: "i-iconoir-book-stack",
  hash: "i-iconoir-hashtag",
  shieldCheck: "i-iconoir-shield-check",
  id: "i-iconoir-user-badge-check",
  tag: "i-iconoir-label",
  check: "i-iconoir-check",
  checkBold: "i-iconoir-check",
  closeBold: "i-iconoir-xmark",
  close: "i-iconoir-xmark",
  eye: "i-iconoir-eye",
  sort: "i-iconoir-sort",
  arrowUp: "i-iconoir-arrow-up",
  arrowDown: "i-iconoir-arrow-down",
  chartBar: "i-iconoir-stats-report",
  info: "i-iconoir-info-circle",
  circleCheckFilled: "i-iconoir-check-circle-solid",
  alertCircleFilled: "i-iconoir-warning-circle-solid",
  shieldAlert: "i-iconoir-shield-alert",
  chevronLeft: "i-iconoir-nav-arrow-left",
  chevronRight: "i-iconoir-nav-arrow-right",
} as const;

export type RipsIconKey = keyof typeof RIPS_ICONS;
