import type { ReactiveController, ReactiveControllerHost } from "lit";
import { dialogs as shared, type ConfirmOptions, type ConfirmRequest, type Dialogs, type ErrorNotice } from "../utils/dialogs";

/**
 * Binds an element to the dialogs service: the host re-renders whenever the confirm or error state
 * changes (subscribed while connected). Replaces the old `useDialogs()` composable.
 */
export class DialogsController implements ReactiveController {
  private unsubscribe: (() => void) | undefined;

  constructor(
    private host: ReactiveControllerHost,
    private service: Dialogs = shared,
  ) {
    host.addController(this);
  }

  get confirm(): ConfirmRequest | null {
    return this.service.confirm;
  }

  get error(): ErrorNotice | null {
    return this.service.error;
  }

  confirmAction(text: string, options?: ConfirmOptions): Promise<boolean> {
    return this.service.confirmAction(text, options);
  }

  answerConfirm(accepted: boolean): void {
    this.service.answerConfirm(accepted);
  }

  notifyError(message: string): void {
    this.service.notifyError(message);
  }

  dismissError(): void {
    this.service.dismissError();
  }

  hostConnected(): void {
    this.unsubscribe = this.service.subscribe(() => this.host.requestUpdate());
    this.host.requestUpdate(); // a dialog may already be pending
  }

  hostDisconnected(): void {
    this.unsubscribe?.();
    this.unsubscribe = undefined;
  }
}
