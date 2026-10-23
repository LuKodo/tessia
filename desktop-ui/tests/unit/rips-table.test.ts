import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/rips/rips-table";
import type { TrackingRow } from "../../src/types/api";
import { click, deepText, mountEl, q, qa, settle, text } from "../support/dashboard-dom";
import { trackingRow } from "../support/dashboard-fixtures";

afterEach(() => {
  document.body.innerHTML = "";
});

type Props = {
  rows: TrackingRow[];
  total?: number;
  filtered?: boolean;
  hasMore?: boolean;
  loading?: boolean;
  loadingMore?: boolean;
  error?: string;
  sort?: { key: string | null; dir: 1 | -1 };
};
async function mountTable(props: Props) {
  const el = await mountEl("rips-table", { loading: false, ...props });
  await settle(el);
  return el;
}

const bodyRows = (el: Element) => qa(el, "tbody tr");
const summary = (el: Element) => q(el, ".summary")!.textContent!.trim();
const invoices = (el: Element) => qa(el, ".invoice-link").map((b) => b.textContent!.trim());
const attrOf = (el: Element, name: string) => el.getAttribute(name);

describe("<rips-table>", () => {
  it("renders one row per invoice with its dots, steps, estado and amounts", async () => {
    const el = await mountTable({
      rows: [
        trackingRow({
          factura: "FVS1001",
          artefactos: ["ok", "error", "pendiente", "en_curso"],
          radicacion: ["ok", "ok", "pendiente", "error"],
          estado: "REVISION_MANUAL",
          estado_detalle: "radicado confirmado=true",
          ssc: "5618214",
          subplan: "EVENTO POS",
          valor_xml: "1500",
          valor_convenio: "1400.5",
          autorizacion: "A-9",
          id_mipres: "M-1",
        }),
      ],
    });
    const row = bodyRows(el)[0]!;
    const t = deepText(row);
    expect(t).toContain("FVS1001");
    expect(qa(row, "rips-dot")).toHaveLength(4);
    const steps = q(row, "rips-steps")!;
    expect(qa(steps, ".step").map((s) => s.className.match(/tone-(\w+)/)?.[1])).toEqual(["success", "success", "neutral", "error"]);
    expect(t).toContain("REVISION MANUAL");
    expect(t).toContain("radicado confirmado=true");
    expect(t).toContain("5618214");
    expect(t).toContain("EVENTO POS");
    expect(t).toContain("XML: $ 1.500");
    expect(t).toContain("Convenio: $ 1.400,5");
    expect(t).toContain("A-9");
    expect(t).toContain("M-1");
  });

  it("labels each artifact dot in column order", async () => {
    const el = await mountTable({ rows: [trackingRow({ artefactos: ["ok", "error", "pendiente", "en_curso"] })] });
    const labels = qa(bodyRows(el)[0]!, "rips-dot").map((d) => `${attrOf(d, "label") ?? (d as unknown as { label: string }).label}:${(d as unknown as { status: string }).status}`);
    expect(labels).toEqual(["ZIP XML/PDF:ok", "JSON RIPS:error", "TXT CUV:pendiente", "ZIP CRC:en_curso"]);
  });

  it("shows dashes for empty cells and SIN INICIAR for a fresh row, with no delivery-support column", async () => {
    const el = await mountTable({ rows: [trackingRow({ fecha_factura: undefined })] });
    const row = bodyRows(el)[0]!;
    expect(deepText(row)).toContain("SIN INICIAR");
    expect(deepText(row)).not.toContain("Por validar");
    expect(text(el)).not.toContain("Soporte de entrega");
    expect(qa(row, "td").filter((td) => td.textContent!.trim() === "-").length).toBeGreaterThanOrEqual(5);
  });

  it("renders the rows it gets as they come (the server filters, sorts and pages)", async () => {
    const el = await mountTable({ rows: [trackingRow({ factura: "Z9" }), trackingRow({ factura: "A1", pipeline: "no_aplica" }), trackingRow({ factura: "M5" })], total: 3 });
    expect(invoices(el)).toEqual(["Z9", "A1", "M5"]);
  });

  it("says how many invoices there are and how many are loaded", async () => {
    const el = await mountTable({ rows: [trackingRow({ factura: "A1" }), trackingRow({ factura: "B2" })], total: 2 });
    expect(summary(el)).toBe("2 facturas");
    el.remove();
    const partial = await mountTable({ rows: [trackingRow({ factura: "A1" }), trackingRow({ factura: "B2" })], total: 950, hasMore: true });
    expect(summary(partial)).toBe("950 facturas · mostrando 2");
    const one = await mountTable({ rows: [trackingRow()], total: 1 });
    expect(summary(one)).toBe("1 factura");
  });

  it("marks the count as filtered, and falls back to the loaded count while the total is unknown", async () => {
    const filtered = await mountTable({ rows: [trackingRow({ factura: "B2" })], total: 1, filtered: true });
    expect(summary(filtered)).toBe("1 factura (filtradas)");
    const unknown = await mountTable({ rows: [trackingRow({ factura: "A1" }), trackingRow({ factura: "B2" })] });
    expect(summary(unknown)).toBe("2 facturas cargadas");
  });

  it("re-renders when the rows change", async () => {
    const el = await mountTable({ rows: [trackingRow({ factura: "A1" }), trackingRow({ factura: "B2" })] });
    expect(invoices(el)).toEqual(["A1", "B2"]);
    Object.assign(el, { rows: [trackingRow({ factura: "A1" }), trackingRow({ factura: "C3" })] });
    await settle(el);
    expect(invoices(el)).toEqual(["A1", "C3"]);
  });

  it("explains the empty states and the first load", async () => {
    const loading = await mountTable({ rows: [], loading: true });
    expect(text(loading)).toContain("Cargando facturas…");
    const none = await mountTable({ rows: [] });
    expect(text(none)).toContain("Sin facturas cargadas");
    const filtered = await mountTable({ rows: [], filtered: true, total: 0 });
    expect(text(filtered)).toContain("Ninguna factura coincide con el filtro");
  });

  it("shows the load error as an alert, with the rows it has", async () => {
    const el = await mountTable({ rows: [trackingRow({ factura: "A1" })], error: "sin base" });
    const alert = q(el, ".load-error")!;
    expect(alert.getAttribute("description")).toBe("No se pudieron cargar las facturas: sin base");
    expect(alert.getAttribute("role")).toBe("alert");
    expect(invoices(el)).toEqual(["A1"]);
    const clean = await mountTable({ rows: [trackingRow()] });
    expect(q(clean, ".load-error")).toBeNull();
  });

  it("asks for a new sort with the column key and leaves the order to the server", async () => {
    const rows = [
      trackingRow({ factura: "M", fecha_factura: "2026-10-02" }),
      trackingRow({ factura: "O", fecha_factura: "2026-10-01" }),
      trackingRow({ factura: "N", fecha_factura: "2026-10-03" }),
    ];
    const el = await mountTable({ rows });
    const seen: string[] = [];
    el.addEventListener("sort-change", (e) => seen.push((e as CustomEvent<string>).detail));
    const header = () => qa(el, "th").find((th) => th.textContent!.trim().startsWith("Fecha factura"))!;
    click(header());
    await settle(el);
    expect(seen).toEqual(["fecha_factura"]);
    expect(invoices(el)).toEqual(["M", "O", "N"]);
    header().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(seen).toEqual(["fecha_factura", "fecha_factura"]);
    click(qa(el, "th").find((th) => th.textContent!.trim().startsWith("Actualizado"))!);
    expect(seen.at(-1)).toBe("actualizado_en");
  });

  it("reflects the sort it is given in aria-sort", async () => {
    const el = await mountTable({ rows: [trackingRow()], sort: { key: "fecha_factura", dir: -1 } });
    const state = (label: string) => attrOf(qa(el, "th").find((th) => th.textContent!.trim().startsWith(label))!, "aria-sort");
    expect(state("Fecha factura")).toBe("descending");
    expect(state("Actualizado")).toBe("none");
    Object.assign(el, { sort: { key: "fecha_factura", dir: 1 } });
    await settle(el);
    expect(state("Fecha factura")).toBe("ascending");
  });

  it("renders every row it gets, however many pages were loaded", async () => {
    const rows = Array.from({ length: 320 }, (_, i) => trackingRow({ factura: `F${i}`, cargue: "c", fecha_factura: "" }));
    const el = await mountTable({ rows, total: 320 });
    expect(bodyRows(el)).toHaveLength(320);
    expect(summary(el)).toBe("320 facturas");
  }, 30_000);

  it("offers Cargar más only while more rows are left, and emits cargar-mas", async () => {
    const none = await mountTable({ rows: [trackingRow()], total: 1 });
    expect(q(none, ".more")).toBeNull();
    const el = await mountTable({ rows: [trackingRow()], total: 500, hasMore: true });
    const seen: string[] = [];
    el.addEventListener("cargar-mas", () => seen.push("more"));
    const button = q(el, ".more tx-button")!;
    expect(button.textContent).toContain("Cargar más");
    click(button);
    expect(seen).toEqual(["more"]);
  });

  it("puts Cargar más in its loading state while the next page is loading", async () => {
    const el = await mountTable({ rows: [trackingRow()], total: 500, hasMore: true, loadingMore: true });
    const button = q(el, ".more tx-button")!;
    expect(button.hasAttribute("loading")).toBe(true);
  });

  it("keeps the DOM of a row when a new poll brings an equal list in another order", async () => {
    const el = await mountTable({ rows: [trackingRow({ factura: "A" }), trackingRow({ factura: "B" })] });
    const before = bodyRows(el).map((tr) => tr);
    Object.assign(el, { rows: [trackingRow({ factura: "B" }), trackingRow({ factura: "A" })] });
    await settle(el);
    const after = bodyRows(el);
    expect(after[0]).toBe(before[1]);
    expect(after[1]).toBe(before[0]);
  });

  it("emits the row for the detail button and for the invoice link", async () => {
    const r = trackingRow({ factura: "FVS7" });
    const el = await mountTable({ rows: [r] });
    const seen: Array<[string, unknown]> = [];
    el.addEventListener("ver-detalle", (e) => seen.push(["detalle", (e as CustomEvent).detail]));
    el.addEventListener("ver-portal", (e) => seen.push(["portal", (e as CustomEvent).detail]));
    click(q(el, "tx-button[title='Ver detalle de FVS7']"));
    click(q(el, ".invoice-link"));
    expect(seen).toEqual([["detalle", r], ["portal", r]]);
  });

  it("labels the portal status icon of each row", async () => {
    const el = await mountTable({
      rows: [
        trackingRow({ factura: "P0" }),
        trackingRow({ factura: "P1", portal_consultado: true, portal_final: true, portal_errores: 2 }),
        trackingRow({ factura: "P2", portal_consultado: true, portal_final: true }),
      ],
    });
    const labels = qa(el, ".portal-icon").map((i) => i.getAttribute("label"));
    expect(labels).toEqual(["Aún no consultado en el portal", "Con errores en el portal", "Validada sin advertencias"]);
    expect(qa(el, ".portal-icon").map((i) => i.getAttribute("name"))).toEqual(["clock", "warning-circle", "check-circle"]);
  });

  it("tints the portal icon with the tone of its state", async () => {
    const el = await mountTable({
      rows: [trackingRow({ factura: "P0" }), trackingRow({ factura: "P1", portal_consultado: true, portal_final: true, portal_errores: 2 })],
    });
    expect(qa(el, ".portal-icon").map((i) => i.getAttribute("data-tone"))).toEqual(["neutral", "error"]);
  });

  it("keeps the two-row header sticky inside a scrolling wrapper and the shared tracking-table class", async () => {
    const el = await mountTable({ rows: [trackingRow()] });
    const table = q(el, "table")!;
    expect(table.classList.contains("tracking-table")).toBe(true);
    expect(table.parentElement!.classList.contains("scroll")).toBe(true);
    expect(qa(el, "thead th").length).toBeGreaterThan(0);
    expect(qa(el, "thead tr")).toHaveLength(2);
  });
});
