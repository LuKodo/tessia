import { afterEach, describe, expect, it, vi } from "vitest";
import "../../src/pages/page-parametros";
import type { TxCheckbox } from "../../src/components/tx-checkbox";
import type { PageParametros } from "../../src/pages/page-parametros";
import { WAVE_SIZE_HINT } from "../../src/utils/parameters-model";
import type { PreparationParametersResponse } from "../../src/types/api";
import { apiSlot, routedApi, settle } from "../support/parametros-routed-api";

vi.mock("../../src/api", async () => ({ api: (await import("../support/parametros-routed-api")).forwardingApi }));

const stored: PreparationParametersResponse = {
  max_invoice_amount: "150000",
  automatic_subplans: ["evento pós", "Plan Especial"],
  wave_size: 10,
  derechos_unavailable_percent: 30,
  derechos_min_sample: 5,
  cuv_token_failure_percent: 20,
  cuv_min_sample: 6,
  configured: true,
};

async function mountPage(routes: Parameters<typeof routedApi>[0] = {}) {
  const routed = routedApi({
    "GET /api/parameters": stored,
    "PUT /api/parameters": stored,
    "GET /api/nota-tecnica": { exists: false },
    ...routes,
  });
  apiSlot.api = routed.api;
  const el = document.createElement("tx-page-parametros") as PageParametros;
  document.body.append(el);
  await settle(el);
  return { ...routed, el };
}
type Page = Awaited<ReturnType<typeof mountPage>>["el"];

const q = <T extends Element>(el: Page, selector: string) => el.shadowRoot!.querySelector<T>(selector);
type Field = HTMLElement & { value: string };
const field = (el: Page, id: string) => q<Field>(el, `[data-testid=${id}]`)!;
async function type(el: Page, id: string, value: string) {
  const f = field(el, id);
  f.value = value;
  f.dispatchEvent(new CustomEvent("tx-input", { detail: { value }, bubbles: true, composed: true }));
  await settle(el);
}
const checkboxes = (el: Page) => [...q(el, "[data-testid=subplans]")!.shadowRoot!.querySelectorAll<TxCheckbox>("tx-checkbox")];
async function toggle(el: Page, index: number) {
  const box = checkboxes(el)[index]!;
  await box.updateComplete;
  const native = box.shadowRoot!.querySelector("input")!;
  native.checked = !native.checked;
  native.dispatchEvent(new Event("change", { bubbles: true }));
  await settle(el);
}
const submit = async (el: Page) => {
  q<HTMLFormElement>(el, "form")!.requestSubmit();
  await settle(el);
};

afterEach(() => {
  document.body.innerHTML = "";
});

describe("<tx-page-parametros>", () => {
  it("renders the heading, the hints and the nota técnica card", async () => {
    const { el } = await mountPage();
    expect(q(el, "h2")!.textContent).toBe("Parameters");
    expect(el.shadowRoot!.textContent).toContain("Configure the global rules used by convenio preparar.");
    expect(q(el, "tx-nota-tecnica")).not.toBeNull();
  });

  it("loads the stored parameters into the form and shows the persistent hints", async () => {
    const { el } = await mountPage();
    expect(field(el, "amount").value).toBe("150000");
    expect(field(el, "wave-size").value).toBe("10");
    expect(field(el, "cuv-percent").value).toBe("20");
    const text = [...el.shadowRoot!.querySelectorAll("tx-form-field")].map((f) => `${f.getAttribute("label")} ${f.getAttribute("help")}`).join("\n");
    expect(text).toContain(WAVE_SIZE_HINT);
    expect(text).toContain("Invoices above this amount will not be prepared automatically.");
    const checked = checkboxes(el).map((c) => c.checked);
    expect(checked).toEqual([true, false, false, true]);
    expect(q(el, "[data-testid=not-configured]")).toBeNull();
    expect(q(el, "[data-testid=form-error]")).toBeNull();
  });

  it("tells the operator nothing is saved yet when the service has no parameters", async () => {
    const { el } = await mountPage({ "GET /api/parameters": { ...stored, configured: false } });
    expect(q(el, "[data-testid=not-configured]")!.getAttribute("heading")).toContain("Not saved yet");
  });

  it("disables the form while loading", async () => {
    apiSlot.api = { parameters: () => new Promise(() => {}), notaTecnica: () => new Promise(() => {}) } as never;
    const el = document.createElement("tx-page-parametros") as PageParametros;
    document.body.append(el);
    await el.updateComplete;
    expect((field(el, "amount") as unknown as { disabled: boolean }).disabled).toBe(true);
    expect((q(el, "[data-testid=save]") as unknown as { disabled: boolean }).disabled).toBe(true);
    expect(q(el, "[data-testid=not-configured]")).toBeNull();
  });

  it("shows the validation error and does not save when a field is invalid", async () => {
    const { el, callsTo } = await mountPage();
    await type(el, "amount", "0");
    await submit(el);
    expect(q(el, "[data-testid=form-error]")!.getAttribute("description")).toBe("Enter a positive maximum amount using up to two decimals.");
    expect(q(el, "[data-testid=form-error]")!.getAttribute("heading")).toBe("Unable to load or save parameters");
    expect(callsTo("PUT", "/api/parameters")).toHaveLength(0);
  });

  it("saves the edited form with the same payload as before", async () => {
    const { el, callsTo } = await mountPage();
    await type(el, "amount", " 200000.50 ");
    await type(el, "wave-size", "25");
    await type(el, "derechos-percent", "40");
    await type(el, "derechos-min-sample", "7");
    await type(el, "cuv-percent", "0");
    await type(el, "cuv-min-sample", "9");
    // Untick "Plan Especial" (the last option) and tick the second default.
    const boxes = checkboxes(el);
    expect(boxes).toHaveLength(4);
    await toggle(el, 3);
    await toggle(el, 1);
    await submit(el);

    const [save] = callsTo("PUT", "/api/parameters");
    expect(save?.body).toEqual({
      max_invoice_amount: "200000.50",
      automatic_subplans: ["EVENTO POS", "EVENTO NO PBS"],
      wave_size: 25,
      derechos_unavailable_percent: 40,
      derechos_min_sample: 7,
      cuv_token_failure_percent: 0,
      cuv_min_sample: 9,
    });
    expect(q(el, "[data-testid=form-message]")!.getAttribute("heading")).toBe("Global preparation parameters saved.");
    expect(q(el, "[data-testid=form-error]")).toBeNull();
  });

  it("submits through the Save parameters button", async () => {
    const { el, callsTo } = await mountPage();
    q<HTMLElement>(el, "[data-testid=save]")!.click();
    await settle(el);
    expect(callsTo("PUT", "/api/parameters")).toHaveLength(1);
  });

  it("submits when Enter is pressed in a text field", async () => {
    const { el, callsTo } = await mountPage();
    field(el, "amount").dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, composed: true }));
    await settle(el);
    expect(callsTo("PUT", "/api/parameters")).toHaveLength(1);
  });

  it("reports a failed load", async () => {
    const { el } = await mountPage({ "GET /api/parameters": new Response(JSON.stringify({ error: "base de datos no disponible" }), { status: 503 }) });
    expect(q(el, "[data-testid=form-error]")!.getAttribute("description")).toContain("base de datos no disponible");
  });
});
