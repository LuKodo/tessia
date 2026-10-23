export interface ConfirmOptions {
  title?: string;
  confirmText?: string;
}

export interface ConfirmRequest {
  text: string;
  title: string;
  confirmText: string;
}

export interface ErrorNotice {
  title: string;
  text: string;
}

export type DialogsListener = () => void;

/**
 * State of the confirm and error dialogs (port of dialogs.js, which wrapped SweetAlert2), free of
 * any UI framework. `confirmAction` replaces window.confirm for live or destructive actions and
 * resolves true only on an explicit accept; `notifyError` replaces window.alert. `<tx-app-dialogs>`
 * renders this state through a `DialogsController`; anything else can `subscribe`.
 */
export function createDialogs() {
  let confirm: ConfirmRequest | null = null;
  let error: ErrorNotice | null = null;
  let settle: ((accepted: boolean) => void) | null = null;
  const listeners = new Set<DialogsListener>();

  const emit = () => {
    for (const listener of [...listeners]) listener();
  };

  /** Clears the open confirmation (if any) and resolves it; returns whether there was one. */
  function resolveConfirm(accepted: boolean): boolean {
    const done = settle;
    const wasOpen = confirm !== null;
    settle = null;
    confirm = null;
    done?.(accepted);
    return wasOpen;
  }

  function answerConfirm(accepted: boolean): void {
    if (resolveConfirm(accepted)) emit();
  }

  function confirmAction(text: string, options: ConfirmOptions = {}): Promise<boolean> {
    // One confirmation at a time: a newer request declines the one still open.
    resolveConfirm(false);
    confirm = {
      text,
      title: options.title || "¿Confirmas esta acción?",
      confirmText: options.confirmText || "Sí, continuar",
    };
    const answer = new Promise<boolean>((resolve) => {
      settle = resolve;
    });
    emit();
    return answer;
  }

  function notifyError(message: string): void {
    error = { title: "No se pudo completar", text: message };
    emit();
  }

  function dismissError(): void {
    if (error === null) return;
    error = null;
    emit();
  }

  /** Calls `listener` after every state change; returns the unsubscribe function. */
  function subscribe(listener: DialogsListener): () => void {
    listeners.add(listener);
    return () => {
      listeners.delete(listener);
    };
  }

  return {
    get confirm() {
      return confirm;
    },
    get error() {
      return error;
    },
    confirmAction,
    answerConfirm,
    notifyError,
    dismissError,
    subscribe,
  };
}

export type Dialogs = ReturnType<typeof createDialogs>;

/** One instance for the whole SPA, rendered by `<tx-app-dialogs>`. */
export const dialogs: Dialogs = createDialogs();
