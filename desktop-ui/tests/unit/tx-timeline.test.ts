import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-timeline";
import type { TxTimeline } from "../../src/components/tx-timeline";

async function mount(props: Partial<TxTimeline> = {}, light = ""): Promise<TxTimeline> {
  const el = document.createElement("tx-timeline") as TxTimeline;
  el.items = [
    { title: "Radicada", description: "Paso 4", date: "2026-10-01", color: "success" },
    { title: "En revisión", icon: "history", color: "warning" },
    { title: "Cargada" },
  ];
  Object.assign(el, props);
  el.innerHTML = light;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const entries = (el: TxTimeline) => [...el.shadowRoot!.querySelectorAll<HTMLLIElement>("li")];

afterEach(() => {
  document.body.innerHTML = "";
});

describe("tx-timeline", () => {
  it("renders an ordered list with one entry per item", async () => {
    const el = await mount();
    expect(el.shadowRoot!.querySelector("ol")).not.toBeNull();
    expect(entries(el)).toHaveLength(3);
    expect(entries(el).map((li) => li.querySelector(".title")!.textContent!.trim())).toEqual(["Radicada", "En revisión", "Cargada"]);
  });

  it("shows description and date only when present", async () => {
    const el = await mount();
    const [first, , third] = entries(el);
    expect(first!.querySelector(".description")!.textContent!.trim()).toBe("Paso 4");
    expect(first!.querySelector("time")!.textContent!.trim()).toBe("2026-10-01");
    expect(first!.querySelector("time")!.getAttribute("datetime")).toBe("2026-10-01");
    expect(third!.querySelector(".description")).toBeNull();
    expect(third!.querySelector("time")).toBeNull();
  });

  it("colors each dot from the item color and falls back to the timeline color", async () => {
    const el = await mount({ color: "info" });
    const dots = entries(el).map((li) => li.querySelector<HTMLElement>(".dot")!.getAttribute("data-color"));
    expect(dots).toEqual(["success", "warning", "info"]);
  });

  it("draws the item icon inside the dot", async () => {
    const el = await mount();
    expect(entries(el)[1]!.querySelector(".dot svg")).not.toBeNull();
    expect(entries(el)[0]!.querySelector(".dot svg")).toBeNull();
  });

  it("does not draw a connector after the last item", async () => {
    const el = await mount();
    expect(entries(el).map((li) => li.hasAttribute("data-last"))).toEqual([false, false, true]);
  });

  it("lets `title-<index>` and `description-<index>` slots override the text", async () => {
    const el = await mount({}, `<b slot="title-0">Custom</b>`);
    const title = entries(el)[0]!.querySelector(".title slot")!;
    expect(title.getAttribute("name")).toBe("title-0");
    expect(entries(el)[0]!.querySelector(".description slot")!.getAttribute("name")).toBe("description-0");
  });

  it("renders an empty list without items", async () => {
    const el = await mount({ items: [] });
    expect(entries(el)).toHaveLength(0);
  });

  it("reflects size on the host for the dot dimensions", async () => {
    const el = await mount({ size: "lg" });
    expect(el.getAttribute("size")).toBe("lg");
  });
});
