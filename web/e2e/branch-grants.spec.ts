import { expect, test } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

// USR-005A branch reviewer grant administration. These are API-level checks
// against the real Axum API and disposable PostgreSQL data; the /branch-review
// UI arrives in USR-005C.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const READY = Boolean(ISSUER && ADMIN && VIEWER);

type Grant = {
  grant_id: number;
  account_id: number;
  branch_key: string;
  active: boolean;
  usable: boolean;
  granted_reason: string | null;
  revoked_by: string | null;
  revoke_reason: string | null;
};

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

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("branch reviewer grants", () => {
  test("administrator grants, lists, revokes, and re-grants branches", async ({
    request,
  }) => {
    const branches = await api(
      request,
      ADMIN,
      "GET",
      "/api/admin/branch-reviewers/branches",
    );
    expect(branches.status).toBe(200);
    const keys = (branches.body.branches as { branch_key: string }[]).map(
      (branch) => branch.branch_key,
    );
    expect(keys).toEqual(expect.arrayContaining(["branch-1", "branch-2", "branch-3"]));

    const granted = await api(request, ADMIN, "POST", "/api/admin/branch-reviewers", {
      issuer: ISSUER,
      subject: "e2e-approver",
      branch_keys: ["branch-1", "branch-2"],
      reason: "Synthetic reviewer appointment",
    });
    expect(granted.status, JSON.stringify(granted.body)).toBe(200);
    const grants = granted.body.grants as Grant[];
    expect(grants).toHaveLength(2);
    expect(grants.every((grant) => grant.active && grant.usable)).toBeTruthy();
    const accountId = grants[0].account_id;

    const listed = await api(
      request,
      ADMIN,
      "GET",
      `/api/admin/branch-reviewers?account_id=${accountId}`,
    );
    expect(listed.status).toBe(200);
    expect((listed.body.grants as Grant[]).length).toBe(2);
    expect(
      (listed.body.events as { event: string }[]).filter(
        (event) => event.event === "granted",
      ).length,
    ).toBe(2);
    expect(JSON.stringify(listed.body)).not.toContain(ISSUER);
    expect(JSON.stringify(listed.body)).not.toContain("e2e-approver");

    const duplicate = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-approver",
        branch_keys: ["branch-1"],
        reason: "duplicate",
      },
    );
    expect(duplicate.status).toBe(409);

    const blank = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-approver",
        branch_keys: ["branch-2", "   "],
        reason: "mixed blank selection",
      },
    );
    expect(blank.status).toBe(400);

    const paged = await api(
      request,
      ADMIN,
      "GET",
      `/api/admin/branch-reviewers?account_id=${accountId}&limit=1`,
    );
    expect(paged.status).toBe(200);
    expect((paged.body.grants as Grant[]).length).toBe(1);
    expect((paged.body.events as { event: string }[]).length).toBe(1);
    expect(typeof paged.body.grants_next_cursor).toBe("number");
    expect(typeof paged.body.events_next_cursor).toBe("number");

    const first = grants.find((grant) => grant.branch_key === "branch-1")!;
    const revoked = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/branch-reviewers/${first.grant_id}/revoke`,
      { reason: "Synthetic term ended" },
    );
    expect(revoked.status).toBe(200);
    expect(revoked.body.grant.active).toBe(false);
    expect(revoked.body.grant.revoke_reason).toBe("Synthetic term ended");

    const afterRevoke = await api(
      request,
      ADMIN,
      "GET",
      `/api/admin/branch-reviewers?account_id=${accountId}`,
    );
    const revokedGrant = (afterRevoke.body.grants as Grant[]).find(
      (grant) => grant.grant_id === first.grant_id,
    );
    expect(revokedGrant?.active).toBe(false);
    expect(
      (afterRevoke.body.events as { event: string }[]).filter(
        (event) => event.event === "revoked",
      ).length,
    ).toBe(1);

    const regranted = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-approver",
        branch_keys: ["branch-1"],
        reason: "Synthetic reappointment",
      },
    );
    expect(regranted.status, JSON.stringify(regranted.body)).toBe(200);
    expect(regranted.body.grants[0].grant_id).not.toBe(first.grant_id);
    expect(regranted.body.grants[0].usable).toBe(true);
  });

  test("non-admin and unlinked accounts cannot receive grants", async ({ request }) => {
    const nonAdmin = await api(
      request,
      VIEWER,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-viewer",
        branch_keys: ["branch-1"],
        reason: "should be denied",
      },
    );
    expect(nonAdmin.status).toBe(403);

    const unlinked = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-viewer",
        branch_keys: ["branch-1"],
        reason: "unlinked target",
      },
    );
    expect(unlinked.status).toBe(409);
    expect(unlinked.body.error).toBe("account_not_linked");

    const unknown = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-does-not-exist",
        branch_keys: ["branch-1"],
        reason: "unknown target",
      },
    );
    expect(unknown.status).toBe(404);
  });

  test("revoking the reviewed link makes an active grant unusable", async ({
    request,
  }) => {
    const linked = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/family-members/links",
      {
        person_key: "e2e-profile-viewer",
        issuer: ISSUER,
        subject: "e2e-viewer",
        decision: "approve",
      },
    );
    expect(linked.status, JSON.stringify(linked.body)).toBe(200);

    const granted = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers",
      {
        issuer: ISSUER,
        subject: "e2e-viewer",
        branch_keys: ["branch-1"],
        reason: "Synthetic viewer appointment",
      },
    );
    expect(granted.status, JSON.stringify(granted.body)).toBe(200);
    const grant = granted.body.grants[0] as Grant;
    expect(grant.usable).toBe(true);

    const revoked = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/family-members/links/${linked.body.link_id}/revoke`,
    );
    expect(revoked.status).toBe(200);

    const listed = await api(
      request,
      ADMIN,
      "GET",
      `/api/admin/branch-reviewers?account_id=${grant.account_id}`,
    );
    const after = (listed.body.grants as Grant[]).find(
      (row) => row.grant_id === grant.grant_id,
    );
    expect(after?.active).toBe(true);
    expect(after?.usable).toBe(false);
  });
});
