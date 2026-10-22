import { afterEach, describe, expect, it } from "vitest";
import "../../src/components/tx-app-dialogs";
import type { TxAppDialogs } from "../../src/components/tx-app-dialogs";
import type { TxModal } from "../../src/components/tx-modal";
import { dialogs } from "../../src/utils/dialogs";

async function mount() {
  const el = document.createElement("tx-app-dialogs") as TxAppDialogs;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

const q = (el: TxAppDialogs, selector: string) => el.shadowRoot!.querySelector<HTMLElement>(selector);
const modal = (el: TxAppDialogs, kind: "confirm" | "error") => q(el, `tx-modal[data-kind=${kind}]`) as TxModal;

async function settle(el: TxAppDialogs) {
  await el.updateComplete;
  await modal(el, "confirm").updateComplete;
  await modal(el, "error").updateComplete;
}

afterEach(() => {
  dialogs.answerConfirm(false);
  dialogs.dismissError();
  document.body.innerHTML = "";
});

describe("tx-app-dialogs", () => {
  it("renders nothing open while there is no dialog", async () => {
    const el = await mount();
    expect(modal(el, "confirm").open).toBe(false);
    expect(modal(el, "error").open).toBe(false);
    expect(q(el, "[data-testid=confirm-dialog]")).toBeNull();
  });

  it("shows a confirmation and resolves it true from the accept button", async () => {
    const el = await mount();
    const answer = dialogs.confirmAction("¿Cancelar el proceso en curso?");
    await settle(el);
    expect(modal(el, "confirm").open).toBe(true);
    expect(q(el, "[data-testid=confirm-dialog]")!.textContent).toContain("¿Cancelar el proceso en curso?");
    expect(q(el, "[data-testid=confirm-dialog] h2")!.textContent).toBe("¿Confirmas esta acción?");
    expect(q(el, "[data-testid=confirm-accept]")!.textContent!.trim()).toBe("Sí, continuar");
    q(el, "[data-testid=confirm-accept]")!.click();
    expect(await answer).toBe(true);
    await settle(el);
    expect(modal(el, "confirm").open).toBe(false);
  });

  it("declines from the Cancelar button", async () => {
    const el = await mount();
    const answer = dialogs.confirmAction("¿Seguro?", { title: "Reintentar", confirmText: "Sí, reintentar" });
    await settle(el);
    expect(q(el, "[data-testid=confirm-accept]")!.textContent!.trim()).toBe("Sí, reintentar");
    expect(q(el, "[data-testid=confirm-cancel]")!.textContent!.trim()).toBe("Cancelar");
    q(el, "[data-testid=confirm-cancel]")!.click();
    expect(await answer).toBe(false);
  });

  it("keeps the confirmation open on Escape and backdrop: only its buttons answer it", async () => {
    const el = await mount();
    const answer = dialogs.confirmAction("¿Seguro?");
    await settle(el);
    const native = modal(el, "confirm").shadowRoot!.querySelector("dialog")!;
    const cancel = new Event("cancel", { cancelable: true });
    native.dispatchEvent(cancel);
    native.click();
    await settle(el);
    expect(cancel.defaultPrevented).toBe(true);
    expect(dialogs.confirm).not.toBeNull();
    expect(modal(el, "confirm").open).toBe(true);
    dialogs.answerConfirm(false);
    expect(await answer).toBe(false);
  });

  it("shows errors with an OK button that dismisses them", async () => {
    const el = await mount();
    dialogs.notifyError("ya hay un proceso en curso");
    await settle(el);
    expect(modal(el, "error").open).toBe(true);
    expect(q(el, "[data-testid=error-dialog]")!.textContent).toContain("ya hay un proceso en curso");
    expect(q(el, "[data-testid=error-dialog] h2")!.textContent).toBe("No se pudo completar");
    q(el, "[data-testid=error-ok]")!.click();
    await settle(el);
    expect(dialogs.error).toBeNull();
    expect(modal(el, "error").open).toBe(false);
  });

  it("dismisses the error dialog with Escape", async () => {
    const el = await mount();
    dialogs.notifyError("falló la descarga");
    await settle(el);
    modal(el, "error").shadowRoot!.querySelector("dialog")!.dispatchEvent(new Event("cancel", { cancelable: true }));
    await settle(el);
    expect(dialogs.error).toBeNull();
    expect(modal(el, "error").open).toBe(false);
  });

  it("uses distinct warning and error icons and labels the dialogs", async () => {
    const el = await mount();
    void dialogs.confirmAction("x");
    dialogs.notifyError("y");
    await settle(el);
    expect(modal(el, "confirm").label).toBe("¿Confirmas esta acción?");
    expect(modal(el, "error").label).toBe("No se pudo completar");
    expect(q(el, "[data-testid=confirm-dialog] svg.warning")).not.toBeNull();
    expect(q(el, "[data-testid=error-dialog] svg.error")).not.toBeNull();
  });
});
