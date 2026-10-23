import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ApiError } from "../../src/utils/api-client";
import type { Hallazgo } from "../../src/types/api";
import { catalog, deferred, fakeHallazgosApi, mountHost } from "../support/hallazgos-fixtures";

// Ported from ui/tests/unit/use-hallazgos.test.ts (same scenarios) against the Lit controller.
const flush = () => vi.advanceTimersByTimeAsync(0);

describe("HallazgosController", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => {
    vi.useRealTimers();
    document.body.innerHTML = "";
  });

  it("loads the catalog into idle rows and filters by the query", async () => {
    const pending = deferred<{ hallazgos: Hallazgo[]; total: number }>();
    const host = await mountHost(fakeHallazgosApi({ hallazgos: vi.fn(() => pending.promise) }));
    const state = host.state;
    expect(state.loading).toBe(true);
    pending.resolve({ hallazgos: catalog, total: 2 });
    await flush();
    expect(state.loading).toBe(false);
    expect(state.rows.map((r) => r.codigo)).toEqual(["RVC012", "RVC040"]);
    expect(state.rows[1]).toMatchObject({ genera_glosa: true, status: "idle" });
    state.query = "cups";
    expect(state.shown.map((r) => r.codigo)).toEqual(["RVC040"]);
    expect(state.summary).toBe("1 de 2 hallazgos");
  });

  it("re-renders the host when the state changes", async () => {
    const host = await mountHost(fakeHallazgosApi());
    await flush();
    await host.updateComplete;
    const before = host.renders;
    host.state.query = "x";
    await host.updateComplete;
    expect(host.renders).toBeGreaterThan(before);
  });

  it("reports a load failure and retries with load()", async () => {
    const hallazgos = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(503, "sin base de datos"))
      .mockResolvedValueOnce({ hallazgos: catalog, total: 2 });
    const host = await mountHost(fakeHallazgosApi({ hallazgos }));
    await flush();
    expect(host.state.loadError).toBe("sin base de datos");
    await host.state.load();
    expect(host.state.loadError).toBe("");
    expect(host.state.rows).toHaveLength(2);
  });

  it("saves a toggled checkbox optimistically, then shows Guardado and returns to idle", async () => {
    const updateHallazgo = vi.fn(async (codigo: string, body: { genera_glosa: boolean }) => ({
      ...catalog[0]!,
      codigo,
      genera_glosa: body.genera_glosa,
    }));
    const { state } = await mountHost(fakeHallazgosApi({ updateHallazgo }));
    await flush();

    state.onToggle("RVC012", true);
    expect(state.rows[0]).toMatchObject({ genera_glosa: true, status: "saving" });
    await flush();
    expect(updateHallazgo).toHaveBeenCalledWith("RVC012", { genera_glosa: true, devolucion: false, descripcion: "" });
    expect(state.rows[0]).toMatchObject({ genera_glosa: true, status: "saved" });
    expect(state.rows[0]!.saved.genera_glosa).toBe(true);

    await vi.advanceTimersByTimeAsync(2000);
    expect(state.rows[0]!.status).toBe("idle");
  });

  it("saves a toggled devolucion checkbox", async () => {
    const updateHallazgo = vi.fn(async (codigo: string, body: object) => ({ ...catalog[0]!, codigo, ...body }));
    const { state } = await mountHost(fakeHallazgosApi({ updateHallazgo }));
    await flush();
    state.onToggleDevolucion("RVC012", true);
    await flush();
    expect(updateHallazgo).toHaveBeenCalledWith("RVC012", { genera_glosa: false, devolucion: true, descripcion: "" });
    expect(state.rows[0]).toMatchObject({ devolucion: true, status: "saved" });
  });

  it("rolls the checkbox back and keeps the typed text when the PUT fails", async () => {
    const updateHallazgo = vi.fn().mockRejectedValue(new ApiError(500, "no se pudo guardar"));
    const { state } = await mountHost(fakeHallazgosApi({ updateHallazgo }));
    await flush();

    state.onEdit("RVC012", "nota");
    state.onToggle("RVC012", true);
    await flush();
    expect(state.rows[0]).toMatchObject({
      genera_glosa: false,
      descripcion: "nota",
      status: "error",
      message: "no se pudo guardar",
    });
  });

  it("sends nothing when nothing changed, and rejects an over-long description locally", async () => {
    const updateHallazgo = vi.fn();
    const { state } = await mountHost(fakeHallazgosApi({ updateHallazgo }));
    await flush();

    state.onCommit("RVC012");
    expect(updateHallazgo).not.toHaveBeenCalled();
    expect(state.rows[0]!.status).toBe("idle");

    state.onEdit("RVC012", "a".repeat(2001));
    state.onCommit("RVC012");
    expect(updateHallazgo).not.toHaveBeenCalled();
    expect(state.rows[0]).toMatchObject({ status: "error", message: "La descripción no puede superar 2000 caracteres." });
  });

  it("serializes the PUTs of one row and sends the latest draft afterwards", async () => {
    const first = deferred<Hallazgo>();
    const updateHallazgo = vi
      .fn()
      .mockImplementationOnce(() => first.promise)
      .mockImplementation(async (codigo: string, body: object) => ({ ...catalog[0]!, codigo, ...body }));
    const { state } = await mountHost(fakeHallazgosApi({ updateHallazgo }));
    await flush();

    state.onToggle("RVC012", true);
    state.onEdit("RVC012", "motivo");
    state.onCommit("RVC012");
    expect(updateHallazgo).toHaveBeenCalledTimes(1);

    first.resolve({ ...catalog[0]!, genera_glosa: true });
    await flush();
    expect(updateHallazgo).toHaveBeenCalledTimes(2);
    expect(updateHallazgo).toHaveBeenLastCalledWith("RVC012", { genera_glosa: true, devolucion: false, descripcion: "motivo" });
    expect(state.rows[0]).toMatchObject({ descripcion: "motivo", status: "saved" });
  });

  it("ignores the answer of a save that settles after the host disconnects", async () => {
    const pending = deferred<Hallazgo>();
    const host = await mountHost(fakeHallazgosApi({ updateHallazgo: vi.fn(() => pending.promise) }));
    await flush();
    host.state.onToggle("RVC012", true);
    host.remove();
    pending.resolve({ ...catalog[0]!, genera_glosa: true });
    await flush();
    expect(host.state.rows[0]!.status).toBe("saving");
  });

  it("ignores a catalog that arrives after the host disconnects", async () => {
    const pending = deferred<{ hallazgos: Hallazgo[]; total: number }>();
    const host = await mountHost(fakeHallazgosApi({ hallazgos: vi.fn(() => pending.promise) }));
    host.remove();
    pending.resolve({ hallazgos: catalog, total: 2 });
    await flush();
    expect(host.state.rows).toEqual([]);
  });
});
