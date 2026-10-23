// Server-only automatic session renewal.
//
// The browser holds an opaque `rag_sid` and the current `rag_session` token.
// This module asks the Rust API to renew the session shortly before the token
// expires. The refresh token never leaves the Rust process; only the new ID
// token and its lifetime cross this boundary.
//
// A transient provider failure keeps the session and is reported as
// `unavailable`; only a definitive expiry or revocation returns `expired`.

import {
  SESSION_MAX_SECONDS,
  csrfCookie,
  sessionCookie,
  sidCookie,
} from "./proxy-cookies";
import type { CookieAttributes } from "./proxy-cookies";

const DEFAULT_REFRESH_WINDOW_SECONDS = 60;
const TOUCH_INTERVAL_MS = 30_000;
const REQUEST_TIMEOUT_MS = 12_000;

export type RenewalResult =
  | {
      kind: "ok";
      idToken: string;
      expiresIn: number;
      absoluteExpiresIn: number;
    }
  | { kind: "expired" }
  | { kind: "busy" }
  | { kind: "unavailable" }
  | { kind: "not_configured" };

// Per-worker touch throttle. Several requests in the same process do not each
// need to move the idle deadline; one call per minute is enough.
const lastTouch = new Map<string, number>();

function apiUrl(): string {
  return (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
}

function internalSecret(): string | null {
  const value = process.env.WEB_SESSION_SECRET?.trim();
  return value ? value : null;
}

/** True when the deployment has a session store and a shared internal secret. */
export function renewalConfigured(): boolean {
  return Boolean(internalSecret());
}

/** Seconds before expiry at which renewal starts. */
export function refreshWindowSeconds(): number {
  const raw = Number(process.env.SESSION_REFRESH_WINDOW_SECONDS);
  if (!Number.isFinite(raw) || raw < 0) {
    return DEFAULT_REFRESH_WINDOW_SECONDS;
  }
  return raw;
}

/** Reads `exp` from a JWT without verifying it. Never an authorization check. */
export function decodeTokenExp(token: string): number | null {
  const parts = token.split(".");
  if (parts.length !== 3) {
    return null;
  }
  try {
    const payload = JSON.parse(
      Buffer.from(parts[1], "base64url").toString("utf8"),
    ) as { exp?: unknown };
    return typeof payload.exp === "number" ? payload.exp : null;
  } catch {
    return null;
  }
}

/** True while the token still has at least `marginSeconds` of validity. */
export function tokenIsValid(
  token: string | null,
  marginSeconds = 5,
  nowSeconds: number = Date.now() / 1000,
): boolean {
  if (!token) {
    return false;
  }
  const exp = decodeTokenExp(token);
  return exp !== null && exp > nowSeconds + marginSeconds;
}

/** True when the token is missing, unreadable, or inside the refresh window. */
export function tokenNeedsRenewal(
  token: string | null,
  nowSeconds: number = Date.now() / 1000,
): boolean {
  if (!token) {
    return true;
  }
  const exp = decodeTokenExp(token);
  return exp === null || exp - nowSeconds <= refreshWindowSeconds();
}

/**
 * True when this worker has not contacted the store for the session recently.
 * This is a read-only check: the throttle is recorded only by `markTouched`, so
 * a failed renewal is retried on the next request instead of being suppressed
 * for a minute.
 */
export function renewalDue(sessionId: string): boolean {
  const last = lastTouch.get(sessionId) ?? 0;
  return Date.now() - last >= TOUCH_INTERVAL_MS;
}

/** Records a successful store contact for this session in this worker. */
export function markTouched(sessionId: string): void {
  const now = Date.now();
  lastTouch.set(sessionId, now);
  if (lastTouch.size > 10_000) {
    for (const [key, value] of lastTouch) {
      if (now - value > TOUCH_INTERVAL_MS) {
        lastTouch.delete(key);
      }
    }
  }
}

/** Records a signed-in session in the Rust store. `null` when unavailable. */
export async function createStoredSession(
  sessionId: string,
  idToken: string,
  refreshToken: string,
): Promise<{ absoluteExpiresIn: number; idleExpiresIn: number } | null> {
  const secret = internalSecret();
  if (!secret) {
    return null;
  }
  try {
    const response = await fetch(`${apiUrl()}/api/internal/sessions`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        "x-internal-secret": secret,
      },
      body: JSON.stringify({
        session_id: sessionId,
        id_token: idToken,
        refresh_token: refreshToken,
      }),
      cache: "no-store",
      signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
    });
    if (!response.ok) {
      return null;
    }
    const body = (await response.json()) as {
      absolute_expires_in?: unknown;
      idle_expires_in?: unknown;
    };
    if (
      typeof body.absolute_expires_in !== "number" ||
      typeof body.idle_expires_in !== "number"
    ) {
      return null;
    }
    return {
      absoluteExpiresIn: body.absolute_expires_in,
      idleExpiresIn: body.idle_expires_in,
    };
  } catch {
    return null;
  }
}

