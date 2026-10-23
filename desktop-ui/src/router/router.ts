import type { ReactiveController, ReactiveControllerHost } from "lit";

export interface Route {
  path: string;
  /** Custom element rendered in the outlet. */
  tag: string;
  /** Sidebar and title text (Spanish UI copy). */
  label: string;
  /** Key into the shell's icon set. */
  icon: string;
}

/** Same order and labels as the sidebar of the old app; "/" is also the 404 fallback. */
export const ROUTES: readonly Route[] = [
  { path: "/", tag: "tx-page-dashboard", label: "Dashboard", icon: "dashboard" },
  { path: "/ejecuciones", tag: "tx-page-ejecuciones", label: "Ejecuciones", icon: "history" },
  { path: "/revision-manual", tag: "tx-page-revision-manual", label: "Revisión manual", icon: "review" },
  { path: "/reportes", tag: "tx-page-reportes", label: "Reportes", icon: "reports" },
  { path: "/hallazgos", tag: "tx-page-hallazgos", label: "Hallazgos", icon: "findings" },
  { path: "/parametros", tag: "tx-page-parametros", label: "Parámetros", icon: "settings" },
];

const normalize = (pathname: string) => (pathname.length > 1 ? pathname.replace(/\/+$/, "") : pathname) || "/";
const find = (routes: readonly Route[], pathname: string) => routes.find((r) => r.path === normalize(pathname));

/** The route of a pathname (trailing slash ignored); anything unknown falls back to "/". */
export function matchRoute(pathname: string, routes: readonly Route[] = ROUTES): Route {
  return find(routes, pathname) ?? routes[0]!;
}

type Listener = () => void;

/**
 * History API router. The current route is always derived from `location`, so there is no state to
 * keep in sync. `attach()`/`detach()` are reference counted (one per connected host); the listeners
 * live only while someone is attached.
 */
export class Router {
  private listeners = new Set<Listener>();
  private attached = 0;

  constructor(
    private routes: readonly Route[] = ROUTES,
    private win: Window = window,
  ) {}

  get route(): Route {
    return matchRoute(this.win.location.pathname, this.routes);
  }

  attach(): void {
    if (this.attached++ > 0) return;
    // 404 fallback: an unknown location is rewritten to "/" without adding a history entry.
    if (!find(this.routes, this.win.location.pathname)) this.win.history.replaceState(null, "", "/");
    this.win.addEventListener("popstate", this.notify);
    this.win.document.addEventListener("click", this.onClick);
  }

  detach(): void {
    if (this.attached === 0 || --this.attached > 0) return;
    this.win.removeEventListener("popstate", this.notify);
    this.win.document.removeEventListener("click", this.onClick);
  }

  subscribe(listener: Listener): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** `url` is a path with an optional query/hash; an unknown path becomes "/". */
  navigate(url: string, options: { replace?: boolean } = {}): void {
    const target = new URL(url, this.win.location.href);
    const known = find(this.routes, target.pathname);
    const next = known ? `${known.path}${target.search}${target.hash}` : "/";
    const current = `${this.win.location.pathname}${this.win.location.search}${this.win.location.hash}`;
    if (next !== current) this.win.history[options.replace ? "replaceState" : "pushState"](null, "", next);
    this.notify();
  }

  private notify = () => this.listeners.forEach((l) => l());

  // Only plain same-origin clicks on links to a known route; everything else is the browser's.
  private onClick = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    // composedPath crosses shadow roots, where the nav links live.
    const anchor = event.composedPath().find((n): n is HTMLAnchorElement => n instanceof HTMLAnchorElement);
    if (!anchor || !anchor.hasAttribute("href") || anchor.hasAttribute("download")) return;
    const target = anchor.getAttribute("target");
    if (target && target !== "_self") return;
    const url = new URL(anchor.href, this.win.location.href);
    if (url.origin !== this.win.location.origin || !find(this.routes, url.pathname)) return;
    // In-page anchors keep the browser's own scrolling.
    if (url.hash && url.pathname === this.win.location.pathname && url.search === this.win.location.search) return;
    event.preventDefault();
    this.navigate(url.pathname + url.search + url.hash);
  };
}

let shared: Router | undefined;
/** The app-wide router, created on first use. */
export function getRouter(): Router {
  return (shared ??= new Router());
}

/** Reactive controller: exposes the current route and re-renders the host when it changes. */
export class RouterController implements ReactiveController {
  private off?: () => void;

  constructor(
    private host: ReactiveControllerHost,
    readonly router: Router = getRouter(),
  ) {
    host.addController(this);
  }

  get route(): Route {
    return this.router.route;
  }

  hostConnected(): void {
    this.router.attach();
    this.off = this.router.subscribe(() => this.host.requestUpdate());
  }

  hostDisconnected(): void {
    this.off?.();
    this.router.detach();
  }
}
