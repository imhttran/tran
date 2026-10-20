import { NextResponse } from "next/server";

import { CSRF_COOKIE, SESSION_COOKIE } from "@/lib/proxy-cookies";

// Clears the browser session. The authoritative session is the HttpOnly cookie;
// the CSRF cookie is cleared with it. This route is intentionally outside the
// catch-all proxy so sign-out never depends on the Rust API being reachable.
export async function POST() {
  const response = NextResponse.json({ ok: true });
  response.cookies.set(SESSION_COOKIE, "", {
    httpOnly: true,
    secure: true,
    sameSite: "strict",
    path: "/",
    maxAge: 0,
  });
  response.cookies.set(CSRF_COOKIE, "", {
    secure: true,
    sameSite: "strict",
    path: "/",
    maxAge: 0,
  });
  return response;
}
