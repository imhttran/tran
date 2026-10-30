import { NextRequest, NextResponse } from "next/server";

import { INVITE_COOKIE, transientCookie } from "@/lib/proxy-cookies";
import { requestOrigin } from "@/lib/oidc";

export const dynamic = "force-dynamic";

// Holds a shared invitation across sign-in. The /activate page posts the token
// and the member's name here, then starts the normal sign-in; the callback
// redeems it. The cookie is HttpOnly, short-lived, and scoped to /api/auth, so
// the token never reaches the identity provider's URLs or client JavaScript.
//
// This runs before sign-in, so it only accepts a same-origin JSON request: a
// cross-site page cannot plant someone else's invitation in this browser.
export async function POST(request: NextRequest) {
  const origin = requestOrigin(request.headers, request.nextUrl);
  if (
    request.headers.get("origin") !== origin ||
    !request.headers.get("content-type")?.startsWith("application/json")
  ) {
    return NextResponse.json({ ok: false, error: "forbidden" }, { status: 403 });
  }
  const body = (await request.json().catch(() => null)) as { token?: unknown; name?: unknown } | null;
  const token = typeof body?.token === "string" ? body.token.trim() : "";
  const name = typeof body?.name === "string" ? body.name.trim() : "";
  if (!/^[0-9a-f]{64}$/.test(token)) {
    return NextResponse.json({ ok: false, error: "invalid_invitation" }, { status: 400 });
  }
  if (name.length === 0 || [...name].length > 100) {
    return NextResponse.json({ ok: false, error: "invalid_name" }, { status: 400 });
  }
  const response = NextResponse.json({ ok: true });
  response.cookies.set(
    transientCookie(INVITE_COOKIE, Buffer.from(JSON.stringify({ token, name })).toString("base64url")),
  );
  return response;
}
