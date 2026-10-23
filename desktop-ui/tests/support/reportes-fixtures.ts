import type { Cargue, RadicadaRow, RadicadasReport } from "../../src/types/api";

export const cargue = (id: string, over: Partial<Cargue> = {}): Cargue => ({
  id,
  nombre: `Convenio ${id}`,
  facturas: 10,
  excluidas: 0,
  revision_manual: 0,
  modificado_en: "2026-09-16T10:00:00Z",
  estado: "COMPLETADO",
  ...over,
});

export const radicada = (over: Partial<RadicadaRow> = {}): RadicadaRow => ({
  cargue: "A",
  cargue_nombre: "Convenio A",
  factura: "FVSA1",
  fecha_radicacion: "2026-09-12T18:30:00Z",
  estado: "TERMINADO",
  fecha_factura: "2026-09-02",
  ssc: "11",
  autorizacion: "AUT1",
  valor: "100",
  observaciones: "radicada",
  ...over,
});

export const reporte = (filas: RadicadaRow[], over: Partial<RadicadasReport> = {}): RadicadasReport => ({
  generado_en: "2026-09-20T09:00:00Z",
  total: filas.length,
  truncado: false,
  filas,
  ...over,
});
