import fs from "node:fs";
import path from "node:path";

import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-005D: the security and end-to-end acceptance matrix for branch review.
//
// Scenarios proven here, against the real proxy, the real Axum API, and
// disposable PostgreSQL fixtures:
//
//   9. account deactivation immediately blocks reviewer capabilities
//  15. administrator role and branch reviewer authority stay independent
//  16. no private profile, cross-branch, or unapproved content leaks through
//      the interface, the API, or the document surface
//
// The remaining scenarios are proven by tests/branch_review.rs,
// tests/onboarding_branch_grants.rs, and the branch-review browser specs; see
// docs/guides/branch-review-acceptance.md for the full matrix.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const REVIEWER_OFF = process.env.E2E_REVIEWER_OFF_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const REVIEWER_ACCOUNT_ID = Number(process.env.E2E_REVIEWER_ACCOUNT_ID || 0);
const APPROVER_ACCOUNT_ID = Number(process.env.E2E_APPROVER_ACCOUNT_ID || 0);
const REVIEWER_OFF_ACCOUNT_ID = Number(
  process.env.E2E_REVIEWER_OFF_ACCOUNT_ID || 0,
);
const READY = Boolean(
  ISSUER &&
    ADMIN &&
    APPROVER &&
    REVIEWER &&
    REVIEWER_OFF &&
    VIEWER &&
    REVIEWER_ACCOUNT_ID &&
    APPROVER_ACCOUNT_ID &&
    REVIEWER_OFF_ACCOUNT_ID,
);

