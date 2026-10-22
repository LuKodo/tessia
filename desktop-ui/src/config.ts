// Runtime configuration read from Vite env (baked in at build time, SPA).

export const DEFAULT_API_BASE = "http://localhost:8090";

/** `VITE_API_BASE`: unset falls back to the local service, an explicit empty string means same origin. */
export function resolveApiBase(value: string | undefined): string {
  return value ?? DEFAULT_API_BASE;
}

// Written inline (not through resolveApiBase) so Vite folds `"" ?? default` at build time and a
// same-origin bundle (`VITE_API_BASE=`) carries no absolute API URL.
export const apiBase: string = import.meta.env.VITE_API_BASE ?? DEFAULT_API_BASE;
