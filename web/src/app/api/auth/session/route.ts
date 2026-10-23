// Session endpoint for the browser. It returns only the account view (state,
// role, display label). The HttpOnly session cookie is read on the server; no
// token, subject, email, OIDC credential, or database URL crosses to the
// browser.

import { NextResponse } from "next/server";

import { clearedSessionCookies } from "@/lib/proxy-cookies";
import { resolveSession } from "@/lib/session";

export const dynamic = "force-dynamic";

export async function GET() {
  const result = await resolveSession();
  // Map the discriminated result onto the flat session shape the client reads.
  if (result.kind === "allowed") {
    return NextResponse.json(result.data, {
      status: 200,
      headers: { "cache-control": "no-store" },
    });
  }
  if (result.kind === "expired") {
    // The token inside the cookie was rejected. Clear the stale browser session
    // on this legal cookie-modifying response so the next request is anonymous
    // rather than repeatedly forwarding an expired credential.
    const response = NextResponse.json(
      { error: "expired" },
      { status: 401, headers: { "cache-control": "no-store" } },
    );
    for (const cookie of clearedSessionCookies()) {
      response.cookies.set(cookie);
    }
    return response;
  }
  return NextResponse.json(
    { state: "anonymous" },
    { status: 200, headers: { "cache-control": "no-store" } },
  );
}
