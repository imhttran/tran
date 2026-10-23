import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import {
  SESSION_MAX_SECONDS,
  clearedSessionCookies,
  csrfCookie,
  sessionCookie,
  sessionLifetimeSeconds,
} from "../src/lib/proxy-cookies";
import {
  markSessionExpired,
  noteAuthStatus,
  onSessionExpired,
  resetSessionExpired,
  sessionIsExpired,
} from "../src/lib/session-expiry";

// Expired browser-session handling.
//
// The lifetime and client-coordination tests run in the Playwright worker on a
// controlled clock, so they need no server. The browser tests drive the real
// Next.js proxy and Rust API started by scripts/web-e2e.sh.

const ORIGIN = process.env.WEB_URL || "http://127.0.0.1:3000";

async function useToken(page: Page, token: string) {
  await page.context().addCookies([
    {
      name: "rag_session",
      value: token,
      url: ORIGIN,
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_csrf",
      value: "e2e-csrf-token",
      url: ORIGIN,
      sameSite: "Lax",
    },
  ]);
}

test.describe("session cookie lifetime", () => {
  test("is bounded by the token's remaining lifetime", () => {
    const now = 1_800_000_000;
    expect(sessionLifetimeSeconds(now + 120, now)).toBe(120);
    expect(sessionCookie("token", 120).maxAge).toBe(120);
    expect(csrfCookie("csrf", 120).maxAge).toBe(120);
  });

  test("never exceeds the maximum session lifetime", () => {
    const now = 1_800_000_000;
    expect(sessionLifetimeSeconds(now + SESSION_MAX_SECONDS * 10, now)).toBe(
      SESSION_MAX_SECONDS,
    );
    expect(sessionCookie("token", SESSION_MAX_SECONDS * 10).maxAge).toBe(
      SESSION_MAX_SECONDS,
    );
    expect(csrfCookie("csrf", SESSION_MAX_SECONDS * 10).maxAge).toBe(
      SESSION_MAX_SECONDS,
    );
  });

  test("an already-expired token yields no session lifetime", () => {
    const now = 1_800_000_000;
    expect(sessionLifetimeSeconds(now, now)).toBe(0);
    expect(sessionLifetimeSeconds(now - 1, now)).toBe(0);
    expect(sessionLifetimeSeconds(Number.NaN, now)).toBe(0);
    expect(sessionCookie("token", 0).maxAge).toBe(0);
    expect(csrfCookie("csrf", -5).maxAge).toBe(0);
  });

  test("keeps HttpOnly, SameSite=Lax, and root path attributes", () => {
    const session = sessionCookie("token", 60);
    expect(session.httpOnly).toBe(true);
    expect(session.sameSite).toBe("lax");
    expect(session.path).toBe("/");
    // The CSRF cookie is intentionally readable by JavaScript.
    expect(csrfCookie("csrf", 60).httpOnly).toBe(false);
  });

  test("clears the session cookies with matching attributes", () => {
    const cleared = clearedSessionCookies();
    expect(cleared.map((cookie) => cookie.name)).toEqual([
      "rag_session",
      "rag_sid",
      "rag_csrf",
    ]);
    for (const cookie of cleared) {
      expect(cookie.value).toBe("");
      expect(cookie.maxAge).toBe(0);
      expect(cookie.path).toBe("/");
      expect(cookie.sameSite).toBe("lax");
    }
  });
});

test.describe("client expiry coordination", () => {
  test.beforeEach(() => resetSessionExpired());

  test("concurrent 401 responses notify exactly once", () => {
    let notified = 0;
    const unsubscribe = onSessionExpired(() => {
      notified += 1;
    });
    // Three concurrent requests fail with an expired credential.
    noteAuthStatus(401);
    noteAuthStatus(401);
    noteAuthStatus(401);
    expect(notified).toBe(1);
    expect(sessionIsExpired()).toBe(true);
    unsubscribe();
  });

  test("403, 5xx, and network failures do not expire the session", () => {
    noteAuthStatus(403);
    noteAuthStatus(500);
    noteAuthStatus(503);
    noteAuthStatus(0);
    expect(sessionIsExpired()).toBe(false);
  });

  test("a fresh signed-in session clears the expired state", () => {
    markSessionExpired();
    expect(sessionIsExpired()).toBe(true);
    resetSessionExpired();
    expect(sessionIsExpired()).toBe(false);
    // A later 401 marks it again.
    noteAuthStatus(401);
    expect(sessionIsExpired()).toBe(true);
  });
});

test.describe("browser session expiry", () => {
  test("an expired session clears cookies and offers sign-in", async ({
    page,
  }) => {
    await useToken(page, "expired-or-invalid-token");
    await page.goto("/profile");
    await expect(page.getByTestId("session-expired")).toBeVisible();
    await expect(page.getByTestId("sign-in-button")).toBeVisible();
    const cookies = await page.context().cookies(ORIGIN);
    expect(cookies.some((cookie) => cookie.name === "rag_session")).toBe(false);
    expect(cookies.some((cookie) => cookie.name === "rag_csrf")).toBe(false);
  });

  test("a permission denial keeps the session and does not claim expiry", async ({
    page,
  }) => {
    await useToken(page, "synthetic-session-token");
    await page.route("**/api/auth/session", (route) =>
      route.fulfill({
        status: 403,
        contentType: "application/json",
        body: JSON.stringify({ error: "forbidden" }),
      }),
    );
    await page.goto("/");
    await expect(page.getByTestId("sign-in-button")).toBeVisible();
    await expect(page.getByTestId("session-expired")).toHaveCount(0);
    const cookies = await page.context().cookies(ORIGIN);
    expect(cookies.some((cookie) => cookie.name === "rag_session")).toBe(true);
  });

  test("a session-service failure keeps the session and does not claim expiry", async ({
    page,
  }) => {
    await useToken(page, "synthetic-session-token");
    await page.route("**/api/auth/session", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ error: "unavailable" }),
      }),
    );
    await page.goto("/");
    await expect(page.getByTestId("sign-in-button")).toBeVisible();
    await expect(page.getByTestId("session-expired")).toHaveCount(0);
    const cookies = await page.context().cookies(ORIGIN);
    expect(cookies.some((cookie) => cookie.name === "rag_session")).toBe(true);
  });

  test("a fresh login restores normal operation", async ({ page }) => {
    await useToken(page, "expired-or-invalid-token");
    await page.goto("/profile");
    await expect(page.getByTestId("session-expired")).toBeVisible();

    await page.goto("/signin");
    await page.getByTestId("signin-button").click();
    await page.waitForURL(/127\.0\.0\.1:3000\/$/, { timeout: 120_000 });

    await expect(page.getByTestId("account-role")).toHaveText("admin");
    await expect(page.getByTestId("session-expired")).toHaveCount(0);
    await expect(page.getByTestId("sign-out-button")).toBeVisible();
  });
});
