// Server-side session helpers for the browser experience.
//
// This module runs only on the Next.js server. It reads the HttpOnly session
// cookie and resolves the current account by calling the Rust Axum API through
// the same-origin proxy, then maps the answer into the small SessionView the
// account menu renders. It never sends a token, subject, email, OIDC
// credential, or database URL toward the browser: only a role and a display
// label survive the mapping.

import type {
  Role,
  SessionResult,
  SessionView,
} from "@/lib/experience-contract";

// Field names the browser must never receive. A response that carries one is
// dropped rather than rendered.
const FORBIDDEN =
  /secret|token|credential|database_url|db_url|postgres|oidc|jwks|issuer|email|subject/i;

// Picks the highest effective role from the account response. The API returns a
// roles array; the browser contract carries exactly one role and a display label.
// The subject, issuer, grants, and every credential stay server-side.
function highestRole(roles: unknown): Role | null {
  if (!Array.isArray(roles)) return null;
  const known: Role[] = ["viewer", "editor", "admin"];
  let best: Role | null = null;
  for (const candidate of roles) {
    if (typeof candidate !== "string") continue;
    const role = known.find((value) => value === candidate);
    if (!role) continue;
    if (best === null || known.indexOf(role) > known.indexOf(best)) best = role;
  }
  return best;
}

export function sanitizeSession(raw: unknown): SessionView | null {
  if (!raw || typeof raw !== "object") {
    return null;
  }
  const record = raw as Record<string, unknown>;
  // The account endpoint returns a roles array. A provisioned account with no
  // role is signed in but receives no privileged capability; a missing/invalid
  // account response is handled as anonymous or no-access by the caller.
  const role = highestRole(record.roles);
  // A provisioned account may have no application role yet (for example a
  // newly linked family member). It is still signed in and may use the
  // self-service profile, but it receives no privileged role.
  const view: Record<string, unknown> = {
    state: "signed_in",
    role,
    display_name: role ? `Signed-in ${role}` : "Signed-in member",
  };
  for (const key of Object.keys(view)) {
    if (FORBIDDEN.test(key)) {
      delete view[key];
    }
  }
  return view as unknown as SessionView;
}

// Resolves the account view for a verified token. The cookie is read by the
// route handler, which also owns renewal and cookie updates. The browser never
// sees this call or the token.
export async function fetchSessionView(token: string): Promise<SessionResult> {
  if (!token) {
    return { kind: "allowed", data: { state: "anonymous" } };
  }
  const api = (process.env.API_URL || "http://127.0.0.1:8080").replace(
    /\/+$/,
    "",
  );
  let status: number;
  let payload: unknown = null;
  try {
    const response = await fetch(`${api}/api/account`, {
      headers: {
        authorization: token.startsWith("Bearer ")
          ? token
          : `Bearer ${token}`,
      },
      cache: "no-store",
    });
    status = response.status;
    if (status >= 200 && status < 300) {
      payload = await response.json();
    }
  } catch {
    return { kind: "denied", reason: "The account service is unreachable." };
  }
  if (status === 401) {
    return { kind: "expired", signInUrl: "/signin" };
  }
  if (status === 403) {
    return {
      kind: "denied",
      reason: "Your role does not permit this request.",
    };
  }
  if (status >= 200 && status < 300) {
    const view = sanitizeSession(payload);
    if (!view) {
      return {
        kind: "no_access",
        reason: "No account is available to your identity.",
      };
    }
    return { kind: "allowed", data: view };
  }
  return {
    kind: "no_access",
    reason: "No account is available to your identity.",
  };
}

export function roleOf(result: SessionResult): Role | null {
  if (result.kind === "allowed" && result.data.state === "signed_in") {
    return result.data.role;
  }
  return null;
}
