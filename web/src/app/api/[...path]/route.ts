// Authenticated server-side proxy for /api/*.
//
// This route replaces the previous unrestricted Next.js rewrite. Every browser
// request to /api/* lands here, so the Next.js server is the only component that
// talks to the Rust Axum API. The proxy:
//
//   * renews the browser session shortly before the ID token expires (when the
//     Rust session store is configured) and forwards the renewed token;
//   * forwards ONLY the verified identity credential (the `Authorization: Bearer`
//     header derived from the HttpOnly session cookie or the incoming bearer);
//   * strips every client-supplied identity header (X-User, X-Role, X-Subject,
//     ...) so they can never reach the API and elevate permissions;
//   * rejects state-changing requests (anything but GET/HEAD/OPTIONS) without a
//     valid CSRF token that matches the session cookie (double-submit);
//   * performs no authorization decision of its own. RBAC stays in the Rust
//     policy; a request without a credential yields the API's 401 and a request
//     with an insufficient role yields the API's 403;
//   * never exposes the internal session endpoints to the browser.
//
// Next.js checks this file's exports strictly, so shared cookie definitions live
// in src/lib/proxy-cookies.ts and this file exports only HTTP handlers.

import { timingSafeEqual } from "node:crypto";
import { NextRequest, NextResponse } from "next/server";

import {
  CSRF_COOKIE,
  CSRF_HEADER,
  SESSION_COOKIE,
  SID_COOKIE,
  clearedSessionCookies,
} from "../../../lib/proxy-cookies";
import type { CookieAttributes } from "../../../lib/proxy-cookies";
import {
  renewalConfigured,
  renewalCookies,
  renewStoredSession,
  revokeStoredSession,
  tokenIsValid,
  markTouched,
  renewalDue,
  tokenNeedsRenewal,
} from "../../../lib/session-store";

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");

// Client-supplied headers that must never reach the API. The only identity
// source is the bearer credential the proxy itself sets.
const STRIPPED_REQUEST_HEADERS = [
  "x-user",
  "x-role",
  "x-subject",
  "x-identity",
  "x-forwarded-user",
  "x-auth-request-user",
  "x-auth-request-groups",
];

// Response headers the API may set that the proxy does not pass through.
const HOP_BY_HOP = [
  "connection",
  "keep-alive",
  "proxy-authenticate",
  "proxy-authorization",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
];

function jsonError(
  status: number,
  code: string,
  message: string,
): NextResponse {
  return NextResponse.json(
    { ok: false, error: code, message },
    { status, headers: { "cache-control": "no-store" } },
  );
}

function cleared(status: number, code: string, message: string): NextResponse {
  const response = jsonError(status, code, message);
  for (const cookie of clearedSessionCookies()) {
    response.cookies.set(cookie);
  }
  return response;
}

function isStateChanging(method: string): boolean {
  return !["GET", "HEAD", "OPTIONS"].includes(method.toUpperCase());
}

function readCookie(request: NextRequest, name: string): string | null {
  const value = request.cookies.get(name)?.value;
  return value && value.trim() !== "" ? value : null;
}

function csrfTokensMatch(cookieToken: string, headerToken: string): boolean {
  const a = Buffer.from(cookieToken);
  const b = Buffer.from(headerToken);
  return a.length === b.length && timingSafeEqual(a, b);
}

// The HttpOnly session cookie is the source of truth; a caller-provided
// Authorization header is accepted only when it is a well-formed bearer token,
// so the API still verifies it. Client identity headers never influence this.
function resolveBearer(
  request: NextRequest,
  sessionToken: string | null,
): string | null {
  if (sessionToken) {
    return sessionToken.startsWith("Bearer ")
      ? sessionToken
      : `Bearer ${sessionToken}`;
  }
  const header = request.headers.get("authorization");
  if (header && /^Bearer\s+\S+/i.test(header.trim())) {
    return header.trim();
  }
  return null;
}

function buildUpstreamHeaders(
  request: NextRequest,
  bearer: string | null,
): Headers {
  const headers = new Headers();
  const contentType = request.headers.get("content-type");
  if (contentType) {
    headers.set("content-type", contentType);
  }
  const accept = request.headers.get("accept");
  if (accept) {
    headers.set("accept", accept);
  }
  // Forward only the verified identity credential and the body framing. Every
  // client identity header is dropped.
  if (bearer) {
    headers.set("authorization", bearer);
  }
  for (const name of STRIPPED_REQUEST_HEADERS) {
    headers.delete(name);
  }
  return headers;
}

function forbiddenCsrf(): NextResponse {
  return jsonError(
    403,
    "csrf_failed",
    "the request lacks a valid CSRF token",
  );
}

