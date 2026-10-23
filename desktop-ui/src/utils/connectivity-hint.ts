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
 * (`window.location.origin`). In the desktop app the API is served in-process on the app's
 * own origin, so a network failure means the API is unreachable from the shell: restarting
 * the app is the recovery path (there is no CORS or `--allowed-origin` in the desktop build).
 */
export function connectivityHint(error: unknown, apiBase: string, origin: string): string {
  if (!isNetworkFailure(error)) return "";
  const pageOrigin = origin || "el origen de esta página";
  const base = apiBase.replace(/\/+$/, "") || pageOrigin;
  return (
    `La API no respondió en ${base}. En la aplicación de escritorio la API se sirve dentro del ` +
    `mismo proceso, en el origen de esta ventana: reinicie la aplicación. ` +
    `Origen de esta página: ${pageOrigin}.`
  );
}
