import type { ReactiveController, ReactiveControllerHost } from "lit";

export type Theme = "light" | "dark";

export const THEME_KEY = "tessia.theme";
const CHANGE_EVENT = "tx-theme-change";

const isTheme = (v: unknown): v is Theme => v === "light" || v === "dark";

function stored(): Theme | null {
  try {
    const v = localStorage.getItem(THEME_KEY);
    return isTheme(v) ? v : null;
  } catch {
    return null;
  }
}

/** The saved choice, else the system preference. */
export function resolveTheme(): Theme {
  return stored() ?? (window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light");
}

/** Applies a previously saved choice; with none, tokens.css follows prefers-color-scheme. */
export function applyStoredTheme(): void {
  const t = stored();
  if (t) document.documentElement.dataset.theme = t;
  else delete document.documentElement.dataset.theme;
}

export function setTheme(theme: Theme): void {
  document.documentElement.dataset.theme = theme;
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch {
    // Private mode or a locked-down webview: the choice just lasts for this session.
  }
  window.dispatchEvent(new Event(CHANGE_EVENT));
}

export function toggleTheme(): Theme {
  const next: Theme = resolveTheme() === "dark" ? "light" : "dark";
  setTheme(next);
  return next;
}

/** Reactive controller: the effective theme, re-rendering the host when it changes. */
export class ThemeController implements ReactiveController {
  constructor(private host: ReactiveControllerHost) {
    host.addController(this);
  }

  get theme(): Theme {
    return resolveTheme();
  }

  toggle(): Theme {
    return toggleTheme();
  }

  hostConnected(): void {
    window.addEventListener(CHANGE_EVENT, this.update);
    window.matchMedia?.("(prefers-color-scheme: dark)").addEventListener?.("change", this.update);
  }

  hostDisconnected(): void {
    window.removeEventListener(CHANGE_EVENT, this.update);
    window.matchMedia?.("(prefers-color-scheme: dark)").removeEventListener?.("change", this.update);
  }

  private update = () => this.host.requestUpdate();
}
