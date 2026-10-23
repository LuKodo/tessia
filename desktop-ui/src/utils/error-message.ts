/** Text to show for a caught value: the message of an Error, otherwise its string form. */
export function errorMessage(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}
