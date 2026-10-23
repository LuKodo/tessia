import { vi } from "vitest";
import { LitElement } from "lit";
import { HallazgosController, type HallazgoRow, type HallazgosApi } from "../../src/controllers/hallazgos-controller";
import type { Hallazgo } from "../../src/types/api";

export const catalog: Hallazgo[] = [
  { codigo: "RVC012", detalle: "Valor no coincide", genera_glosa: false, devolucion: false, descripcion: "" },
  { codigo: "RVC040", detalle: "CUPS inválido", genera_glosa: true, devolucion: false, descripcion: "Se glosa" },
];

/** A manually settled promise, to hold a request in flight. */
export function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

/** A fake of the two API methods the Hallazgos page uses; override what a test cares about. */
export function fakeHallazgosApi(overrides: Partial<HallazgosApi> = {}) {
  const base = {
    hallazgos: vi.fn(async () => ({ hallazgos: catalog, total: catalog.length })),
    updateHallazgo: vi.fn(),
  };
  return { ...base, ...overrides } as HallazgosApi & { hallazgos: ReturnType<typeof vi.fn>; updateHallazgo: ReturnType<typeof vi.fn> };
}

/** A table row in its idle state, with the saved snapshot equal to the draft. */
export function row(over: Partial<HallazgoRow> = {}): HallazgoRow {
  return {
    codigo: "H01",
    detalle: "Detalle uno",
    genera_glosa: false,
    devolucion: false,
    descripcion: "",
    saved: { genera_glosa: false, devolucion: false, descripcion: "" },
    status: "idle",
    message: "",
    inflight: false,
    queued: false,
    ...over,
  };
}

/** Minimal element that owns a HallazgosController, to exercise it with a real host lifecycle. */
export class HallazgosTestHost extends LitElement {
  api!: HallazgosApi;
  renders = 0;
  state = new HallazgosController(this, () => this.api);
  protected render() {
    this.renders++;
    return null;
  }
}
if (!customElements.get("hallazgos-test-host")) customElements.define("hallazgos-test-host", HallazgosTestHost);

export async function mountHost(api: HallazgosApi) {
  const host = document.createElement("hallazgos-test-host") as HallazgosTestHost;
  host.api = api;
  document.body.append(host);
  await host.updateComplete;
  return host;
}
