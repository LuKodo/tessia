import { ApiError } from "./api-client";

/**
 * True when the request never got an HTTP answer: ApiError status 0 (the client wraps a
 * failed fetch) or a raw fetch TypeError. A browser cannot tell a stopped service from a
 * CORS block, so both look the same here.
 */
export function isNetworkFailure(error: unknown): boolean {
  return (error instanceof ApiError && error.status === 0) || error instanceof TypeError;
}

/**
 * Actionable Spanish hint for a network-level failure, "" for anything else (no error, HTTP
 * errors). Pure: the caller passes the configured API base and the page origin
 * (`window.location.origin`).
 */
export function connectivityHint(error: unknown, apiBase: string, origin: string): string {
  if (!isNetworkFailure(error)) return "";
  const pageOrigin = origin || "el origen de esta página";
  const base = apiBase.replace(/\/+$/, "") || pageOrigin;
  return (
    `El navegador no distingue un servicio detenido de un bloqueo CORS. ` +
    `Verifique que el servicio esté activo en ${base} y que se haya iniciado con ` +
    `"tessia serve --allowed-origin ${pageOrigin}" ` +
    `(o "serve.allowed_origins" en tessia.env). Origen de esta página: ${pageOrigin}.`
  );
}
