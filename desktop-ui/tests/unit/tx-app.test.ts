import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import "../../src/components/tx-app";
import type { TxApp } from "../../src/components/tx-app";
import { THEME_KEY } from "../../src/theme/theme";
import { fakeApi, useApi } from "../support/ejecuciones-api";

// The shell mounts the job alert snackbar, which polls the API: keep it off the network.
vi.mock("../../src/api", async () => (await import("../support/ejecuciones-api")).apiModule);

async function mountApp(path = "/") {
  history.replaceState(null, "", path);
  const el = document.createElement("tx-app") as TxApp;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const links = (el: TxApp) => [...el.shadowRoot!.querySelectorAll<HTMLAnchorElement>("nav a")];
const outlet = (el: TxApp) => el.shadowRoot!.querySelector("#outlet")!.firstElementChild!;

function click(target: Element) {
  const event = new MouseEvent("click", { bubbles: true, composed: true, cancelable: true, button: 0 });
  target.dispatchEvent(event);
  return event;
}

// happy-dom follows a click nobody cancelled; the shell must cancel its own navigations.
const guard = (e: Event) => e.preventDefault();

beforeEach(() => {
  useApi(fakeApi());
  localStorage.clear();
  delete document.documentElement.dataset.theme;
  window.addEventListener("click", guard);
});
afterEach(() => {
  window.removeEventListener("click", guard);
  document.body.innerHTML = "";
  vi.restoreAllMocks();
});

describe("tx-app navigation", () => {
  it("renders every route in the sidebar with the same labels as the old app", async () => {
    const el = await mountApp();
    expect(links(el).map((a) => a.textContent!.trim())).toEqual(["Dashboard", "Ejecuciones", "Revisión manual", "Reportes", "Hallazgos", "Parámetros"]);
    expect(links(el).map((a) => a.getAttribute("href"))).toEqual(["/", "/ejecuciones", "/revision-manual", "/reportes", "/hallazgos", "/parametros"]);
  });

  it("mounts the app dialogs once, outside the outlet", async () => {
    const el = await mountApp();
    expect(el.shadowRoot!.querySelectorAll("tx-app-dialogs")).toHaveLength(1);
    expect(outlet(el).querySelector("tx-app-dialogs")).toBeNull();
  });

  it("shows the service health indicator in the navbar", async () => {
    const el = await mountApp();
    expect(el.shadowRoot!.querySelectorAll("header.navbar tx-health-indicator")).toHaveLength(1);
  });

  it("mounts the job alert snackbar once, outside the outlet", async () => {
    const el = await mountApp();
    expect(el.shadowRoot!.querySelectorAll("tx-job-alert-snackbar")).toHaveLength(1);
    expect(outlet(el).querySelector("tx-job-alert-snackbar")).toBeNull();
  });

  it("highlights the active route and shows its page and title", async () => {
    const el = await mountApp("/hallazgos");
    const active = links(el).filter((a) => a.getAttribute("aria-current") === "page");
    expect(active.map((a) => a.textContent!.trim())).toEqual(["Hallazgos"]);
    expect(outlet(el).localName).toBe("tx-page-hallazgos");
    expect(el.shadowRoot!.querySelector("h1")!.textContent).toBe("Hallazgos");
  });

  it("shows the Reportes page for /reportes, between Revisión manual and Hallazgos", async () => {
    const el = await mountApp("/reportes");
    expect(links(el).filter((a) => a.getAttribute("aria-current") === "page").map((a) => a.textContent!.trim())).toEqual(["Reportes"]);
    expect(outlet(el).localName).toBe("tx-page-reportes");
    expect(el.shadowRoot!.querySelector("h1")!.textContent).toBe("Reportes");
    expect(links(el)[3]!.querySelector("svg")).not.toBeNull(); // it has its own icon
  });

  it("clicking a link navigates without reloading the page", async () => {
    const el = await mountApp("/");
    const event = click(links(el).find((a) => a.textContent!.includes("Parámetros"))!);
    await el.updateComplete;
    expect(event.defaultPrevented).toBe(true);
    expect(location.pathname).toBe("/parametros");
    expect(outlet(el).localName).toBe("tx-page-parametros");
    expect(links(el).filter((a) => a.getAttribute("aria-current") === "page").map((a) => a.textContent!.trim())).toEqual(["Parámetros"]);
  });

  it("follows browser back/forward", async () => {
    const el = await mountApp("/");
    click(links(el)[1]!);
    await el.updateComplete;
    history.replaceState(null, "", "/revision-manual");
    window.dispatchEvent(new PopStateEvent("popstate"));
    await el.updateComplete;
    expect(outlet(el).localName).toBe("tx-page-revision-manual");
  });

  it("falls back to the dashboard on an unknown path", async () => {
    const el = await mountApp("/no-such-page");
    expect(location.pathname).toBe("/");
    expect(outlet(el).localName).toBe("tx-page-dashboard");
  });

});

describe("tx-app logo", () => {
  const sidebarLogo = (el: TxApp) => el.shadowRoot!.querySelector<HTMLImageElement>(".sidebar .brand img")!;
  const navbarLogo = (el: TxApp) => el.shadowRoot!.querySelector<HTMLImageElement>("header.navbar img")!;

  it("shows the full logo in the sidebar instead of the text", async () => {
    const el = await mountApp();
    const img = sidebarLogo(el);
    expect(img.getAttribute("alt")).toBe("Tessia");
    expect(img.getAttribute("src")).toBe("/logo.png");
    expect(img.getAttribute("width")).toBeTruthy();
    expect(img.getAttribute("height")).toBeTruthy();
    expect(el.shadowRoot!.querySelector(".brand")!.textContent!.trim()).toBe("");
  });

  it("uses the light-wordmark variant in the dark theme and switches back", async () => {
    localStorage.setItem(THEME_KEY, "dark");
    const el = await mountApp();
    expect(sidebarLogo(el).getAttribute("src")).toBe("/logo-dark.png");
    el.shadowRoot!.querySelector<HTMLButtonElement>(".theme-toggle")!.click();
    await el.updateComplete;
    expect(sidebarLogo(el).getAttribute("src")).toBe("/logo.png");
  });

  it("carries the icon-only image in the navbar for the narrow layout", async () => {
    const el = await mountApp();
    const img = navbarLogo(el);
    expect(img.getAttribute("alt")).toBe("Tessia");
    expect(img.getAttribute("src")).toBe("/logo-icon.png");
    expect(img.getAttribute("width")).toBeTruthy();
    expect(img.getAttribute("height")).toBeTruthy();
  });
});

describe("tx-app responsive menu", () => {
  it("toggles the drawer and closes it when a link is followed", async () => {
    const el = await mountApp("/");
    const toggle = el.shadowRoot!.querySelector<HTMLButtonElement>(".menu-toggle")!;
    const layout = el.shadowRoot!.querySelector(".layout")!;
    expect(toggle.getAttribute("aria-expanded")).toBe("false");
    toggle.click();
    await el.updateComplete;
    expect(layout.classList.contains("open")).toBe(true);
    expect(toggle.getAttribute("aria-expanded")).toBe("true");
    click(links(el)[1]!);
    await el.updateComplete;
    expect(layout.classList.contains("open")).toBe(false);
  });
});

describe("tx-app theme toggle", () => {
  const toggle = (el: TxApp) => el.shadowRoot!.querySelector<HTMLButtonElement>(".theme-toggle")!;

  it("switches the theme, labels the next action and persists the choice", async () => {
    const el = await mountApp();
    expect(toggle(el).getAttribute("aria-label")).toBe("Cambiar a tema oscuro");
    toggle(el).click();
    await el.updateComplete;
    expect(document.documentElement.dataset.theme).toBe("dark");
    expect(localStorage.getItem(THEME_KEY)).toBe("dark");
    expect(toggle(el).getAttribute("aria-label")).toBe("Cambiar a tema claro");
    toggle(el).click();
    await el.updateComplete;
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(localStorage.getItem(THEME_KEY)).toBe("light");
  });

  it("starts from the saved choice", async () => {
    localStorage.setItem(THEME_KEY, "dark");
    const el = await mountApp();
    expect(toggle(el).getAttribute("aria-label")).toBe("Cambiar a tema claro");
  });
});
