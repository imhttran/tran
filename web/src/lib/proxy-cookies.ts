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

// Secure cookies require HTTPS. Production must be secure; local HTTP E2E can
// opt out explicitly without weakening a deployed environment.
export function cookieSecure(): boolean {
  return (
    process.env.NODE_ENV === "production" ||
    process.env.OIDC_COOKIE_SECURE === "true"
  );
}

// Builds the Secure, HttpOnly, SameSite session cookie attributes.
export function sessionCookie(value: string): {
  name: string;
  value: string;
  httpOnly: boolean;
  secure: boolean;
  sameSite: "lax";
  path: string;
  maxAge: number;
} {
  return {
    name: SESSION_COOKIE,
    value,
    httpOnly: true,
    secure: cookieSecure(),
    sameSite: "lax",
    path: "/",
    maxAge: 60 * 60,
  };
}

export function csrfCookie(value: string): {
  name: string;
  value: string;
  httpOnly: boolean;
  secure: boolean;
  sameSite: "lax";
  path: string;
  maxAge: number;
} {
  return {
    name: CSRF_COOKIE,
    value,
    httpOnly: false,
    secure: cookieSecure(),
    sameSite: "lax",
    path: "/",
    maxAge: 60 * 60,
  };
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
