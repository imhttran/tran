import { NextRequest, NextResponse } from "next/server";

import {
  OIDC_NONCE_COOKIE,
  OIDC_RETURN_COOKIE,
  OIDC_STATE_COOKIE,
  OIDC_VERIFIER_COOKIE,
  transientCookie,
} from "@/lib/proxy-cookies";
import {
  oidcSettings,
  pkceChallenge,
  randomString,
  requestOrigin,
  safeReturnPath,
} from "@/lib/oidc";

export const dynamic = "force-dynamic";

// Starts the OIDC authorization-code + PKCE flow. The state, nonce, and verifier
// live only in short-lived HttpOnly cookies; they are never exposed to client
// JavaScript and never written to logs.
export async function GET(request: NextRequest) {
  const origin = requestOrigin(request.headers, request.nextUrl);
  const settings = oidcSettings(origin);
  if (!settings) {
    return NextResponse.redirect(
      new URL("/signin?error=unconfigured", origin),
      303,
    );
  }

  const state = randomString(32);
  const nonce = randomString(32);
  const verifier = randomString(64);
  const challenge = pkceChallenge(verifier);
  // An invitation link can ask to return to its welcome screen after sign-in.
  // Only a same-origin absolute path is kept.
  const returnTo = safeReturnPath(request.nextUrl.searchParams.get("returnTo"));

  const target = new URL(settings.authorizationEndpoint);
  target.searchParams.set("response_type", "code");
  target.searchParams.set("client_id", settings.clientId);
  target.searchParams.set("redirect_uri", settings.redirectUri);
  target.searchParams.set("scope", settings.scopes);
  target.searchParams.set("state", state);
  target.searchParams.set("nonce", nonce);
  target.searchParams.set("code_challenge", challenge);
  target.searchParams.set("code_challenge_method", "S256");

  const response = NextResponse.redirect(target, 303);
  response.cookies.set(transientCookie(OIDC_STATE_COOKIE, state));
  response.cookies.set(transientCookie(OIDC_NONCE_COOKIE, nonce));
  response.cookies.set(transientCookie(OIDC_VERIFIER_COOKIE, verifier));
  response.cookies.set(transientCookie(OIDC_RETURN_COOKIE, returnTo));
  return response;
}
