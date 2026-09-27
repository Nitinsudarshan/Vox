/**
 * The readable message in whatever a failed `invoke()` or a thrown value
 * carries: a Tauri `CommandError` (`{ code, message }`), an `Error`, or a
 * plain string. `fallback` when none of those has anything to say.
 *
 * Replaces `err?.message || fallback`, which silently dropped the message
 * whenever an error arrived as a plain string.
 */
export function describeError(err: unknown, fallback = 'Something went wrong.'): string {
  if (typeof err === 'string' && err.trim()) return err;
  if (err !== null && typeof err === 'object' && 'message' in err) {
    const { message } = err as { message?: unknown };
    if (typeof message === 'string' && message.trim()) return message;
  }
  return fallback;
}
