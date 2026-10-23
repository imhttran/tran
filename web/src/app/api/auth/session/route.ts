// Session endpoint for the browser. It returns only the account view (state,
// role, display label). Renewal happens here and in the /api/* proxy, both of
// which are route handlers that may legally set cookies. No token, subject,
// email, OIDC credential, refresh token, or database URL crosses to the
// browser.

import { NextRequest, NextResponse } from "next/server";

import {
  CSRF_COOKIE,
  SESSION_COOKIE,
  SID_COOKIE,
  clearedSessionCookies,
} from "@/lib/proxy-cookies";
import type { CookieAttributes } from "@/lib/proxy-cookies";
import { fetchSessionView } from "@/lib/session";
import {
  renewalConfigured,
  renewalCookies,
  renewStoredSession,
  tokenIsValid,
  markTouched,
  renewalDue,
  tokenNeedsRenewal,
} from "@/lib/session-store";

export const dynamic = "force-dynamic";

function noStore(body: unknown, status: number): NextResponse {
  return NextResponse.json(body, {
    status,
    headers: { "cache-control": "no-store" },
  });
}

function expired(): NextResponse {
  const response = noStore({ error: "expired" }, 401);
  for (const cookie of clearedSessionCookies()) {
    response.cookies.set(cookie);
  }
  return response;
}

function withCookies(
  response: NextResponse,
  updates: CookieAttributes[],
): NextResponse {
  for (const cookie of updates) {
    response.cookies.set(cookie);
  }
  return response;
}

export async function GET(request: NextRequest) {
  let token = request.cookies.get(SESSION_COOKIE)?.value ?? null;
  const sessionId = request.cookies.get(SID_COOKIE)?.value ?? null;
  const csrf = request.cookies.get(CSRF_COOKIE)?.value ?? null;
  let updates: CookieAttributes[] = [];

  if (sessionId && renewalConfigured()) {
    if (tokenNeedsRenewal(token) || renewalDue(sessionId)) {
      const renewal = await renewStoredSession(sessionId);
      if (renewal.kind === "ok") {
        markTouched(sessionId);
        token = renewal.idToken;
        updates = renewalCookies(
          sessionId,
          renewal.idToken,
          renewal.expiresIn,
          renewal.absoluteExpiresIn,
          csrf,
        );
      } else if (renewal.kind === "expired") {
        return expired();
      } else if (!tokenIsValid(token)) {
        // Busy or unavailable renewal: the session is kept. Only a token that
        // can no longer carry the request yields a service error. The session is
        // kept; this is a service failure, not an expiry.
        return noStore({ error: "provider_unavailable" }, 503);
      }
    }
  }

  if (!token) {
    return noStore({ state: "anonymous" }, 200);
  }

  const result = await fetchSessionView(token);
  if (result.kind === "allowed") {
    return withCookies(noStore(result.data, 200), updates);
  }
  if (result.kind === "expired") {
    return expired();
  }
  return withCookies(noStore({ state: "anonymous" }, 200), updates);
}
