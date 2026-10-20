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

// Builds the Secure, HttpOnly, SameSite session cookie attributes. Session
// establishment happens out of band; the cookie attributes live in one place.
export function sessionCookie(value: string): {
  name: string;
  value: string;
  httpOnly: boolean;
  secure: boolean;
  sameSite: "strict";
  path: string;
} {
  return {
    name: SESSION_COOKIE,
    value,
    httpOnly: true,
    secure: true,
    sameSite: "strict",
    path: "/",
  };
}
