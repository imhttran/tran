// Session and CSRF cookie definitions for the authenticated /api/* proxy.
//
// Next.js validates the exports of a route file (src/app/api/[...path]/route.ts)
// strictly: only HTTP method handlers and a small set of config fields are
// allowed. Shared cookie helpers therefore live here, in a plain module, not in
// the route file.

// The session cookie carries the verified bearer credential. It is HttpOnly so
// browser JavaScript cannot read or forge it.
export const SESSION_COOKIE = "rag_session";
// The CSRF cookie is readable by browser JavaScript (the double-submit pattern).
export const CSRF_COOKIE = "rag_csrf";
// The header the browser sends with the CSRF token on state-changing requests.
export const CSRF_HEADER = "x-csrf-token";

// OIDC transient cookies: short-lived, HttpOnly, and scoped to the auth routes.
export const OIDC_STATE_COOKIE = "rag_oidc_state";
export const OIDC_NONCE_COOKIE = "rag_oidc_nonce";
export const OIDC_VERIFIER_COOKIE = "rag_oidc_verifier";

// Hard ceiling for a browser session. The ID token's remaining lifetime always
// wins when it is shorter; a cookie is never allowed to outlive the token.
export const SESSION_MAX_SECONDS = 60 * 60;

// Secure cookies require HTTPS. Production must be secure; local HTTP E2E can
// opt out explicitly without weakening a deployed environment.
export function cookieSecure(): boolean {
  return (
    process.env.NODE_ENV === "production" ||
    process.env.OIDC_COOKIE_SECURE === "true"
  );
}

// Derives the cookie lifetime from the accepted token. The result is zero when
// the token is already expired, and is capped so a long-lived token cannot
// create an hour-plus session cookie. It never exceeds the token's own expiry.
// Fractional seconds are truncated, which can only shorten a lifetime, never
// extend one.
export function sessionLifetimeSeconds(
  expSeconds: number,
  nowSeconds: number,
): number {
  if (!Number.isFinite(expSeconds) || !Number.isFinite(nowSeconds)) {
    return 0;
  }
  const remaining = Math.floor(expSeconds - nowSeconds);
  if (remaining <= 0) {
    return 0;
  }
  return Math.min(remaining, SESSION_MAX_SECONDS);
}

function boundedMaxAge(maxAge: number): number {
  if (!Number.isFinite(maxAge) || maxAge <= 0) {
    return 0;
  }
  return Math.min(Math.floor(maxAge), SESSION_MAX_SECONDS);
}

type CookieAttributes = {
  name: string;
  value: string;
  httpOnly: boolean;
  secure: boolean;
  sameSite: "lax";
  path: string;
  maxAge: number;
};

// Builds the Secure, HttpOnly, SameSite session cookie attributes. `maxAge`
// comes from `sessionLifetimeSeconds`; the default is only for callers that do
// not have a token yet (such as tests).
export function sessionCookie(
  value: string,
  maxAge: number = SESSION_MAX_SECONDS,
): CookieAttributes {
  return {
    name: SESSION_COOKIE,
    value,
    httpOnly: true,
    secure: cookieSecure(),
    sameSite: "lax",
    path: "/",
    maxAge: boundedMaxAge(maxAge),
  };
}

export function csrfCookie(
  value: string,
  maxAge: number = SESSION_MAX_SECONDS,
): CookieAttributes {
  return {
    name: CSRF_COOKIE,
    value,
    httpOnly: false,
    secure: cookieSecure(),
    sameSite: "lax",
    path: "/",
    maxAge: boundedMaxAge(maxAge),
  };
}

// Expires the browser session and its CSRF companion. Route handlers apply
// these to the response; server components never mutate cookies.
export function clearedSessionCookies(): CookieAttributes[] {
  return [
    {
      name: SESSION_COOKIE,
      value: "",
      httpOnly: true,
      secure: cookieSecure(),
      sameSite: "lax",
      path: "/",
      maxAge: 0,
    },
    {
      name: CSRF_COOKIE,
      value: "",
      httpOnly: false,
      secure: cookieSecure(),
      sameSite: "lax",
      path: "/",
      maxAge: 0,
    },
  ];
}

export function transientCookie(name: string, value: string) {
  return {
    name,
    value,
    httpOnly: true,
    secure: cookieSecure(),
    sameSite: "lax" as const,
    path: "/api/auth",
    maxAge: 10 * 60,
  };
}
