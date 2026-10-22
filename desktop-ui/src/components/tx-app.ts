import { LitElement, css } from "lit";
import { html, unsafeStatic } from "lit/static-html.js";
import { ROUTES, RouterController } from "../router/router";
import { ThemeController } from "../theme/theme";
import { icon } from "./shell-icons";
import "./tx-app-dialogs";
import "./job-alert-snackbar";
import "./health-indicator";

/** App shell: sidebar navigation, navbar with the theme toggle, and the outlet of the current route. */
export class TxApp extends LitElement {
  static properties = { menuOpen: { state: true } };

  static styles = css`
    :host {
      display: block;
      min-height: 100vh;
    }
    .layout {
      display: grid;
      grid-template-columns: var(--tx-sidebar-width) 1fr;
      min-height: 100vh;
    }
    .sidebar {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-4);
      padding: var(--tx-space-4);
      border-right: 1px solid var(--tx-border);
      background: var(--tx-bg);
    }
    .brand {
      display: flex;
      align-items: center;
      justify-content: space-between;
      height: calc(var(--tx-navbar-height) - var(--tx-space-4));
    }
    /* Both images carry width/height attributes (their aspect ratio); the height decides the size.
       The wordmark of logo.png is dark slate, unreadable on the dark surfaces, so the dark theme loads
       logo-dark.png (same art, light wordmark) instead; see ui/scripts/make-logo-assets.py. */
    .logo-full {
      display: block;
      height: 2.25rem;
      width: auto;
    }
    .logo-icon {
      display: none;
      height: 2rem;
      width: auto;
    }
    nav {
      display: flex;
      flex-direction: column;
      gap: var(--tx-space-1);
    }
    nav a {
      display: flex;
      align-items: center;
      gap: var(--tx-space-3);
      padding: var(--tx-space-2) var(--tx-space-3);
      border-radius: var(--tx-radius);
      color: var(--tx-text-muted);
      text-decoration: none;
      font-weight: 500;
    }
    nav a:hover {
      background: var(--tx-bg-muted);
      color: var(--tx-text-highlighted);
    }
    nav a[aria-current="page"] {
      background: var(--tx-primary-soft);
      color: var(--tx-primary);
    }
    .panel {
      display: flex;
      flex-direction: column;
      min-width: 0;
    }
    header.navbar {
      display: flex;
      align-items: center;
      gap: var(--tx-space-3);
      height: var(--tx-navbar-height);
      padding: 0 var(--tx-space-4);
      border-bottom: 1px solid var(--tx-border);
    }
    header.navbar h1 {
      flex: 1;
      margin: 0;
      font-size: var(--tx-text-lg);
      font-weight: 700;
      color: var(--tx-text-highlighted);
    }
    button {
      display: inline-grid;
      place-items: center;
      width: 2.25rem;
      height: 2.25rem;
      padding: 0;
      border: 0;
      border-radius: var(--tx-radius);
      background: transparent;
      color: var(--tx-text-muted);
      cursor: pointer;
    }
    button:hover {
      background: var(--tx-bg-muted);
      color: var(--tx-text-highlighted);
    }
    :is(a, button):focus-visible {
      outline: var(--tx-focus-ring);
      outline-offset: 2px;
    }
    main {
      flex: 1;
      padding: var(--tx-space-6);
    }
    .menu-toggle,
    .close,
    .backdrop {
      display: none;
    }
    /* Narrow screens: the sidebar becomes an off-canvas drawer opened from the navbar. */
    @media (max-width: 768px) {
      .layout {
        grid-template-columns: 1fr;
      }
      .sidebar {
        position: fixed;
        inset: 0 auto 0 0;
        z-index: 20;
        width: min(var(--tx-sidebar-width), 80vw);
        transform: translateX(-100%);
        transition: transform 0.2s ease;
      }
      .layout.open .sidebar {
        transform: none;
        box-shadow: var(--tx-shadow);
      }
      .menu-toggle,
      .close {
        display: inline-grid;
      }
      /* With the sidebar tucked away, the navbar carries the compact mark. */
      .logo-icon {
        display: block;
      }
      .layout.open .backdrop {
        display: block;
        position: fixed;
        inset: 0;
        z-index: 10;
        background: rgb(0 0 0 / 0.4);
      }
      main {
        padding: var(--tx-space-4);
      }
    }
    @media (prefers-reduced-motion: reduce) {
      .sidebar {
        transition: none;
      }
    }
  `;

  private router = new RouterController(this);
  private theme = new ThemeController(this);
  menuOpen = false;

  private offNavigate?: () => void;

  // Following a link closes the drawer.
  connectedCallback(): void {
    super.connectedCallback();
    this.offNavigate = this.router.router.subscribe(() => (this.menuOpen = false));
  }

  disconnectedCallback(): void {
    this.offNavigate?.();
    super.disconnectedCallback();
  }

  render() {
    const current = this.router.route;
    const dark = this.theme.theme === "dark";
    const outlet = unsafeStatic(current.tag);
    return html`
      <div class="layout ${this.menuOpen ? "open" : ""}">
        <aside class="sidebar" id="sidebar">
          <div class="brand">
            <img class="logo-full" src=${dark ? "/logo-dark.png" : "/logo.png"} alt="Tessia" width="1000" height="313" />
            <button class="close" type="button" aria-label="Cerrar menú" @click=${() => (this.menuOpen = false)}>${icon("close")}</button>
          </div>
          <nav aria-label="Navegación principal">
            ${ROUTES.map(
              (r) => html`<a href=${r.path} aria-current=${r.path === current.path ? "page" : "false"}>${icon(r.icon)}<span>${r.label}</span></a>`,
            )}
          </nav>
        </aside>
        <div class="backdrop" @click=${() => (this.menuOpen = false)}></div>
        <div class="panel">
          <header class="navbar">
            <button
              class="menu-toggle"
              type="button"
              aria-label="Abrir menú"
              aria-controls="sidebar"
              aria-expanded=${this.menuOpen ? "true" : "false"}
              @click=${() => (this.menuOpen = !this.menuOpen)}
            >
              ${icon("menu")}
            </button>
            <img class="logo-icon" src="/logo-icon.png" alt="Tessia" width="512" height="512" />
            <h1>${current.label}</h1>
            <tx-health-indicator></tx-health-indicator>
            <button
              class="theme-toggle"
              type="button"
              aria-label=${dark ? "Cambiar a tema claro" : "Cambiar a tema oscuro"}
              @click=${() => this.theme.toggle()}
            >
              ${icon(dark ? "sun" : "moon")}
            </button>
          </header>
          <main id="outlet">${html`<${outlet}></${outlet}>`}</main>
        </div>
      </div>
      <tx-app-dialogs></tx-app-dialogs>
      <tx-job-alert-snackbar></tx-job-alert-snackbar>
    `;
  }
}

if (!customElements.get("tx-app")) customElements.define("tx-app", TxApp);
