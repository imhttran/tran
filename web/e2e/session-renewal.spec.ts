import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// Automatic browser-session renewal against the real Next.js proxy, the real
// Rust session store, and the synthetic OIDC provider (extended with refresh
// tokens and rotation). Each test seeds a short-lived session through the
// server-to-server internal API, so the browser only ever holds the opaque
// `rag_sid` and the current ID token.

const ORIGIN = (process.env.WEB_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
const API = (process.env.E2E_API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
const ISSUER = (process.env.E2E_ISSUER || "").replace(/\/+$/, "");
const SECRET = process.env.WEB_SESSION_SECRET || "";

type Minted = { id_token: string; refresh_token: string };

function configured(): boolean {
  return Boolean(SECRET && ISSUER);
}

async function mint(
  request: APIRequestContext,
  ttl: number,
): Promise<Minted> {
  const response = await request.post(`${ISSUER}/test/mint`, {
    data: { subject: "e2e-admin", ttl },
  });
  expect(response.ok(), "mint a test token").toBeTruthy();
  return (await response.json()) as Minted;
}

async function createSession(
  request: APIRequestContext,
  sessionId: string,
  minted: Minted,
): Promise<void> {
  const response = await request.post(`${API}/api/internal/sessions`, {
    headers: { "x-internal-secret": SECRET },
    data: {
      session_id: sessionId,
      id_token: minted.id_token,
      refresh_token: minted.refresh_token,
    },
  });
  expect(response.ok(), "create a stored session").toBeTruthy();
}

async function useSession(
  page: Page,
  sessionId: string,
  minted: Minted,
): Promise<void> {
  await page.context().addCookies([
    {
      name: "rag_session",
      value: minted.id_token,
      url: ORIGIN,
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_sid",
      value: sessionId,
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

async function refreshCalls(request: APIRequestContext): Promise<number> {
  const response = await request.get(`${ISSUER}/test/stats`);
  expect(response.ok()).toBeTruthy();
  const body = (await response.json()) as { refresh_calls?: number };
  return body.refresh_calls ?? 0;
}

function sessionId(): string {
  return `e2e-session-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

test.describe("automatic session renewal", () => {
  test.skip(() => !configured(), "session renewal is not configured in this harness");

  test("a short-lived token is renewed and the session stays signed in", async ({
    request,
    page,
  }) => {
    const minted = await mint(request, 10);
    const id = sessionId();
    await createSession(request, id, minted);
    await useSession(page, id, minted);

    const before = (await page.context().cookies(ORIGIN)).find(
      (cookie) => cookie.name === "rag_session",
    )?.value;

    await page.goto("/");
    await expect(page.getByTestId("account-role")).toHaveText("admin");

    const cookies = await page.context().cookies(ORIGIN);
    const after = cookies.find((cookie) => cookie.name === "rag_session")?.value;
    expect(after, "the ID token must be renewed").not.toBe(before);
    expect(
      cookies.some((cookie) => cookie.name === "rag_sid"),
      "the opaque session id must be kept",
    ).toBe(true);
  });

  test("concurrent requests trigger exactly one provider refresh", async ({
    request,
    page,
  }) => {
    const minted = await mint(request, 10);
    const id = sessionId();
    await createSession(request, id, minted);
    await useSession(page, id, minted);
    // The sign-in page does not read the session, so the first renewal happens
    // inside the two concurrent API calls.
    await page.goto("/signin");
    const before = await refreshCalls(request);

    const statuses = await page.evaluate(async () => {
      const responses = await Promise.all([
        fetch("/api/account", { cache: "no-store" }),
        fetch("/api/account", { cache: "no-store" }),
      ]);
      return responses.map((response) => response.status);
    });
    expect(statuses).toEqual([200, 200]);

    const after = await refreshCalls(request);
    expect(after - before, "one refresh for two concurrent requests").toBe(1);
  });

  test("a revoked session is cleared and requires sign-in", async ({
    request,
    page,
  }) => {
    // A token inside the refresh window forces the revocation check; a
    // still-valid token would otherwise keep working until it expires.
    const minted = await mint(request, 10);
    const id = sessionId();
    await createSession(request, id, minted);
    await useSession(page, id, minted);
    const revoked = await request.delete(
      `${API}/api/internal/sessions/${encodeURIComponent(id)}`,
      { headers: { "x-internal-secret": SECRET } },
    );
    expect(revoked.ok(), "revoke the stored session").toBeTruthy();

    await page.goto("/");
    await expect(page.getByTestId("session-expired")).toBeVisible();
    const cookies = await page.context().cookies(ORIGIN);
    expect(cookies.some((cookie) => cookie.name === "rag_session")).toBe(false);
    expect(cookies.some((cookie) => cookie.name === "rag_sid")).toBe(false);
  });

  test("a provider outage keeps the session and is not treated as expiry", async ({
    request,
    page,
  }) => {
    const minted = await mint(request, 10);
    const id = sessionId();
    await createSession(request, id, minted);
    await useSession(page, id, minted);

    const failing = await request.post(`${ISSUER}/test/refresh-failure`, {
      data: { enabled: true },
    });
    expect(failing.ok()).toBeTruthy();
    await page.goto("/");
    await expect(page.getByTestId("session-expired")).toHaveCount(0);
    expect(
      (await page.context().cookies(ORIGIN)).some(
        (cookie) => cookie.name === "rag_sid",
      ),
      "a transient failure must not delete the session",
    ).toBe(true);

    const recovered = await request.post(`${ISSUER}/test/refresh-failure`, {
      data: { enabled: false },
    });
    expect(recovered.ok()).toBeTruthy();
    await page.goto("/");
    await expect(page.getByTestId("account-role")).toHaveText("admin");
  });

  test("sign-out revokes the stored session", async ({ request, page }) => {
    const minted = await mint(request, 600);
    const id = sessionId();
    await createSession(request, id, minted);
    await useSession(page, id, minted);

    await page.goto("/");
    await page.getByTestId("sign-out-button").click();
    await expect(page.getByTestId("sign-in-button")).toBeVisible();

    const renew = await request.post(
      `${API}/api/internal/sessions/${encodeURIComponent(id)}/renew`,
      { headers: { "x-internal-secret": SECRET } },
    );
    expect(renew.status(), "a signed-out session cannot renew").toBe(401);
  });
});