type Grant = { grant_id: number; branch_key: string; active: boolean };
type Evidence = { source_evidence_id: number; document_ref: string; page: number };

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
    reason: "Synthetic E2E acceptance appointment",
  });
  expect(granted.status, JSON.stringify(granted.body)).toBe(200);
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
  const response = await api(
    request,
    token,
    "GET",
    `/api/branch-review/persons/${options.personKey}/evidence`,
  );
  expect(response.status, JSON.stringify(response.body)).toBe(200);
  const evidence = response.body.evidence as Evidence[];
  expect(evidence.length).toBeGreaterThan(0);
  const created = await api(request, token, "POST", "/api/branch-review/submissions", {
    person_key: options.personKey,
    branch_key: options.branchKey,
    assertion_kind: "name",
    assertion_key: options.assertionKey,
    proposed_value: options.value,
    evidence: [evidence[0]],
    note: "Synthetic E2E acceptance proposal",
  });
  expect(created.status, JSON.stringify(created.body)).toBe(200);
  return created.body.proposal_id as number;
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("branch review security acceptance", () => {
  // Scenario 9.
  test("deactivating a reviewer immediately removes review capability", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, REVIEWER_OFF_ACCOUNT_ID, "e2e-reviewer-off", [
      "branch-1",
    ]);
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);

    // The capability works before deactivation, so the denial is meaningful.
    const grants = await api(
      request,
      REVIEWER_OFF,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(grants.status).toBe(200);
    expect(grants.body.grants).toHaveLength(1);
    const grantedId = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey: `name.off-before.${RUN}`,
      value: `Synthetic Offboarding Approval ${RUN}`,
    });
    const approved = await api(
      request,
      REVIEWER_OFF,
      "POST",
      `/api/branch-review/submissions/${grantedId}/decision`,
      { decision: "approve", note: "Approved before deactivation." },
    );
    expect(approved.status, JSON.stringify(approved.body)).toBe(200);

    const pendingId = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey: `name.off-after.${RUN}`,
      value: `Synthetic Offboarding Pending ${RUN}`,
    });

    await useToken(page, REVIEWER_OFF);
    await page.goto("/branch-review");
    await expect(page.getByTestId("branch-review-console")).toBeVisible();
    await expect(page.getByTestId(`branch-review-row-${pendingId}`)).toBeVisible();

    const deactivated = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/users/e2e-reviewer-off/deactivate?issuer=${encodeURIComponent(ISSUER)}`,
    );
    expect(deactivated.status, JSON.stringify(deactivated.body)).toBe(200);

    // Every review read and write is denied, not merely emptied.
    const afterGrants = await api(
      request,
      REVIEWER_OFF,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect([401, 403]).toContain(afterGrants.status);
    const afterQueue = await api(
      request,
      REVIEWER_OFF,
      "GET",
      "/api/branch-review/submissions",
    );
    expect([401, 403]).toContain(afterQueue.status);
    const afterDecision = await api(
      request,
      REVIEWER_OFF,
      "POST",
      `/api/branch-review/submissions/${pendingId}/decision`,
      { decision: "approve" },
    );
    expect([401, 403]).toContain(afterDecision.status);

    await page.goto("/branch-review");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("branch-review-console")).toHaveCount(0);
  });

  // Scenario 15.
  test("administrator role and branch reviewer authority stay independent", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);

    // A branch reviewer is an ordinary member with a grant: no admin surface.
    for (const [method, path, data] of [
      ["GET", "/api/admin/reviews", undefined],
      ["GET", "/api/admin/family-members", undefined],
      [
        "POST",
        "/api/admin/branch-reviewers",
        {
          issuer: ISSUER,
          subject: "e2e-reviewer",
          branch_keys: ["branch-2"],
          reason: "should be denied",
        },
      ],
    ] as const) {
      const denied = await api(request, REVIEWER, method, path, data);
      expect(denied.status, `${method} ${path}`).toBe(403);
    }

    await useToken(page, REVIEWER);
    await page.goto("/branch-review");
    const nav = page.getByTestId("site-nav");
    await expect(page.getByTestId("branch-review-console")).toBeVisible();
    await expect(nav.getByRole("link", { name: /Reviews/ })).toHaveCount(0);
    await expect(nav.getByRole("link", { name: /Bulk onboard/ })).toHaveCount(0);

    // An administrator without a grant has no branch authority either.
    const adminGrants = await api(
      request,
      ADMIN,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(adminGrants.status).toBe(200);
    expect(adminGrants.body.grants).toEqual([]);
    const adminQueue = await api(
      request,
      ADMIN,
      "GET",
      "/api/branch-review/submissions",
    );
    expect(adminQueue.status).toBe(200);
    expect(adminQueue.body.submissions).toEqual([]);

    await useToken(page, ADMIN);
    await page.goto("/branch-review");
    await expect(page.getByTestId("branch-review-no-authority")).toBeVisible();
  });

  // Scenario 16, branch-review surface.
  test("branch review never exposes private profile data or another branch", async ({
    page,
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);
    // The approver needs branch-2 only to build the cross-branch fixture.
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
      "branch-2",
    ]);
    const visibleId = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey: `name.leak-visible.${RUN}`,
      value: `Synthetic Visible Proposal ${RUN}`,
    });
    const otherBranchId = await propose(request, APPROVER, {
      personKey: "e2e-branch-other",
      branchKey: "branch-2",
      assertionKey: `name.leak-other.${RUN}`,
      value: `Synthetic Off Branch Proposal ${RUN}`,
    });

    // The server refuses the off-branch proposal outright.
    const offBranch = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${otherBranchId}`,
    );
    expect(offBranch.status).toBe(404);
    expect(JSON.stringify(offBranch.body)).not.toContain(
      `Synthetic Off Branch Proposal ${RUN}`,
    );

    const profileCalls: string[] = [];
    page.on("request", (request) => {
      const url = request.url();
      if (url.includes("/api/profile")) profileCalls.push(url);
    });

    await useToken(page, REVIEWER);
    await page.goto("/branch-review");
    await page.getByTestId(`branch-review-row-${visibleId}`).click();
    await expect(
      page.getByTestId(`branch-review-detail-${visibleId}`),
    ).toBeVisible();

    // The page never asks for profile data and never renders private fields.
    expect(profileCalls).toEqual([]);
    const body = await page.locator("body").innerText();
    for (const forbidden of [
      "owner_private",
      "current_location",
      "contact_email",
      "Preferred name",
    ]) {
      expect(body).not.toContain(forbidden);
    }
    const detail = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${visibleId}`,
    );
    expect(detail.status).toBe(200);
    expect(JSON.stringify(detail.body)).not.toContain("owner_private");

    // Restore the approver to branch-1 for the following specs.
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
  });

  // Scenario 16, document surface: an unapproved submission is review-only.
  test("a pending document submission is review-only, not family-visible", async ({
    page,
    request,
  }) => {
    const title = `Synthetic Acceptance Document ${RUN}`;
    const upload = await request.post(`${API_URL}/api/documents/ingest`, {
      headers: auth(ADMIN),
      multipart: {
        file: {
          name: "acceptance.md",
          mimeType: "text/markdown",
          buffer: fs.readFileSync(
            path.join(process.cwd(), "..", "examples", "loan-policy.md"),
          ),
        },
        title,
        category: "other",
        scope: "family",
      },
    });
    expect(upload.status(), await upload.text()).toBe(200);
    const submitted = (await upload.json()) as {
      version_id: number;
      review_status: string;
      chunks: number;
    };
    expect(submitted.review_status).toBe("pending_review");
    expect(submitted.chunks).toBe(0);

    // A reviewer sees it on /documents; a family viewer cannot reach the surface
    // at all, and the API refuses the listing.
    await useToken(page, ADMIN);
    await page.goto("/documents");
    await expect(
      page.getByTestId(`document-row-${submitted.version_id}`),
    ).toContainText(title);

    const viewerList = await api(
      request,
      VIEWER,
      "GET",
      "/api/documents/submissions",
    );
    expect(viewerList.status).toBe(403);

    await useToken(page, VIEWER);
    await page.goto("/documents");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("document-list")).toHaveCount(0);
  });
});
