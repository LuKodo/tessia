import type { InvoiceTracking, PortalView, TrackingRow } from "../../src/types/api";

/**
 * A complete tracking row; override only what a test cares about. It may carry the detail fields
 * (`estado_detalle`, `lineas_convenio`) that only GET /api/facturas/{f}/seguimiento serves.
 */
export function trackingRow(over: Partial<InvoiceTracking> = {}): InvoiceTracking {
  return {
    factura: "FVS1001",
    resultado: "preparada",
    cargue: "c1",
    cargue_nombre: "Convenio Uno",
    artefactos: ["ok", "ok", "ok", "ok"],
    radicacion: ["pendiente", "pendiente", "pendiente", "pendiente"],
    portal_consultado: false,
    pipeline: "no_radicado",
    alertas: [],
    fecha_factura: "2026-10-05",
    ...over,
  };
}

/** The list row the table gets: the same row without the fields the slim list payload omits. */
export function listRow(row: InvoiceTracking): TrackingRow {
  const { estado_detalle: _estadoDetalle, lineas_convenio: _lineas, ...slim } = row;
  return slim;
}

/** A portal payload the way GET /api/facturas/{f}/portal answers it (every string present). */
export function portalView(over: Partial<PortalView> = {}): PortalView {
  return {
    factura: "FVS1001",
    consultado: true,
    final: true,
    estado: "VALIDADO",
    remision: "",
    fecha_cargue: "",
    fecha_cargue_texto: "",
    usuario: "",
    servicios: "",
    modalidad: "",
    nombre_archivo: "",
    id_archivo: "",
    id_cargue: "",
    id_evento: "",
    tipo_evento: "",
    tipo_error: "",
    codigo_error: "",
    devuelto: false,
    fecha_expedicion: "",
    receipt_name: "",
    documento_soporte: null,
    advertencias: 0,
    errores: 0,
    hallazgos: [],
    consultas: 1,
    consultado_en: "",
    origen: "base de datos",
    mensaje: "",
    error: "",
    ...over,
  };
}

/** The tracking rows of the dashboard tests share one cargue unless stated otherwise. */
export const rowsOf = (...facturas: string[]): TrackingRow[] => facturas.map((factura) => trackingRow({ factura }));
