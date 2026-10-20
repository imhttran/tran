import { NextResponse } from "next/server";

import {
  CSRF_COOKIE,
  OIDC_NONCE_COOKIE,
  OIDC_STATE_COOKIE,
  OIDC_VERIFIER_COOKIE,
  SESSION_COOKIE,
  cookieSecure,
} from "@/lib/proxy-cookies";
import { oidcSettings } from "@/lib/oidc";

// Clears the browser session. The authoritative session is the HttpOnly cookie;
// the CSRF cookie is cleared with it. This route is intentionally outside the
// catch-all proxy so sign-out never depends on the Rust API being reachable.
export async function POST(request: Request) {
  const origin = new URL(request.url).origin;
  const settings = oidcSettings(origin);
  const response = NextResponse.json({
    ok: true,
    end_session_endpoint: settings?.endSessionEndpoint ?? null,
    client_id: settings?.clientId ?? null,
  });
  for (const name of [SESSION_COOKIE, CSRF_COOKIE]) {
    response.cookies.set({
      name,
      value: "",
      httpOnly: name === SESSION_COOKIE,
      secure: cookieSecure(),
      sameSite: "lax",
      path: "/",
      maxAge: 0,
    });
  }
  for (const name of [
    OIDC_STATE_COOKIE,
    OIDC_NONCE_COOKIE,
    OIDC_VERIFIER_COOKIE,
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
