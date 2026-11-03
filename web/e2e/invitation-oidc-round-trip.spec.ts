import { expect, test } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

// PIR-009: the real OIDC authorization-code round trip for a brand-new invitee.
//
// This is the case application-level provisioning alone cannot prove: the
// identity provider issues a token for a subject the application has never seen,
// the browser completes the login redirect, the callback provisions the account,
// the invitation survives the round trip, and acceptance reaches pending review.
// The test also proves the account has no archive access until approval.

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(
  /\/+$/,
  "",
);
const ISSUER = (process.env.OIDC_ISSUER || "http://127.0.0.1:9099").replace(
  /\/+$/,
  "",
);
const ADMIN = process.env.E2E_ADMIN_TOKEN ?? "";
const INVITEE = process.env.E2E_OIDC_INVITEE_TOKEN ?? "";
const INVITATION = process.env.E2E_OIDC_INVITATION_TOKEN ?? "";
const CORPUS = process.env.E2E_CORPUS ?? "";
const SUBJECT = "e2e-oidc-invitee";
const PERSON_KEY = "e2e-oidc-invitee-person";

const READY = Boolean(ADMIN && INVITEE && INVITATION && CORPUS);

type ApiResult = { status: number; body: Record<string, unknown> | null };

async function api(
  request: APIRequestContext,
  method: "GET" | "POST",
  path: string,
  token?: string,
  data?: unknown,
): Promise<ApiResult> {
  const headers: Record<string, string> = {};
  if (token) headers.authorization = `Bearer ${token}`;
  if (data !== undefined) headers["content-type"] = "application/json";
  const response = await request.fetch(`${API_URL}${path}`, {
    method,
    headers,
    data,
  });
  let body: Record<string, unknown> | null = null;
  try {
    body = (await response.json()) as Record<string, unknown>;
  } catch {
    body = null;
  }
  return { status: response.status(), body };
}

test.describe.serial("invitation OIDC round trip", () => {
  test.skip(!READY, "OIDC invitation E2E fixture not provided by the harness");

  test.beforeAll(async ({ request }) => {
    // The issuer is the authority for who the browser signs in as. Point it at a
    // subject the application has never provisioned.
    const response = await request.post(`${ISSUER}/test/subject`, {
      data: { subject: SUBJECT },
    });
    expect(response.ok()).toBeTruthy();
  });

  test.afterAll(async ({ request }) => {
    // Leave the issuer as the other specs expect it.
    await request.post(`${ISSUER}/test/subject`, {
      data: { subject: "e2e-admin" },
    });
  });

  test("a person with no application account signs in and reaches pending review", async ({
    page,
    request,
  }) => {
    // No application account exists for this identity yet.
    const before = await api(request, "GET", "/api/account", INVITEE);
    expect(before.status).toBe(403);

    await page.goto(`/activate/${INVITATION}`);
    await expect(page.getByTestId("activation-welcome")).toBeVisible();
    await expect(page.getByTestId("activation-signin")).toBeVisible();

    // Complete the real redirect chain: login -> issuer -> callback -> welcome.
    await page.getByTestId("activation-signin").click();
    await page.waitForURL(new RegExp(`/activate/${INVITATION}$`), {
      timeout: 30_000,
    });

    // The callback provisioned the account and established the session, and the
    // invitation survived the round trip.
    await expect(page.getByTestId("activation-accept")).toBeVisible();
    await expect(page.getByTestId("activation-person")).toContainText(
      "Synthetic OIDC Invitee Person",
    );

    await page.getByTestId("activation-accept").click();
    await expect(page.getByTestId("activation-pending-review")).toBeVisible();

    // The account exists, but has no role and no archive access before approval.
    const account = await api(request, "GET", "/api/account", INVITEE);
    expect(account.status).toBe(200);
    expect(account.body?.roles).toEqual([]);
    expect(account.body?.corpus_grants).toEqual([]);
    const treeBefore = await api(request, "GET", "/api/genealogy/tree", INVITEE);
    expect(treeBefore.status).toBe(403);

    // An approved link grants exactly the member access.
    const list = await api(
      request,
      "GET",
      "/api/admin/family-members/link-requests",
      ADMIN,
    );
    const requests = list.body?.requests as { request_id: number; person_key: string }[];
    const entry = requests.find((item) => item.person_key === PERSON_KEY);
    expect(entry, "the pending request is listed").toBeTruthy();
    const decided = await api(
      request,
      "POST",
      `/api/admin/family-members/link-requests/${entry!.request_id}/decision`,
      ADMIN,
      { decision: "approve" },
    );
    expect(decided.status).toBe(200);
    expect(decided.body?.linked).toBe(true);

    const after = await api(request, "GET", "/api/account", INVITEE);
    const roles = after.body?.roles as string[];
    expect(roles).toContain("viewer");
    expect(roles).not.toContain("admin");
    const grants = after.body?.corpus_grants as string[];
    expect(grants).toContain(CORPUS);
    const treeAfter = await api(request, "GET", "/api/genealogy/tree", INVITEE);
    expect(treeAfter.status).toBe(200);
  });
});
