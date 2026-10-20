// People-facing text for WebAuthn ceremony failures.

/** A ceremony cancelled by us (a new one started, the page left): never worth a message. */
export function isAborted(e: unknown): boolean {
  const err = e as { name?: string; code?: string } | null;
  return err?.code === 'ERROR_CEREMONY_ABORTED' || err?.name === 'AbortError';
}

/**
 * A message for an error thrown by `startAuthentication`/`startRegistration`,
 * or null when it isn't a recognisable WebAuthn failure (describe it generically).
 */
export function ceremonyError(e: unknown): string | null {
  switch ((e as { name?: string } | null)?.name) {
    case 'NotAllowedError':
      return 'The passkey prompt was closed or timed out. Try again when you are ready.';
    case 'InvalidStateError':
      return 'This device already has a passkey for these books. Sign in with it instead.';
    case 'SecurityError':
      return "Passkeys need a secure address (https, or localhost) that matches the server's configured name.";
    case 'NotSupportedError':
      return "This browser or device can't create a passkey of the kind the server asks for.";
    default:
      return null;
  }
}