function sleep(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

/**
 * Renews or touches a stored session. A concurrent renewal returns `busy`; it
 * is retried briefly, then reported so the caller can keep the current token.
 */
export async function renewStoredSession(
  sessionId: string,
): Promise<RenewalResult> {
  const secret = internalSecret();
  if (!secret) {
    return { kind: "not_configured" };
  }
  for (let attempt = 0; attempt < 3; attempt += 1) {
    try {
      const response = await fetch(
        `${apiUrl()}/api/internal/sessions/${encodeURIComponent(sessionId)}/renew`,
        {
          method: "POST",
          headers: { "x-internal-secret": secret },
          cache: "no-store",
          signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
        },
      );
      if (response.status === 401 || response.status === 400) {
        // 401 is an expiry/revocation; 400 means the stored session cannot be
        // decrypted (for example after a key change). Both require sign-in.
        return { kind: "expired" };
      }
      if (response.status === 503) {
        return { kind: "unavailable" };
      }
      if (response.status === 409) {
        if (attempt < 2) {
          await sleep(150 * (attempt + 1));
          continue;
        }
        return { kind: "busy" };
      }
      if (!response.ok) {
        return { kind: "unavailable" };
      }
      const body = (await response.json()) as {
        id_token?: unknown;
        expires_in?: unknown;
        absolute_expires_in?: unknown;
      };
      if (typeof body.id_token !== "string" || body.id_token.length === 0) {
        return { kind: "unavailable" };
      }
      return {
        kind: "ok",
        idToken: body.id_token,
        expiresIn:
          typeof body.expires_in === "number" ? body.expires_in : 0,
        absoluteExpiresIn:
          typeof body.absolute_expires_in === "number"
            ? body.absolute_expires_in
            : 0,
      };
    } catch {
      return { kind: "unavailable" };
    }
  }
  return { kind: "busy" };
}

/** Revokes a stored session. Best-effort and idempotent. */
export async function revokeStoredSession(
  sessionId: string,
  timeoutMs: number = REQUEST_TIMEOUT_MS,
): Promise<void> {
  const secret = internalSecret();
  if (!secret) {
    return;
  }
  try {
    await fetch(
      `${apiUrl()}/api/internal/sessions/${encodeURIComponent(sessionId)}`,
      {
        method: "DELETE",
        headers: { "x-internal-secret": secret },
        cache: "no-store",
        signal: AbortSignal.timeout(timeoutMs),
      },
    );
  } catch {
    // The local cookies are cleared regardless; an unreachable store expires on
    // its own idle/absolute deadline.
  }
}

/**
 * Cookie updates for a renewed token. The ID-token and CSRF lifetimes stay
 * bounded by the token; the opaque session id lives to the absolute limit.
 */
export function renewalCookies(
  sessionId: string,
  idToken: string,
  expiresIn: number,
  absoluteExpiresIn: number,
  csrf: string | null,
): CookieAttributes[] {
  const tokenMaxAge = Math.max(
    1,
    Math.min(expiresIn > 0 ? expiresIn : SESSION_MAX_SECONDS, SESSION_MAX_SECONDS),
  );
  const updates: CookieAttributes[] = [
    sessionCookie(idToken, tokenMaxAge),
    sidCookie(sessionId, Math.max(1, absoluteExpiresIn)),
  ];
  if (csrf) {
    updates.push(csrfCookie(csrf, tokenMaxAge));
  }
  return updates;
}
