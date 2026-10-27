import { describe, expect, it, vi } from "vitest";
import type { ReactiveController, ReactiveControllerHost } from "lit";
import { ParametersFormController } from "../../src/controllers/parameters-form-controller";
import { ApiError } from "../../src/utils/api-client";
import type { PreparationParametersResponse } from "../../src/types/api";

const stored: PreparationParametersResponse = {
  max_invoice_amount: "150000",
  automatic_subplans: ["evento pós", "Plan Especial"],
  wave_size: 10,
  derechos_unavailable_percent: 30,
  derechos_min_sample: 5,
  cuv_token_failure_percent: 20,
  cuv_min_sample: 6,
  configured: true,
};

/** Minimal host: records the controller and counts update requests. */
function fakeHost() {
  const controllers: ReactiveController[] = [];
  const host = {
    addController: (c: ReactiveController) => controllers.push(c),
    removeController: () => {},
    requestUpdate: vi.fn(),
    updateComplete: Promise.resolve(true),
  } as unknown as ReactiveControllerHost & { requestUpdate: ReturnType<typeof vi.fn> };
  return { host, connect: () => controllers.forEach((c) => c.hostConnected?.()), disconnect: () => controllers.forEach((c) => c.hostDisconnected?.()) };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

function mountForm(api: ConstructorParameters<typeof ParametersFormController>[1]) {
  const { host, connect, disconnect } = fakeHost();
  const form = new ParametersFormController(host, api);
  connect();
  return { form, host, disconnect };
}

describe("ParametersFormController", () => {
  it("loads the stored parameters and selects the matching subplans", async () => {
    const { form } = mountForm({ parameters: vi.fn(async () => stored), saveParameters: vi.fn() });
    expect(form.loading).toBe(true);
    await flush();
    expect(form.loading).toBe(false);
    expect(form.amount).toBe("150000");
    expect(form.waveSize).toBe(10);
    expect(form.options).toEqual([
      "EVENTO POS",
      "EVENTO NO PBS",
      "TRANSFERENCIA DE PENDIENTES CON AUTORIZACION",
      "EVENTO SAS SIN AUTORIZACION",
      "Plan Especial",
    ]);
    expect(form.selected).toEqual(["EVENTO POS", "Plan Especial"]);
    expect(form.configured).toBe(true);
    expect(form.error).toBe("");
  });

  it("keeps the defaults and reports the failure when loading fails", async () => {
    const { form } = mountForm({
      parameters: vi.fn(async () => {
        throw new ApiError(503, "base de datos no disponible");
      }),
      saveParameters: vi.fn(),
    });
    await flush();
    expect(form.loading).toBe(false);
    expect(form.error).toBe("base de datos no disponible");
    expect(form.configured).toBe(false);
    expect(form.waveSize).toBe(20);
  });

  it("asks the host to re-render when the state changes", async () => {
    const { form, host } = mountForm({ parameters: vi.fn(async () => stored), saveParameters: vi.fn() });
    await flush();
    const before = host.requestUpdate.mock.calls.length;
    expect(before).toBeGreaterThan(0);
    form.set("waveSize", "30");
    expect(form.waveSize).toBe("30");
    expect(host.requestUpdate.mock.calls.length).toBe(before + 1);
  });

  it("saves the form in the order of the options and reports success", async () => {
    const saveParameters = vi.fn(async () => ({ ...stored, configured: true }));
    const { form } = mountForm({ parameters: vi.fn(async () => ({ ...stored, configured: false })), saveParameters });
    await flush();
    expect(form.configured).toBe(false);

    form.set("selected", ["Plan Especial", "EVENTO POS"]);
    form.set("waveSize", "25");
    await form.save();

    expect(saveParameters).toHaveBeenCalledWith({
      max_invoice_amount: "150000",
      automatic_subplans: ["EVENTO POS", "Plan Especial"],
      wave_size: 25,
      derechos_unavailable_percent: 30,
      derechos_min_sample: 5,
      cuv_token_failure_percent: 20,
      cuv_min_sample: 6,
    });
    expect(form.message).toBe("Global preparation parameters saved.");
    expect(form.configured).toBe(true);
    expect(form.saving).toBe(false);
  });

  it("is saving while the request is in flight", async () => {
    let finish!: (value: PreparationParametersResponse) => void;
    const saveParameters = vi.fn(() => new Promise<PreparationParametersResponse>((resolve) => (finish = resolve)));
    const { form } = mountForm({ parameters: vi.fn(async () => stored), saveParameters });
    await flush();
    const pending = form.save();
    expect(form.saving).toBe(true);
    finish(stored);
    await pending;
    expect(form.saving).toBe(false);
  });

  it("does not call the API when the form is invalid", async () => {
    const saveParameters = vi.fn();
    const { form } = mountForm({ parameters: vi.fn(async () => stored), saveParameters });
    await flush();
    form.set("amount", "0");
    await form.save();
    expect(saveParameters).not.toHaveBeenCalled();
    expect(form.error).toMatch(/positive maximum amount/i);
    expect(form.message).toBe("");
  });

  it("shows the server error when saving fails and clears it on the next attempt", async () => {
    const saveParameters = vi
      .fn()
      .mockRejectedValueOnce(new ApiError(400, "campo desconocido"))
      .mockResolvedValueOnce(stored);
    const { form } = mountForm({ parameters: vi.fn(async () => stored), saveParameters });
    await flush();
    await form.save();
    expect(form.error).toBe("campo desconocido");
    expect(form.saving).toBe(false);
    await form.save();
    expect(form.error).toBe("");
    expect(form.message).toBe("Global preparation parameters saved.");
  });

  it("ignores a load that settles after the host disconnected", async () => {
    let finish!: (value: PreparationParametersResponse) => void;
    const { form, disconnect } = mountForm({
      parameters: vi.fn(() => new Promise<PreparationParametersResponse>((resolve) => (finish = resolve))),
      saveParameters: vi.fn(),
    });
    disconnect();
    finish(stored);
    await flush();
    expect(form.amount).toBe("");
    expect(form.loading).toBe(true);
  });
});
