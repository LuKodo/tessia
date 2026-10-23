export type StatusColor = "warning" | "success" | "error" | "neutral";

/**
 * Maps a raw backend status string (cargue estado, phase status, job history kind) to a
 * Color name of a status text, for the badge `color` attribute.
 */
export function statusColor(text: string | null | undefined): StatusColor {
  const v = (text || "").toLowerCase().replace(/[\s_]+/g, "");
  if (["procesando", "cargue", "enproceso", "iniciado", "sinpreparar"].includes(v)) return "warning";
  if (["completado", "terminado"].includes(v)) return "success";
  if (["conincidencias", "rechazado", "vencido", "notaajuste", "terminadoconerror"].includes(v)) return "error";
  return "neutral";
}
