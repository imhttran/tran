// Server-side OIDC authorization-code + PKCE helpers.
//
// The browser never receives a token, client secret, or provider configuration.
// The Next.js server performs the code exchange, sanity-checks issuer/audience/
// expiry/nonce, and then asks the Rust API to verify the credential before it
// stores the HttpOnly session cookie. Rust remains the cryptographic authority.

import { createHash, randomBytes, timingSafeEqual } from "node:crypto";

export type OidcSettings = {
  issuer: string;
  clientId: string;
  clientSecret: string | null;
  redirectUri: string;
  authorizationEndpoint: string;
  tokenEndpoint: string;
  endSessionEndpoint: string | null;
  scopes: string;
};

function trimmed(value: string | undefined): string | null {
  const result = value?.trim();
  return result ? result : null;
}

export function oidcSettings(origin: string): OidcSettings | null {
  const issuer = trimmed(process.env.OIDC_ISSUER);
  const clientId = trimmed(process.env.OIDC_CLIENT_ID);
  if (!issuer || !clientId) {
    return null;
  }
  return {
    issuer,
    clientId,
    clientSecret: trimmed(process.env.OIDC_CLIENT_SECRET),
    redirectUri:
      trimmed(process.env.OIDC_REDIRECT_URI) ?? `${origin}/api/auth/callback`,
    authorizationEndpoint:
      trimmed(process.env.OIDC_AUTHORIZATION_ENDPOINT) ?? `${issuer}/authorize`,
    tokenEndpoint:
      trimmed(process.env.OIDC_TOKEN_ENDPOINT) ?? `${issuer}/token`,
    endSessionEndpoint:
      trimmed(process.env.OIDC_END_SESSION_ENDPOINT) ?? `${issuer}/logout`,
    scopes: trimmed(process.env.OIDC_SCOPES) ?? "openid profile email",
  };
}

export function oidcConfigured(): boolean {
  return Boolean(
    trimmed(process.env.OIDC_ISSUER) && trimmed(process.env.OIDC_CLIENT_ID),
  );
}

// Uses the actual request Host header so a redirect stays on the same host that
// received the callback. Next.js may canonicalize nextUrl.origin to localhost.
export function requestOrigin(headers: Headers, fallback: URL): string {
  const host = headers.get("host");
  if (!host) {
    return fallback.origin;
  }
  const forwarded = headers.get("x-forwarded-proto");
  const protocol =
    forwarded === "https" || fallback.protocol === "https:" ? "https" : "http";
  return `${protocol}://${host}`;
}

export function randomString(bytes = 32): string {
  return randomBytes(bytes).toString("base64url");
}

export function pkceChallenge(verifier: string): string {
  return createHash("sha256").update(verifier).digest("base64url");
}

export function safeEqual(left: string, right: string): boolean {
  const a = Buffer.from(left);
  const b = Buffer.from(right);
  return a.length === b.length && timingSafeEqual(a, b);
}

// Accepts only a same-origin absolute path as a post-sign-in destination. A
// scheme, a protocol-relative URL, or a backslash would let an invitation link
// bounce the browser to another host, so anything else falls back to the home
// page.
export function safeReturnPath(value: string | null | undefined): string {
  if (!value) return "/";
  if (!value.startsWith("/")) return "/";
  if (value.startsWith("//") || value.startsWith("/\\")) return "/";
  if (value.includes("\\") || value.includes("://")) return "/";
  return value;
}

export type IdTokenClaims = {
  iss?: string;
  aud?: string | string[];
  sub?: string;
  exp?: number;
  nonce?: string;
};

export function decodeIdTokenClaims(token: string): IdTokenClaims | null {
  const parts = token.split(".");
  if (parts.length !== 3) {
    return null;
  }
  try {
    const payload = JSON.parse(
      Buffer.from(parts[1], "base64url").toString("utf8"),
    ) as IdTokenClaims;
    return payload;
  } catch {
    return null;
  }
}

export function audienceMatches(
  audience: string | string[] | undefined,
  clientId: string,
): boolean {
  if (typeof audience === "string") {
    return audience === clientId;
  }
  return Array.isArray(audience) && audience.includes(clientId);
}
