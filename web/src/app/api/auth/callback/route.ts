import { NextRequest, NextResponse } from "next/server";

import {
  OIDC_NONCE_COOKIE,
  OIDC_STATE_COOKIE,
  OIDC_VERIFIER_COOKIE,
  csrfCookie,
  sessionCookie,
  sessionLifetimeSeconds,
} from "@/lib/proxy-cookies";
import {
  audienceMatches,
  decodeIdTokenClaims,
  oidcSettings,
  randomString,
  requestOrigin,
  safeEqual,
} from "@/lib/oidc";

export const dynamic = "force-dynamic";

function clearTransient(response: NextResponse) {
  for (const name of [
    OIDC_STATE_COOKIE,
    OIDC_NONCE_COOKIE,
    OIDC_VERIFIER_COOKIE,
  ]) {
    response.cookies.set({ name, value: "", path: "/api/auth", maxAge: 0 });
  }
}

function fail(origin: string, code: string): NextResponse {
  const response = NextResponse.redirect(
    new URL(`/signin?error=${encodeURIComponent(code)}`, origin),
    303,
  );
  clearTransient(response);
  return response;
}

// Completes the authorization-code exchange. The Rust API is the cryptographic
// authority: after the local state/nonce/issuer/audience/expiry checks, the ID
// token is sent to /api/account. Only a 2xx response establishes a session.
export async function GET(request: NextRequest) {
  const origin = requestOrigin(request.headers, request.nextUrl);
  const settings = oidcSettings(origin);
  if (!settings) {
    return fail(origin, "unconfigured");
  }

  const state = request.nextUrl.searchParams.get("state");
  const code = request.nextUrl.searchParams.get("code");
  const cookieState = request.cookies.get(OIDC_STATE_COOKIE)?.value;
  const nonce = request.cookies.get(OIDC_NONCE_COOKIE)?.value;
  const verifier = request.cookies.get(OIDC_VERIFIER_COOKIE)?.value;

  if (!state || !code || !cookieState || !nonce || !verifier) {
    return fail(origin, "invalid_callback");
  }
  if (!safeEqual(state, cookieState)) {
    return fail(origin, "state_mismatch");
  }

  const body = new URLSearchParams({
    grant_type: "authorization_code",
    code,
    redirect_uri: settings.redirectUri,
    client_id: settings.clientId,
    code_verifier: verifier,
  });
  if (settings.clientSecret) {
    body.set("client_secret", settings.clientSecret);
  }

  let idToken: string | null = null;
  try {
    const tokenResponse = await fetch(settings.tokenEndpoint, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded" },
      body,
      cache: "no-store",
    });
    if (!tokenResponse.ok) {
      return fail(origin, "token_exchange_failed");
    }
    const payload = (await tokenResponse.json()) as { id_token?: unknown };
    if (typeof payload.id_token !== "string" || !payload.id_token) {
      return fail(origin, "missing_id_token");
    }
    idToken = payload.id_token;
  } catch {
    return fail(origin, "provider_unavailable");
  }

  const claims = decodeIdTokenClaims(idToken);
  const now = Math.floor(Date.now() / 1000);
  if (
    !claims ||
    claims.iss !== settings.issuer ||
    !audienceMatches(claims.aud, settings.clientId) ||
    typeof claims.exp !== "number" ||
    claims.exp <= now ||
    claims.nonce !== nonce
  ) {
    return fail(origin, "invalid_id_token");
  }

  // The browser session may never outlive the accepted token. A token that
  // expires within the same second yields no session at all.
  const sessionLifetime = sessionLifetimeSeconds(claims.exp, now);
  if (sessionLifetime <= 0) {
    return fail(origin, "invalid_id_token");
  }

  // Delegate signature verification and database-backed role/grant resolution
  // to Rust. A failure never becomes an anonymous session.
  const api = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
  try {
    const account = await fetch(`${api}/api/account`, {
      headers: { authorization: `Bearer ${idToken}` },
      cache: "no-store",
    });
    if (!account.ok) {
      return fail(origin, account.status === 403 ? "not_provisioned" : "session_denied");
    }
  } catch {
    return fail(origin, "account_unavailable");
  }

  const response = NextResponse.redirect(new URL("/", origin), 303);
  response.cookies.set(sessionCookie(idToken, sessionLifetime));
  response.cookies.set(csrfCookie(randomString(32), sessionLifetime));
  clearTransient(response);
  return response;
}
