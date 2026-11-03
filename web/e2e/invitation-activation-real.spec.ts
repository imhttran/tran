import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// PIR-008: the invitation flow against the real application APIs.
//
// Unlike the stubbed panel suites, this spec starts from a seeded, unprovisioned
// invitee and a real pending invitation. It exercises provisioning, the pending
// link request, the administrative review, and the resulting family-profile
// access through the running Axum API and PostgreSQL. It never sends email.

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(
  /\/+$/,
  "",
);
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ADMIN = process.env.E2E_ADMIN_TOKEN ?? "";
const INVITEE = process.env.E2E_INVITEE_TOKEN ?? "";
const INVITEE_TWO = process.env.E2E_INVITEE_TWO_TOKEN ?? "";
const INVITATION = process.env.E2E_INVITATION_TOKEN ?? "";
const INVITATION_TWO = process.env.E2E_INVITATION_TWO_TOKEN ?? "";
const CORPUS = process.env.E2E_CORPUS ?? "";
const PERSON_KEY = "e2e-invitee-person";
const PERSON_TWO_KEY = "e2e-invitee-person-2";

const READY = Boolean(
  ADMIN && INVITEE && INVITEE_TWO && INVITATION && INVITATION_TWO && CORPUS,
);

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

async function useSession(page: Page, token: string) {
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

test.describe.serial("invitation activation against the real API", () => {
  test.skip(!READY, "invitation E2E fixture not provided by the harness");

  test("the welcome link previews the intended person", async ({ request }) => {
    const preview = await api(request, "GET", `/api/invitations/${INVITATION}`);
    expect(preview.status).toBe(200);
    expect(preview.body?.state).toBe("pending");
    expect(preview.body?.request_state).toBe("none");
    expect(preview.body?.display_name).toContain("Synthetic Invitee Person");
    // Provisioning is authorized by the verified identity plus the link.
    const unauth = await api(
      request,
      "POST",
      `/api/invitations/${INVITATION}/provision`,
    );
    expect(unauth.status).toBe(401);
  });

  test("signing in creates an account without archive access", async ({
    request,
  }) => {
    const provisioned = await api(
      request,
      "POST",
      `/api/invitations/${INVITATION}/provision`,
      INVITEE,
    );
    expect(provisioned.status).toBe(200);
    expect(provisioned.body?.provisioned).toBe(true);

    const account = await api(request, "GET", "/api/account", INVITEE);
    expect(account.status).toBe(200);
    // Signing in creates the account; access follows the approved link.
    expect(account.body?.roles).toEqual([]);
    expect(account.body?.corpus_grants).toEqual([]);
    const tree = await api(request, "GET", "/api/genealogy/tree", INVITEE);
    expect(tree.status).toBe(403);
  });

  test("accepting records a link request instead of approving it", async ({
    request,
  }) => {
    const accepted = await api(
      request,
      "POST",
      `/api/invitations/${INVITATION}/accept`,
      INVITEE,
    );
    expect(accepted.status).toBe(200);
    expect(accepted.body?.link_state).toBe("pending_review");
    expect(accepted.body?.state).toBe("accepted");

    // The person is not linked yet, so the family profile stays closed.
    const profile = await api(
      request,
      "GET",
      `/api/family/persons/${PERSON_KEY}/profile`,
      INVITEE,
    );
    expect(profile.status).toBe(403);
  });

  test("an administrator approves the reviewed link", async ({ request }) => {
    const list = await api(
      request,
      "GET",
      "/api/admin/family-members/link-requests",
      ADMIN,
    );
    expect(list.status).toBe(200);
    const requests = list.body?.requests as {
      request_id: number;
      person_key: string;
    }[];
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

    const profile = await api(
      request,
      "GET",
      `/api/family/persons/${PERSON_KEY}/profile`,
      INVITEE,
    );
    expect(profile.status).toBe(200);

    // Approval is what grants the member access, not the invitation token.
    const account = await api(request, "GET", "/api/account", INVITEE);
    const roles = account.body?.roles as string[];
    expect(roles).toContain("viewer");
    expect(roles).not.toContain("admin");
    const grants = account.body?.corpus_grants as string[];
    expect(grants).toContain(CORPUS);
    const tree = await api(request, "GET", "/api/genealogy/tree", INVITEE);
    expect(tree.status).toBe(200);
  });

  test("the browser accepts a second invitation and shows pending review", async ({
    page,
    request,
  }) => {
    // A second invitee proves the flow from the page itself. Provision first, so
    // the session established by the cookie can reach the API through the proxy.
    const provisioned = await api(
      request,
      "POST",
      `/api/invitations/${INVITATION_TWO}/provision`,
      INVITEE_TWO,
    );
    expect(provisioned.status).toBe(200);

    await useSession(page, INVITEE_TWO);
    await page.goto(`/activate/${INVITATION_TWO}`);

    await expect(page.getByTestId("activation-welcome")).toBeVisible();
    await expect(page.getByTestId("activation-person")).toContainText(
      "Synthetic Invitee Person Two",
    );
    await page.getByTestId("activation-accept").click();

    await expect(page.getByTestId("activation-pending-review")).toBeVisible();
    await expect(page.getByTestId("activation-pending-review")).toContainText(
      "Synthetic Invitee Person Two",
    );
    await expect(page.getByTestId("activation-open-profile")).toHaveCount(0);

    // The administrator's decision in the real console is the only path to the
    // person. Switch the browser session to the administrator.
    await useSession(page, ADMIN);
    await page.goto("/admin/family-members");
    const section = page.getByTestId("link-reviews");
    await expect(section).toBeVisible();
    const row = section.locator("li").filter({ hasText: PERSON_TWO_KEY });
    await expect(row).toBeVisible();
    await row.getByRole("button", { name: "Approve link" }).click();
    await expect(page.getByTestId("link-review-notice")).toContainText(
      "Link approved",
    );

    // The invitee reloads the same link and now sees the completed flow.
    await useSession(page, INVITEE_TWO);
    await page.goto(`/activate/${INVITATION_TWO}`);
    await expect(page.getByTestId("activation-accepted")).toBeVisible();
    await expect(page.getByTestId("activation-open-profile")).toHaveAttribute(
      "href",
      `/family/${PERSON_TWO_KEY}`,
    );

    // The console approval is what opened archive access.
    const tree = await api(request, "GET", "/api/genealogy/tree", INVITEE_TWO);
    expect(tree.status).toBe(200);
  });
});
