import { afterEach, describe, expect, it } from "vitest";
import { LitElement, html } from "lit";
import { DialogsController } from "../../src/controllers/dialogs-controller";
import { createDialogs } from "../../src/utils/dialogs";

const dialogs = createDialogs();

class DialogsHost extends LitElement {
  d = new DialogsController(this, dialogs);
  renders = 0;
  render() {
    this.renders++;
    return html`<span id="c">${this.d.confirm?.text ?? ""}</span><span id="e">${this.d.error?.text ?? ""}</span>`;
  }
}
customElements.define("dialogs-host", DialogsHost);

async function mount() {
  const el = document.createElement("dialogs-host") as DialogsHost;
  document.body.append(el);
  await el.updateComplete;
  return el;
}

afterEach(() => {
  dialogs.dismissError();
  dialogs.answerConfirm(false);
  document.body.innerHTML = "";
});

describe("DialogsController", () => {
  it("re-renders the host when a confirmation opens and closes", async () => {
    const el = await mount();
    const answer = el.d.confirmAction("¿Seguro?");
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("#c")!.textContent).toBe("¿Seguro?");
    el.d.answerConfirm(true);
    await el.updateComplete;
    expect(await answer).toBe(true);
    expect(el.shadowRoot!.querySelector("#c")!.textContent).toBe("");
  });

  it("re-renders the host for errors", async () => {
    const el = await mount();
    el.d.notifyError("falló");
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("#e")!.textContent).toBe("falló");
    el.d.dismissError();
    await el.updateComplete;
    expect(el.shadowRoot!.querySelector("#e")!.textContent).toBe("");
  });

  it("shows a dialog that was already pending when the host connected", async () => {
    void dialogs.confirmAction("pendiente");
    const el = await mount();
    expect(el.shadowRoot!.querySelector("#c")!.textContent).toBe("pendiente");
  });

  it("stops listening once the host disconnects", async () => {
    const el = await mount();
    el.remove();
    const before = el.renders;
    dialogs.notifyError("x");
    await Promise.resolve();
    expect(el.renders).toBe(before);
  });
});
