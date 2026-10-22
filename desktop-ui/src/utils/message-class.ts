export type MessageKind = "" | "ok" | "error";

/** Inline status line under a form: what it says and how it should look. */
export interface UiMessage {
  text: string;
  kind: MessageKind;
}

/** Text class of an inline message (muted while it is only progress). */
export function messageClass(kind: MessageKind): string {
  return kind === "error" ? "text-error" : kind === "ok" ? "text-success" : "text-muted";
}
