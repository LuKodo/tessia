import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LitElement, html } from "lit";
import { THEME_KEY, ThemeController, applyStoredTheme, resolveTheme, setTheme, toggleTheme } from "../../src/theme/theme";

function prefersDark(dark: boolean) {
  vi.spyOn(window, "matchMedia").mockImplementation(
    (query: string) => ({ matches: dark && query.includes("dark"), media: query, addEventListener() {}, removeEventListener() {} }) as unknown as MediaQueryList,
  );
}

beforeEach(() => {
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  prefersDark(false);
});
afterEach(() => vi.restoreAllMocks());

describe("theme", () => {
  it("follows the system preference until the operator chooses", () => {
    expect(resolveTheme()).toBe("light");
    prefersDark(true);
    expect(resolveTheme()).toBe("dark");
  });

  it("setTheme persists the choice and applies data-theme", () => {
    setTheme("dark");
    expect(localStorage.getItem(THEME_KEY)).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(resolveTheme()).toBe("dark");
  });

  it("the stored choice wins over the system preference", () => {
    prefersDark(true);
    setTheme("light");
    expect(resolveTheme()).toBe("light");
  });

  it("toggleTheme flips the effective theme and stores it", () => {
    expect(toggleTheme()).toBe("dark");
    expect(toggleTheme()).toBe("light");
    expect(localStorage.getItem(THEME_KEY)).toBe("light");
  });

  it("applyStoredTheme restores a saved choice and ignores garbage", () => {
    localStorage.setItem(THEME_KEY, "dark");
    applyStoredTheme();
    expect(document.documentElement.dataset.theme).toBe("dark");
    delete document.documentElement.dataset.theme;
    localStorage.setItem(THEME_KEY, "purple");
    applyStoredTheme();
    expect(document.documentElement.dataset.theme).toBeUndefined();
    expect(resolveTheme()).toBe("light");
  });

  it("survives an unavailable localStorage", () => {
    vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
      throw new Error("denied");
    });
    expect(() => setTheme("dark")).not.toThrow();
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});

class Probe extends LitElement {
  theme = new ThemeController(this);
  render() {
    return html`${this.theme.theme}`;
  }
}
customElements.define("tx-theme-probe", Probe);

describe("ThemeController", () => {
  it("re-renders the host when the theme changes", async () => {
    const el = document.createElement("tx-theme-probe") as Probe;
    document.body.append(el);
    await el.updateComplete;
    expect(el.shadowRoot!.textContent).toBe("light");
    el.theme.toggle();
    await el.updateComplete;
    expect(el.shadowRoot!.textContent).toBe("dark");
    el.remove();
  });
});
