import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-005C: the /branch-review interface.
//
// These run against the real Next.js proxy, the real Axum API, and disposable
// PostgreSQL fixtures seeded by scripts/web-e2e.sh. Nothing is mocked: if the
// server refuses a branch, the page cannot show it. The full negative security
// matrix is USR-005D.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const DEACTIVATED = process.env.E2E_DEACTIVATED_TOKEN || "";
const REVIEWER_ACCOUNT_ID = Number(process.env.E2E_REVIEWER_ACCOUNT_ID || 0);
const APPROVER_ACCOUNT_ID = Number(process.env.E2E_APPROVER_ACCOUNT_ID || 0);
const READY = Boolean(
  ISSUER &&
    ADMIN &&
    APPROVER &&
    REVIEWER &&
    DEACTIVATED &&
    REVIEWER_ACCOUNT_ID &&
    APPROVER_ACCOUNT_ID,
);

type Grant = {
  grant_id: number;
  branch_key: string;
  active: boolean;
  usable: boolean;
};

type Evidence = {
  source_evidence_id: number;
  document_ref: string;
  page: number;
};

// Unique per run: one open proposal per fact is enforced by the database.
const RUN = `${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6).toString(36)}`;

function auth(token: string) {
  return { authorization: `Bearer ${token}` };
}

