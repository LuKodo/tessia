import { apiBase } from "./config";
import { createApiClient } from "./utils/api-client";
import { createTessiaApi } from "./utils/tessia-api";

/** Low-level client (getJSON/requestJSON/sendJSON/upload) bound to the configured API base. */
export const apiClient = createApiClient({ baseUrl: apiBase });

/** Typed method per route of `tessia serve`, e.g. `await api.cargues()`. */
export const api = createTessiaApi(apiClient);
