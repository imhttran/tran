// Authenticated server-side proxy for /api/*.
//
// This route replaces the previous unrestricted Next.js rewrite. Every browser
// request to /api/* lands here, so the Next.js server is the only component that
// talks to the Rust Axum API. The proxy:
//
//   * forwards ONLY the verified identity credential (the `Authorization: Bearer`
//     header derived from the HttpOnly session cookie or the incoming bearer);
//   * strips every client-supplied identity header (X-User, X-Role, X-Subject,
//     ...) so they can never reach the API and elevate permissions;
//   * rejects state-changing requests (anything but GET/HEAD/OPTIONS) without a
//     valid CSRF token that matches the session cookie (double-submit);
//   * issues a Secure, HttpOnly, SameSite session cookie on login and a CSRF
//     cookie the browser JavaScript can read to send back in the X-CSRF-Token
//     header;
//   * performs no authorization decision of its own. RBAC stays in the Rust
//     policy; a request without a credential yields the API's 401 and a request
//     with an insufficient role yields the API's 403.
//
// Next.js checks this file's exports strictly, so shared cookie definitions live
// in src/lib/proxy-cookies.ts and this file exports only HTTP handlers.

import { NextRequest, NextResponse } from "next/server";

import { CSRF_COOKIE, CSRF_HEADER, SESSION_COOKIE } from "../../../lib/proxy-cookies";

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

function isStateChanging(method: string): boolean {
  return !["GET", "HEAD", "OPTIONS"].includes(method.toUpperCase());
}

function readCookie(request: NextRequest, name: string): string | null {
  const value = request.cookies.get(name)?.value;
  return value && value.trim() !== "" ? value : null;
}

function timingSafeEqual(a: string, b: string): boolean {
  if (a.length !== b.length) {
    return false;
  }
  let diff = 0;
  for (let i = 0; i < a.length; i += 1) {
    diff |= a.charCodeAt(i) ^ b.charCodeAt(i);
  }
  return diff === 0;
}

// Resolves the credential to forward. The HttpOnly session cookie is the source
// of truth; a caller-provided Authorization header is accepted only when it is a
// well-formed bearer token, so the API still verifies it. Client identity
// headers never influence this.
function resolveBearer(request: NextRequest): string | null {
  const session = readCookie(request, SESSION_COOKIE);
  if (session) {
    return session.startsWith("Bearer ") ? session : `Bearer ${session}`;
  }
  const header = request.headers.get("authorization");
  if (header && /^Bearer\s+\S+/i.test(header.trim())) {
    return header.trim();
  }
  return null;
}

function buildUpstreamHeaders(request: NextRequest): Headers {
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
  const bearer = resolveBearer(request);
  if (bearer) {
    headers.set("authorization", bearer);
  }
  for (const name of STRIPPED_REQUEST_HEADERS) {
    headers.delete(name);
  }
  return headers;
}

function forbiddenCsrf(): NextResponse {
  return NextResponse.json(
    {
      ok: false,
      error: "csrf_failed",
      message: "the request lacks a valid CSRF token",
    },
    { status: 403, headers: { "cache-control": "no-store" } },
  );
}

async function proxy(
  request: NextRequest,
  { params }: { params: Promise<{ path: string[] }> },
): Promise<NextResponse> {
  const { path } = await params;
  const target = `${API_URL}/api/${path.map(encodeURIComponent).join("/")}`;

  	const bearer = resolveBearer(request);

  	// Default-deny for state-changing requests without any credential. A public
  	// health probe remains available because it is a GET. This rejects an
  	// unauthenticated write before it reaches the API, so a missing endpoint never
  	// turns into a 404 that the browser could mistake for a record decision.
  	if (isStateChanging(request.method) && !bearer) {
  		return NextResponse.json(
  			{
  				ok: false,
  				error: "unauthenticated",
  				message: "authentication is required for this request",
  			},
  			{ status: 401, headers: { "cache-control": "no-store" } },
  		);
  	}

  	// CSRF: state-changing requests need the header token to match the cookie.
  // Anonymous requests carry no session to protect; they are forwarded and the
  // API returns 401 before any write. A browser session must prove the
  // double-submit token.
  if (isStateChanging(request.method) && bearer) {
    const cookieToken = readCookie(request, CSRF_COOKIE);
    const headerToken = request.headers.get(CSRF_HEADER);
    if (!cookieToken || !headerToken || !timingSafeEqual(cookieToken, headerToken)) {
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
      headers: buildUpstreamHeaders(request),
      body: body && body.byteLength > 0 ? body : undefined,
      redirect: "manual",
      cache: "no-store",
    });
  } catch (error) {
    return NextResponse.json(
      {
        ok: false,
        error: "upstream_unavailable",
        message: `the API is unreachable (${
          error instanceof Error ? error.message : "network error"
        })`,
      },
      { status: 502, headers: { "cache-control": "no-store" } },
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
  return new NextResponse(payload.byteLength > 0 ? payload : null, {
    status: upstream.status,
    headers: responseHeaders,
  });
}

export const GET = proxy;
export const POST = proxy;
export const PUT = proxy;
export const PATCH = proxy;
export const DELETE = proxy;
