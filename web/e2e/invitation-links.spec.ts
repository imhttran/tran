import { expect, test } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

// Shareable invitation links against the real Axum API, the real Next.js
// proxy and sign-in callback, and the synthetic OIDC issuer started by
// scripts/web-e2e.sh. `e2e-invitee` is a login the app has never seen; the
// person `synthetic-invitee` is seeded unlinked. No email is involved.

const API_URL = process.env.E2E_API_URL || process.env.API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000").origin;
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const INVITEE = process.env.E2E_INVITEE_TOKEN || "";
const READY = Boolean(ADMIN && INVITEE);

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

const auth = (token: string) => ({ authorization: `Bearer ${token}` });

async function shareLink(request: APIRequestContext, personKey: string) {
  const response = await request.post(`${API_URL}/api/admin/family-members/invitation-links`, {
    headers: auth(ADMIN),
    data: { person_key: personKey },
  });
  expect(response.status()).toBe(200);
  const body = (await response.json()) as { url: string };
  return new URL(body.url, ORIGIN);
}

async function openClaims(request: APIRequestContext) {
  const response = await request.get(`${API_URL}/api/admin/family-members/claims`, {
    headers: auth(ADMIN),
  });
  expect(response.status()).toBe(200);
  return ((await response.json()) as {
    claims: { invitation_id: number; person_key: string; claimed_name: string }[];
  }).claims;
}

test.describe.serial("shareable invitation links", () => {
  test("a new login redeems a link, waits for confirmation, then gets access", async ({
    request,
  }) => {
    const link = await shareLink(request, "synthetic-invitee");
    const token = link.searchParams.get("token") || "";
    expect(token).toMatch(/^[0-9a-f]{64}$/);

    // Before redeeming, the unknown login has no access at all.
    expect((await request.get(`${API_URL}/api/account`, { headers: auth(INVITEE) })).status()).toBe(403);

    const redeemed = await request.post(`${API_URL}/api/invitations/redeem`, {
      headers: auth(INVITEE),
      data: { token, name: "Synthetic Invitee Member" },
    });
    expect(redeemed.status()).toBe(200);

    // A claim is not access: the login now exists but holds no role, so it
    // can read only its own (empty) account, not the family tree.
    const pending = await request.get(`${API_URL}/api/account`, { headers: auth(INVITEE) });
    expect(((await pending.json()) as { roles: string[] }).roles).toEqual([]);
    expect((await request.get(`${API_URL}/api/genealogy/tree`, { headers: auth(INVITEE) })).status()).toBe(403);
    const unlinked = await request.get(`${API_URL}/api/profile`, { headers: auth(INVITEE) });
    expect(((await unlinked.json()) as { linked: boolean }).linked).toBe(false);

    // The link works once.
    const again = await request.post(`${API_URL}/api/invitations/redeem`, {
      headers: auth(INVITEE),
      data: { token, name: "Synthetic Invitee Member" },
    });
    expect(again.status()).toBe(409);

    const claim = (await openClaims(request)).find((item) => item.person_key === "synthetic-invitee");
    expect(claim?.claimed_name).toBe("Synthetic Invitee Member");

    // A non-admin cannot decide claims.
    const forbidden = await request.post(
      `${API_URL}/api/admin/family-members/claims/${claim!.invitation_id}`,
      { headers: auth(INVITEE), data: { decision: "approve" } },
    );
    expect(forbidden.status()).toBe(403);

    const confirmed = await request.post(
      `${API_URL}/api/admin/family-members/claims/${claim!.invitation_id}`,
      { headers: auth(ADMIN), data: { decision: "approve" } },
    );
    expect(confirmed.status()).toBe(200);
    expect(((await confirmed.json()) as { linked: boolean }).linked).toBe(true);

    // Confirmed: the member can sign in as a viewer and sees their own profile.
    const account = await request.get(`${API_URL}/api/account`, { headers: auth(INVITEE) });
    expect(((await account.json()) as { roles: string[] }).roles).toEqual(["viewer"]);
    expect((await request.get(`${API_URL}/api/genealogy/tree`, { headers: auth(INVITEE) })).status()).toBe(200);
    const profile = await request.get(`${API_URL}/api/profile`, { headers: auth(INVITEE) });
    expect(((await profile.json()) as { linked: boolean }).linked).toBe(true);
    expect((await openClaims(request)).some((item) => item.invitation_id === claim!.invitation_id)).toBe(false);

    // A linked person cannot be invited again.
    const relink = await request.post(`${API_URL}/api/admin/family-members/invitation-links`, {
      headers: auth(ADMIN),
      data: { person_key: "synthetic-invitee" },
    });
    expect(relink.status()).toBe(409);
  });

  test("redeeming needs a verified login", async ({ request }) => {
    const response = await request.post(`${API_URL}/api/invitations/redeem`, {
      data: { token: "0".repeat(64), name: "Nobody" },
    });
    expect(response.status()).toBe(401);
  });

  test("another site cannot plant an invitation in this browser", async ({ request }) => {
    const response = await request.post(`${ORIGIN}/api/auth/activate`, {
      headers: { origin: "https://elsewhere.example", "content-type": "application/json" },
      data: { token: "a".repeat(64), name: "Someone" },
    });
    expect(response.status()).toBe(403);
  });

  test("the activate page hides the token, then reports the sign-in result", async ({
    page,
    request,
  }) => {
    // The synthetic issuer always signs in as e2e-admin, so the outcome is a
    // real redemption by that login; any claim it makes is rejected below.
    const link = await shareLink(request, "synthetic-e2e-person");
    await page.goto(link.pathname + link.search);
    await expect(page.getByTestId("activate-form")).toBeVisible();
    expect(page.url()).not.toContain("token=");

    await page.getByTestId("activate-name").fill("Synthetic Browser Member");
    await page.getByTestId("activate-continue").click();
    await page.waitForURL(/\/activate\?result=/, { timeout: 30_000 });
    expect(page.url()).not.toContain("token=");
    await expect(page.locator("[data-testid^=activate-result-]")).toBeVisible();

    for (const claim of await openClaims(request)) {
      if (claim.person_key === "synthetic-e2e-person") {
        await request.post(`${API_URL}/api/admin/family-members/claims/${claim.invitation_id}`, {
          headers: auth(ADMIN),
          data: { decision: "reject" },
        });
      }
    }
  });

  test("an admin creates and copies a link from the admin screen", async ({ page }) => {
    await page.context().addCookies([
      { name: "rag_session", value: ADMIN, url: ORIGIN, httpOnly: true, sameSite: "Lax" },
      { name: "rag_csrf", value: "e2e-csrf-token", url: ORIGIN, sameSite: "Lax" },
    ]);
    await page.goto("/admin/family-members");
    const section = page.getByTestId("invitation-links");
    await expect(section).toBeVisible();
    await section.getByRole("combobox").fill("Synthetic E2E");
    await section.getByRole("option", { name: /Synthetic E2E Person/ }).first().click();
    page.once("dialog", (dialog) => dialog.accept());
    await section.getByTestId("invite-link-create").click();
    await expect(section.getByTestId("invite-link-url")).toHaveValue(/\/activate\?token=[0-9a-f]{64}$/);
  });
});