async function proxy(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> },
): Promise<NextResponse> {
  const { path } = await params;

  // The internal session endpoints are server-to-server only. A browser request
  // can never reach them through this proxy.
  if (path[0] === "internal") {
    return jsonError(404, "not_found", "no such endpoint");
  }

  const target = `${API_URL}/api/${path.map(encodeURIComponent).join("/")}${request.nextUrl.search}`;

  let sessionToken = readCookie(request, SESSION_COOKIE);
  const sessionId = readCookie(request, SID_COOKIE);
  const csrf = readCookie(request, CSRF_COOKIE);
  let updates: CookieAttributes[] = [];

  if (sessionId && renewalConfigured()) {
    if (tokenNeedsRenewal(sessionToken) || renewalDue(sessionId) || !csrf) {
      const renewal = await renewStoredSession(sessionId);
      if (renewal.kind === "ok") {
        markTouched(sessionId);
        sessionToken = renewal.idToken;
        updates = renewalCookies(
          sessionId,
          renewal.idToken,
          renewal.expiresIn,
          renewal.absoluteExpiresIn,
          csrf,
        );
      } else if (renewal.kind === "expired") {
        return cleared(
          401,
          "unauthenticated",
          "the browser session has expired",
        );
      } else if (!tokenIsValid(sessionToken)) {
        // Busy or unavailable renewal: the session is kept. Only a token that
        // can no longer carry the request yields a service error. Keep the session and report a
        // service failure so this is not mistaken for an authentication result.
        return jsonError(
          503,
          "provider_unavailable",
          "the identity provider is temporarily unavailable",
        );
      }
    }
  }

  const bearer = resolveBearer(request, sessionToken);

  // Default-deny for state-changing requests without any credential. A public
  // health probe remains available because it is a GET. This rejects an
  // unauthenticated write before it reaches the API, so a missing endpoint never
  // turns into a 404 that the browser could mistake for a record decision.
  if (isStateChanging(request.method) && !bearer) {
    return jsonError(
      401,
      "unauthenticated",
      "authentication is required for this request",
    );
  }

  // CSRF: state-changing requests need the header token to match the cookie.
  // Anonymous requests carry no session to protect; they are forwarded and the
  // API returns 401 before any write. A browser session must prove the
  // double-submit token.
  if (isStateChanging(request.method) && bearer) {
    const headerToken = request.headers.get(CSRF_HEADER);
    if (!csrf || !headerToken || !csrfTokensMatch(csrf, headerToken)) {
      return forbiddenCsrf();
    }
  }

  // Build the request body unchanged. Multipart and JSON bodies both stream.
  const method = request.method.toUpperCase();
  const hasBody = !["GET", "HEAD"].includes(method);
  const body = hasBody ? await request.arrayBuffer() : undefined;

  let upstream: Response;
  try {
    upstream = await fetch(target, {
      method,
      headers: buildUpstreamHeaders(request, bearer),
      body: body && body.byteLength > 0 ? body : undefined,
      redirect: "manual",
      cache: "no-store",
    });
  } catch (error) {
    return jsonError(
      502,
      "upstream_unavailable",
      `the API is unreachable (${
        error instanceof Error ? error.message : "network error"
      })`,
    );
  }

  const responseHeaders = new Headers();
  upstream.headers.forEach((value, key) => {
    if (!HOP_BY_HOP.includes(key.toLowerCase())) {
      responseHeaders.set(key, value);
    }
  });
  responseHeaders.set("cache-control", "no-store");

  const payload = await upstream.arrayBuffer();
  const response = new NextResponse(payload.byteLength > 0 ? payload : null, {
    status: upstream.status,
    headers: responseHeaders,
  });

  // A renewed token is only useful if the browser receives it with this
  // response.
  for (const cookie of updates) {
    response.cookies.set(cookie);
  }

  // An authentication rejection of a browser session means the token in the
  // HttpOnly cookie is no longer usable. Clear the stale session, its opaque
  // renewal id, and the CSRF cookie on the same response while preserving the
  // API's 401 body and status for every caller. A bearer-token client (no
  // session cookie) is untouched.
  if (
    upstream.status === 401 &&
    (readCookie(request, SESSION_COOKIE) || sessionId)
  ) {
    if (sessionId) {
      // Do not let a rejected session be renewed later. The revocation is
      // bounded so the 401 is not delayed by an unreachable store or provider.
      await revokeStoredSession(sessionId, 2_000);
    }
    for (const cookie of clearedSessionCookies()) {
      response.cookies.set(cookie);
    }
  }

  return response;
}

export const GET = proxy;
export const POST = proxy;
export const PUT = proxy;
export const PATCH = proxy;
export const DELETE = proxy;
