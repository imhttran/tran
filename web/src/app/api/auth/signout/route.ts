import { NextRequest, NextResponse } from "next/server";

import {
  OIDC_NONCE_COOKIE,
  OIDC_RETURN_COOKIE,
  OIDC_STATE_COOKIE,
  OIDC_VERIFIER_COOKIE,
  SID_COOKIE,
  clearedSessionCookies,
  cookieSecure,
} from "@/lib/proxy-cookies";
import { oidcSettings } from "@/lib/oidc";
import { revokeStoredSession } from "@/lib/session-store";

// Clears the browser session. The authoritative session is the HttpOnly cookie;
// the CSRF cookie is cleared with it. This route is intentionally outside the
// catch-all proxy so sign-out never depends on the Rust API being reachable.
export async function POST(request: NextRequest) {
  const origin = new URL(request.url).origin;
  const settings = oidcSettings(origin);
  const sessionId = request.cookies.get(SID_COOKIE)?.value;
  if (sessionId) {
    // Best-effort server-side revocation. The local cookies are cleared below
    // even when the store is unreachable; the row then expires on its limits.
    await revokeStoredSession(sessionId);
  }
  const response = NextResponse.json({
    ok: true,
    end_session_endpoint: settings?.endSessionEndpoint ?? null,
    client_id: settings?.clientId ?? null,
  });
  for (const cookie of clearedSessionCookies()) {
    response.cookies.set(cookie);
  }
  for (const name of [
    OIDC_STATE_COOKIE,
    OIDC_NONCE_COOKIE,
    OIDC_VERIFIER_COOKIE,
    OIDC_RETURN_COOKIE,
  ]) {
    response.cookies.set({
      name,
      value: "",
      httpOnly: true,
      secure: cookieSecure(),
      sameSite: "lax",
      path: "/api/auth",
      maxAge: 0,
    });
  }
  return response;
}
