import { describe, expect, it, vi } from "vitest";
import { createDialogs, dialogs as shared } from "../../src/utils/dialogs";

describe("createDialogs", () => {
  it("resolves confirmAction with true only when the operator accepts", async () => {
    const dialogs = createDialogs();
    const accepted = dialogs.confirmAction("¿Seguro?");
    expect(dialogs.confirm).toEqual({ text: "¿Seguro?", title: "¿Confirmas esta acción?", confirmText: "Sí, continuar" });
    dialogs.answerConfirm(true);
    expect(await accepted).toBe(true);
    expect(dialogs.confirm).toBeNull();

    const declined = dialogs.confirmAction("¿Seguro?");
    dialogs.answerConfirm(false);
    expect(await declined).toBe(false);
  });

  it("uses the title and confirm text of the options", () => {
    const dialogs = createDialogs();
    void dialogs.confirmAction("x", { title: "Reintentar factura", confirmText: "Sí, reintentar" });
    expect(dialogs.confirm).toMatchObject({ title: "Reintentar factura", confirmText: "Sí, reintentar" });
  });

  it("declines a pending confirmation when another one replaces it", async () => {
    const dialogs = createDialogs();
    const first = dialogs.confirmAction("primera");
    const second = dialogs.confirmAction("segunda");
    expect(await first).toBe(false);
    expect(dialogs.confirm?.text).toBe("segunda");
    dialogs.answerConfirm(true);
    expect(await second).toBe(true);
  });

  it("ignores an answer when nothing is pending", () => {
    const dialogs = createDialogs();
    expect(() => dialogs.answerConfirm(true)).not.toThrow();
  });

  it("shows an error message until it is dismissed", () => {
    const dialogs = createDialogs();
    dialogs.notifyError("ya hay un proceso en curso");
    expect(dialogs.error).toEqual({ title: "No se pudo completar", text: "ya hay un proceso en curso" });
    dialogs.dismissError();
    expect(dialogs.error).toBeNull();
  });

  it("notifies subscribers on every state change and stops after unsubscribe", async () => {
    const dialogs = createDialogs();
    const listener = vi.fn();
    const unsubscribe = dialogs.subscribe(listener);
    const answer = dialogs.confirmAction("x");
    expect(listener).toHaveBeenCalledTimes(1);
    dialogs.answerConfirm(true);
    await answer;
    expect(listener).toHaveBeenCalledTimes(2);
    dialogs.notifyError("e");
    dialogs.dismissError();
    expect(listener).toHaveBeenCalledTimes(4);
    unsubscribe();
    dialogs.notifyError("e");
    expect(listener).toHaveBeenCalledTimes(4);
  });

  it("does not notify when an answer or dismissal has nothing to clear", () => {
    const dialogs = createDialogs();
    const listener = vi.fn();
    dialogs.subscribe(listener);
    dialogs.answerConfirm(true);
    dialogs.dismissError();
    expect(listener).not.toHaveBeenCalled();
  });

  it("a replaced confirmation notifies once with the new state already in place", () => {
    const dialogs = createDialogs();
    void dialogs.confirmAction("primera");
    const seen: Array<string | undefined> = [];
    dialogs.subscribe(() => seen.push(dialogs.confirm?.text));
    void dialogs.confirmAction("segunda");
    expect(seen.at(-1)).toBe("segunda");
  });

  it("exposes one shared instance for the whole app", () => {
    expect(shared.confirm).toBeNull();
    expect(typeof shared.confirmAction).toBe("function");
  });
});
