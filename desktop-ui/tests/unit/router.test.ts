import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LitElement, html } from "lit";
import { ROUTES, Router, RouterController, matchRoute } from "../../src/router/router";

function click(el: Element, init: MouseEventInit = {}) {
  const event = new MouseEvent("click", { bubbles: true, composed: true, cancelable: true, button: 0, ...init });
  el.dispatchEvent(event);
  return event;
}

describe("matchRoute", () => {
  it("matches every route of the table", () => {
    expect(ROUTES.map((r) => r.path)).toEqual(["/", "/ejecuciones", "/revision-manual", "/reportes", "/hallazgos", "/parametros"]);
    for (const route of ROUTES) expect(matchRoute(route.path)).toBe(route);
  });

  it("ignores a trailing slash", () => {
    expect(matchRoute("/hallazgos/").path).toBe("/hallazgos");
  });

  it("falls back to the dashboard for an unknown path", () => {
    expect(matchRoute("/nope").path).toBe("/");
    expect(matchRoute("/hallazgos/extra").path).toBe("/");
  });
});

describe("Router", () => {
  let router: Router;
  beforeEach(() => {
    history.replaceState(null, "", "/");
    router = new Router(ROUTES, window);
    router.attach();
  });
  afterEach(() => router.detach());

  it("starts on the current location", () => {
    expect(router.route.path).toBe("/");
  });

  it("navigate pushes a history entry and notifies subscribers", () => {
    const seen = vi.fn();
    router.subscribe(seen);
    router.navigate("/hallazgos");
    expect(location.pathname).toBe("/hallazgos");
    expect(router.route.path).toBe("/hallazgos");
    expect(seen).toHaveBeenCalledTimes(1);
  });

  it("does not push a duplicate entry for the same url", () => {
    router.navigate("/hallazgos");
    const length = history.length;
    router.navigate("/hallazgos");
    expect(history.length).toBe(length);
  });

  it("replaces the entry when asked", () => {
    const length = history.length;
    router.navigate("/parametros", { replace: true });
    expect(history.length).toBe(length);
    expect(location.pathname).toBe("/parametros");
  });

  it("keeps the query string and still matches by path", () => {
    router.navigate("/hallazgos?convenio=X");
    expect(location.search).toBe("?convenio=X");
    expect(router.route.path).toBe("/hallazgos");
  });

  it("rewrites an unknown location to / on attach (404 fallback)", () => {
    router.detach();
    history.replaceState(null, "", "/no-such-page");
    router.attach();
    expect(location.pathname).toBe("/");
    expect(router.route.path).toBe("/");
  });

  it("reacts to popstate", () => {
    router.navigate("/ejecuciones");
    const seen = vi.fn();
    router.subscribe(seen);
    history.replaceState(null, "", "/parametros");
    window.dispatchEvent(new PopStateEvent("popstate"));
    expect(seen).toHaveBeenCalledTimes(1);
    expect(router.route.path).toBe("/parametros");
  });

  it("unsubscribe stops notifications", () => {
    const seen = vi.fn();
    const off = router.subscribe(seen);
    off();
    router.navigate("/hallazgos");
    expect(seen).not.toHaveBeenCalled();
  });

  describe("link interception", () => {
    function link(attrs: Record<string, string>, parent: ParentNode = document.body) {
      const a = document.createElement("a");
      for (const [k, v] of Object.entries(attrs)) a.setAttribute(k, v);
      parent.append(a);
      return a;
    }
    // happy-dom follows an unhandled link click; record the verdict, then cancel it like a stand-in browser.
    let leftToBrowser: boolean | undefined;
    const guard = (e: Event) => {
      leftToBrowser = !e.defaultPrevented;
      e.preventDefault();
    };
    beforeEach(() => {
      leftToBrowser = undefined;
      window.addEventListener("click", guard);
    });
    afterEach(() => {
      window.removeEventListener("click", guard);
      document.body.innerHTML = "";
    });

    it("navigates on a plain same-origin click without reloading", () => {
      click(link({ href: "/hallazgos" }));
      expect(leftToBrowser).toBe(false);
      expect(router.route.path).toBe("/hallazgos");
    });

    it("finds the anchor through shadow roots", () => {
      const host = document.createElement("div");
      document.body.append(host);
      const root = host.attachShadow({ mode: "open" });
      const a = link({ href: "/parametros" }, root);
      const inner = document.createElement("span");
      a.append(inner);
      click(inner);
      expect(leftToBrowser).toBe(false);
      expect(router.route.path).toBe("/parametros");
    });

    it.each([
      ["a modified click", { href: "/hallazgos" }, { ctrlKey: true }],
      ["a non-primary button", { href: "/hallazgos" }, { button: 1 }],
      ["target=_blank", { href: "/hallazgos", target: "_blank" }, {}],
      ["a download link", { href: "/hallazgos", download: "" }, {}],
      ["another origin", { href: "https://example.org/hallazgos" }, {}],
      ["an unknown same-origin path (e.g. an API file)", { href: "/api/export.csv" }, {}],
      ["a hash-only link", { href: "#top" }, {}],
    ])("leaves %s to the browser", (_name, attrs, init) => {
      click(link(attrs), init);
      expect(leftToBrowser).toBe(true);
      expect(router.route.path).toBe("/");
    });

    it("stops intercepting after the last detach", () => {
      router.detach();
      click(link({ href: "/hallazgos" }));
      expect(leftToBrowser).toBe(true);
      expect(router.route.path).toBe("/");
      router.attach(); // balance the afterEach detach
    });
  });
});

class Probe extends LitElement {
  router = new Router(ROUTES, window);
  ctrl = new RouterController(this, this.router);
  render() {
    return html`${this.ctrl.route.label}`;
  }
}
customElements.define("tx-router-probe", Probe);

describe("RouterController", () => {
  beforeEach(() => history.replaceState(null, "", "/"));

  it("exposes the current route and re-renders the host on navigation", async () => {
    const el = document.createElement("tx-router-probe") as Probe;
    document.body.append(el);
    await el.updateComplete;
    expect(el.shadowRoot!.textContent).toBe("Dashboard");
    el.router.navigate("/hallazgos");
    await el.updateComplete;
    expect(el.shadowRoot!.textContent).toBe("Hallazgos");
    el.remove();
  });

  it("detaches from the router when the host disconnects", async () => {
    const el = document.createElement("tx-router-probe") as Probe;
    document.body.append(el);
    await el.updateComplete;
    el.remove();
    const a = Object.assign(document.createElement("a"), { href: "/hallazgos" });
    document.body.append(a);
    let prevented: boolean | undefined;
    const guard = (e: Event) => {
      prevented = e.defaultPrevented;
      e.preventDefault();
    };
    window.addEventListener("click", guard);
    click(a);
    window.removeEventListener("click", guard);
    expect(prevented).toBe(false);
    a.remove();
  });
});
