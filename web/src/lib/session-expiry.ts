// Client-side coordination for an expired browser session.
//
// The HttpOnly session cookie can outlive the ID token inside it, so a request
// may fail with 401 while the cookie still exists. Every protected call funnels
// its auth status through `noteAuthStatus`; the first 401 marks the session
// expired and notifies the shell exactly once. Concurrent failures therefore
// produce one transition, not a storm of reloads or sign-out calls.
//
// Only 401 is an authentication result. 403 (authorization), 5xx (service), and
// network errors (status 0) never expire the session.

export const SESSION_EXPIRED_MESSAGE = "Session expired. Sign in again.";

type Listener = () => void;

let expired = false;
const listeners = new Set<Listener>();

/** Reports whether the current page has already observed an expired session. */
export function sessionIsExpired(): boolean {
  return expired;
}

/**
 * Marks the session expired and notifies subscribers once. Repeated calls while
 * already expired are no-ops, which collapses concurrent 401 responses.
 */
export function markSessionExpired(): void {
  if (expired) {
    return;
  }
  expired = true;
  for (const listener of Array.from(listeners)) {
    try {
      listener();
    } catch {
      // A failing listener must not re-trigger expiry handling or block others.
    }
  }
}

/** Clears the expired state after a fresh signed-in session is observed. */
export function resetSessionExpired(): void {
  expired = false;
}

/**
 * Records an HTTP status from a protected call. Only 401 expires the session.
 */
export function noteAuthStatus(status: number): void {
  if (status === 401) {
    markSessionExpired();
  }
}

/** Subscribes to the expiry transition. Returns an unsubscribe function. */
export function onSessionExpired(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}
