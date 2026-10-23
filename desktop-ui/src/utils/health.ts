import type { ServiceHealth } from "../types/api";
import { connectivityHint } from "./connectivity-hint";

export type HealthState = "ok" | "degraded" | "unreachable" | "unknown";

/** Where the UI is served from and where it calls, used to build the connectivity hint. */
export interface ConnectivityContext {
  apiBase: string;
  origin: string;
}

export interface HealthSummary {
  state: HealthState;
  label: string;
  detail: string;
}

/**
 * Collapses GET /api/health into one indicator. A failed request wins over stale
 * data (the service is unreachable); otherwise any unavailable adjacent service is
 * "degraded".
 */
export function summarizeHealth(
  services: ServiceHealth[] | null,
  error: unknown,
  context?: ConnectivityContext,
): HealthSummary {
  if (error) {
    const raw = error instanceof Error ? error.message : String(error);
    const hint = context ? connectivityHint(error, context.apiBase, context.origin) : "";
    return { state: "unreachable", label: "Sin conexión", detail: hint ? `${raw}\n${hint}` : raw };
  }
  if (!services || services.length === 0) return { state: "unknown", label: "Verificando…", detail: "" };
  const down = services.filter((s) => !s.disponible).map((s) => s.nombre);
  if (down.length === 0) return { state: "ok", label: "Servicio activo", detail: "Todos los servicios disponibles" };
  return { state: "degraded", label: "Degradado", detail: `No disponible: ${down.join(", ")}` };
}