async function api(
  request: APIRequestContext,
  token: string,
  method: "GET" | "POST",
  path: string,
  data?: unknown,
) {
  const response = await request.fetch(`${API_URL}${path}`, {
    method,
    headers: auth(token),
    data,
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

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

async function replaceGrants(
  request: APIRequestContext,
  accountId: number,
  subject: string,
  branchKeys: string[],
) {
  const listed = await api(
    request,
    ADMIN,
    "GET",
    `/api/admin/branch-reviewers?account_id=${accountId}`,
  );
  expect(listed.status, JSON.stringify(listed.body)).toBe(200);
  for (const grant of listed.body.grants as Grant[]) {
    if (!grant.active) continue;
    const revoked = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/branch-reviewers/${grant.grant_id}/revoke`,
      { reason: "E2E reset before granting" },
    );
    expect(revoked.status, JSON.stringify(revoked.body)).toBe(200);
  }
  if (branchKeys.length === 0) return;
  const granted = await api(request, ADMIN, "POST", "/api/admin/branch-reviewers", {
    issuer: ISSUER,
    subject,
    branch_keys: branchKeys,
    reason: "Synthetic E2E branch review appointment",
  });
  expect(granted.status, JSON.stringify(granted.body)).toBe(200);
}

async function firstEvidence(
  request: APIRequestContext,
  token: string,
  personKey: string,
): Promise<Evidence> {
  const response = await api(
    request,
    token,
    "GET",
    `/api/branch-review/persons/${personKey}/evidence`,
  );
  expect(response.status, JSON.stringify(response.body)).toBe(200);
  const evidence = response.body.evidence as Evidence[];
  expect(evidence.length).toBeGreaterThan(0);
  return evidence[0];
}

async function propose(
  request: APIRequestContext,
  token: string,
  options: {
    personKey: string;
    branchKey: string;
    assertionKey: string;
    value: string;
  },
): Promise<number> {
  const evidence = await firstEvidence(request, token, options.personKey);
  const created = await api(request, token, "POST", "/api/branch-review/submissions", {
    person_key: options.personKey,
    branch_key: options.branchKey,
    assertion_kind: "name",
    assertion_key: options.assertionKey,
    proposed_value: options.value,
    evidence: [evidence],
    note: "Synthetic E2E shared-fact proposal",
  });
  expect(created.status, JSON.stringify(created.body)).toBe(200);
  return created.body.proposal_id as number;
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("branch review interface", () => {
  test("a granted reviewer reviews and approves a correction in the interface", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
    const value = `Synthetic UI Approved Name ${RUN}`;
    const proposalId = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey: `name.ui-approve.${RUN}`,
      value,
    });

    await useToken(page, REVIEWER);
    await page.goto("/branch-review");

    // The console renders only after the grants call succeeds, and the shared
    // navigation offers the route.
    await expect(page.getByTestId("branch-review-console")).toBeVisible();
    await expect(
      page.getByTestId("site-nav").getByRole("link", { name: /Branch review/ }),
    ).toBeVisible();
    await expect(
      page.getByTestId("branch-review-branch-filter"),
    ).toContainText("First Branch");

    const row = page.getByTestId(`branch-review-row-${proposalId}`);
    await expect(row).toBeVisible();
    await row.click();

    const detail = page.getByTestId(`branch-review-detail-${proposalId}`);
    await expect(detail).toBeVisible();
    await expect(page.getByTestId("branch-review-proposed")).toContainText(
      value,
    );
    await expect(
      page.getByTestId("branch-review-proposal-evidence"),
    ).toBeVisible();

    // Approval is a two-step action with a confirmation.
    await page.getByTestId("branch-review-approve").click();
    await expect(page.getByTestId("branch-review-confirm")).toBeVisible();
    await page.getByTestId("branch-review-confirm-button").click();

    await expect(page.getByTestId("branch-review-message")).toContainText(
      "Change approved",
    );
    await expect(detail).toContainText("Approved");
    await expect(page.getByTestId("branch-review-current")).toContainText(
      value,
    );
    await expect(page.getByTestId("branch-review-events")).toContainText(
      "Approved",
    );
  });

  test("a member with no branch authority sees an explanation, not a queue", async ({
    page,
    request,
  }) => {
    // This identity is verified and linked but holds no branch grant.
    const grants = await api(
      request,
      DEACTIVATED,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(grants.status).toBe(200);
    expect(grants.body.grants).toEqual([]);

    await useToken(page, DEACTIVATED);
    await page.goto("/branch-review");

    await expect(page.getByTestId("branch-review-no-authority")).toBeVisible();
    await expect(page.getByTestId("branch-review-console")).toHaveCount(0);
    await expect(page.getByTestId("branch-review-list")).toHaveCount(0);
    await expect(page.getByTestId("branch-review-toolbar")).toHaveCount(0);
  });

  test("self-submitted and linked-owner proposals explain why they cannot be decided", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);

    // The reviewer is linked to e2e-branch-owner, so another reviewer must
    // decide proposals about that person.
    const ownedId = await propose(request, APPROVER, {
      personKey: "e2e-branch-owner",
      branchKey: "branch-1",
      assertionKey: `name.ui-owned.${RUN}`,
      value: `Synthetic UI Owned Name ${RUN}`,
    });

    await useToken(page, REVIEWER);
    await page.goto("/branch-review");
    await expect(page.getByTestId("branch-review-console")).toBeVisible();

    await page.getByTestId(`branch-review-row-${ownedId}`).click();
    await expect(
      page.getByTestId(`branch-review-detail-${ownedId}`),
    ).toBeVisible();
    await expect(
      page.getByTestId("branch-review-decision-blocked"),
    ).toContainText("linked to this genealogy person");
    await expect(page.getByTestId("branch-review-approve")).toHaveCount(0);

    // The reviewer cannot approve through the API either.
    const direct = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${ownedId}/decision`,
      { decision: "approve" },
    );
    expect(direct.status).toBe(403);
  });

  test("rejecting a proposal requires a note and keeps the verified fact", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);
    const proposalId = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey: `name.ui-reject.${RUN}`,
      value: `Synthetic UI Rejected Name ${RUN}`,
    });

    await useToken(page, REVIEWER);
    await page.goto("/branch-review");
    await page.getByTestId(`branch-review-row-${proposalId}`).click();
    await expect(
      page.getByTestId(`branch-review-detail-${proposalId}`),
    ).toBeVisible();

    await page.getByTestId("branch-review-reject").click();
    const confirmButton = page.getByTestId("branch-review-confirm-button");
    await expect(confirmButton).toBeDisabled();
    await expect(page.getByTestId("branch-review-confirm")).toContainText(
      "Add a reviewer note",
    );

    await page
      .getByTestId("branch-review-note")
      .fill("The cited page does not name this person.");
    await expect(confirmButton).toBeEnabled();
    await confirmButton.click();

    await expect(page.getByTestId("branch-review-message")).toContainText(
      "Proposal rejected",
    );
    await expect(
      page.getByTestId(`branch-review-detail-${proposalId}`),
    ).toContainText("Rejected");
    // Rejection never rewrites the verified value.
    const detail = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${proposalId}`,
    );
    expect(detail.body.current_assertion).toBeNull();
  });

  test("the console stays usable on a mobile viewport and by keyboard", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
    const proposalId = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey: `name.ui-mobile.${RUN}`,
      value: `Synthetic UI Mobile Name ${RUN}`,
    });

    await page.setViewportSize({ width: 390, height: 844 });
    await useToken(page, REVIEWER);
    await page.goto("/branch-review");
    await expect(page.getByTestId("branch-review-console")).toBeVisible();

    // The filters are labelled and reachable by keyboard.
    await page.getByTestId("branch-review-branch-filter").focus();
    await page.keyboard.press("Tab");
    await expect(
      page.getByTestId("branch-review-status-filter"),
    ).toBeFocused();

    // A row is a real button, so Enter opens it without a pointer.
    const row = page.getByTestId(`branch-review-row-${proposalId}`);
    await row.focus();
    await page.keyboard.press("Enter");
    await expect(
      page.getByTestId(`branch-review-detail-${proposalId}`),
    ).toBeVisible();
    await expect(page.getByTestId("branch-review-proposed")).toBeVisible();

    // Both panes remain in view rather than overflowing horizontally.
    const scrollWidth = await page.evaluate(
      () => document.documentElement.scrollWidth,
    );
    const clientWidth = await page.evaluate(
      () => document.documentElement.clientWidth,
    );
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);
  });
});
