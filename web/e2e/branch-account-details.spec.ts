import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// BD-003/BD-004/BD-005/BD-006 — administrative account/contact details and the
// recorded-branch grant path, against the real Axum API and disposable
// PostgreSQL data seeded by scripts/web-e2e.sh. Every value is synthetic.

const API_URL = (process.env.E2E_API_URL || "http://127.0.0.1:8080").replace(
  /\/+$/,
  "",
);
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const READY = Boolean(ISSUER && ADMIN && VIEWER);

type ContactValue = {
  value: string | null;
  source: "identity_account" | "not_recorded";
  verification: "not_recorded";
};

type Contact = { email: ContactValue; phone: ContactValue };

type MemberRow = {
  user_id: number;
  subject: string;
  email: string | null;
  linked_person: {
    link_id: number;
    person_id: number;
    person_key: string;
    display_name: string;
    username: string | null;
  } | null;
  contact: Contact;
  branches: {
    state: string;
    verified_branch_key: string | null;
    claims: {
      value_text: string | null;
      verified: boolean;
      branch_key: string | null;
    }[];
  } | null;
  grants: { grant_id: number; branch_key: string; active: boolean }[];
};

type BranchBody = {
  state: string;
  verified_branch_key: string | null;
  claims: {
    value_text: string | null;
    review_status: string;
    verified: boolean;
    branch_key: string | null;
    branch_label: string | null;
  }[];
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

async function member(request: APIRequestContext, subject: string) {
  const response = await api(
    request,
    ADMIN,
    "GET",
    `/api/admin/family-members?q=${encodeURIComponent(subject)}`,
  );
  expect(response.status, JSON.stringify(response.body)).toBe(200);
  const rows = (response.body.members as MemberRow[]).filter(
    (row) => row.subject === subject,
  );
  expect(rows.length, `one directory row for ${subject}`).toBe(1);
  return rows[0];
}

async function personBranch(request: APIRequestContext, personKey: string) {
  const response = await api(
    request,
    ADMIN,
    "GET",
    `/api/genealogy/person/${encodeURIComponent(personKey)}`,
  );
  expect(response.status, JSON.stringify(response.body)).toBe(200);
  return response.body.branches as BranchBody;
}

async function signIn(page: Page) {
  await page.goto("/signin");
  await page.getByTestId("signin-button").click();
  const escaped = ORIGIN.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  await page.waitForURL(new RegExp(`${escaped}/$`), { timeout: 30_000 });
  await expect(page.getByTestId("account-role")).toHaveText("admin");
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

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("branch account details", () => {
  test("non-admin and anonymous callers cannot read the directory or grant", async ({
    request,
  }) => {
    const viewer = { authorization: `Bearer ${VIEWER}` };
    const directory = await request.get(
      `${API_URL}/api/admin/family-members?q=e2e-member`,
      { headers: viewer },
    );
    expect(directory.status()).toBe(403);
    const directoryBody = await directory.text();
    expect(directoryBody).not.toContain("@example.test");

    const search = await request.get(
      `${API_URL}/api/admin/branch-reviewers/persons?q=synthetic`,
      { headers: viewer },
    );
    expect(search.status()).toBe(403);

    const anonymous = await request.get(`${API_URL}/api/admin/family-members`);
    expect(anonymous.status()).toBe(401);
    expect(
      (
        await request.get(
          `${API_URL}/api/admin/branch-reviewers/persons?q=synthetic`,
        )
      ).status(),
    ).toBe(401);

    const deniedGrant = await request.post(
      `${API_URL}/api/admin/branch-reviewers/grants`,
      {
        headers: viewer,
        data: {
          issuer: ISSUER,
          subject: "e2e-branch-grantee",
          person_key: "e2e-branch-grantee",
          reason: "Synthetic viewer attempt",
        },
      },
    );
    expect(deniedGrant.status()).toBe(403);
  });

  test("the directory labels the contact source and records a missing phone", async ({
    request,
  }) => {
    const withEmail = await member(request, "e2e-member");
    expect(withEmail.contact.email).toEqual({
      value: "e2e-member@example.test",
      source: "identity_account",
      verification: "not_recorded",
    });
    expect(withEmail.contact.phone).toEqual({
      value: null,
      source: "not_recorded",
      verification: "not_recorded",
    });

    // The grantee has a linked person with one verified branch and no email.
    const grantee = await member(request, "e2e-branch-grantee");
    expect(grantee.linked_person?.person_id).toBeGreaterThan(0);
    expect(grantee.linked_person?.person_key).toBe("e2e-branch-grantee");
    expect(grantee.contact.email.value).toBeNull();
    expect(grantee.contact.email.source).toBe("identity_account");
    expect(grantee.branches?.state).toBe("verified");
    expect(grantee.branches?.verified_branch_key).toBe("branch-2");

    // A member-facing read never carries the admin contact facts.
    const person = await api(
      request,
      ADMIN,
      "GET",
      "/api/genealogy/person/e2e-branch-grantee",
    );
    expect(person.status).toBe(200);
    expect(JSON.stringify(person.body)).not.toContain("contacts");
    expect(JSON.stringify(person.body)).not.toContain("@example.test");
  });

  test("the four recorded branch states are reported without inference", async ({
    request,
  }) => {
    const verified = await personBranch(request, "e2e-branch-subject");
    expect(verified.state).toBe("verified");
    expect(verified.verified_branch_key).toBe("branch-1");

    const unverified = await personBranch(request, "e2e-branch-unverified");
    expect(unverified.state).toBe("unverified");
    expect(unverified.verified_branch_key).toBeNull();
    expect(
      unverified.claims.every((claim) => claim.verified === false),
    ).toBeTruthy();
    expect(unverified.claims[0]?.value_text).toBe("branch-1");

    const conflicting = await personBranch(request, "e2e-branch-conflicting");
    expect(conflicting.state).toBe("conflicting");
    expect(conflicting.verified_branch_key).toBeNull();
    const keys = new Set(
      conflicting.claims.map((claim) => claim.branch_key).filter(Boolean),
    );
    expect(keys.size).toBeGreaterThan(1);

    const missing = await personBranch(request, "e2e-branch-missing");
    expect(missing.state).toBe("missing");
    expect(missing.verified_branch_key).toBeNull();
    expect(missing.claims).toHaveLength(0);
  });

  test("a viewer sees the denial and no grant console", async ({ page }) => {
    await useToken(page, VIEWER);
    await page.goto("/admin/family-members");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("branch-grant-admin")).toHaveCount(0);
  });

  test("an administrator grants and revokes through the console", async ({
    page,
  }) => {
    await signIn(page);
    await page.goto("/admin/family-members");
    const panel = page.getByTestId("branch-grant-admin");
    await expect(panel).toBeVisible();

    await page
      .getByTestId("branch-grant-search")
      .fill("Synthetic Recorded Branch Grantee");
    const result = page.locator(
      '[data-testid="branch-grant-result"][data-person="e2e-branch-grantee"]',
    );
    await expect(result).toHaveCount(1);
    await expect(result).toContainText("e2e-branch-grantee");
    await expect(result).toContainText("person_id");
    await expect(result).toContainText("user_id");
    await expect(result).toContainText(
      "not recorded (source: identity account, verification: not recorded)",
    );
    await expect(result).toContainText("not recorded (source: not recorded)");
    await expect(result).toContainText("Second Branch (branch key: branch-2)");
    await expect(result.getByTestId("branch-grant-eligibility")).toContainText(
      "Eligible",
    );

    // Cancel takes focus, Escape cancels, and the confirm action stays disabled
    // until a reason is entered.
    await result.getByTestId("branch-grant-start").click();
    const dialog = page.getByTestId("branch-grant-confirm");
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText("Synthetic Recorded Branch Grantee");
    await expect(dialog).toContainText("Second Branch");
    await expect(page.getByTestId("branch-grant-confirm-cancel")).toBeFocused();
    await expect(page.getByTestId("branch-grant-confirm-yes")).toBeDisabled();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);

    await result.getByTestId("branch-grant-start").click();
    await page
      .getByTestId("branch-grant-confirm-reason")
      .fill("Synthetic recorded branch appointment");
    await page.getByTestId("branch-grant-confirm-yes").click();
    await expect(panel.locator(".form-status")).toContainText(
      "Granted branch review",
    );

    const activeRow = page.locator(
      '[data-testid="branch-grant-row"][data-person="e2e-branch-grantee"][data-branch="branch-2"][data-active="true"]',
    );
    await expect(activeRow).toHaveCount(1);
    await expect(activeRow).toContainText("Synthetic Recorded Branch Grantee");
    await expect(activeRow).toContainText("active");
    await expect(activeRow).toContainText(
      "Synthetic recorded branch appointment",
    );

    // The search re-runs, so eligibility now reports the existing grant.
    await expect(result.getByTestId("branch-grant-eligibility")).toContainText(
      "Already granted",
    );
    await expect(result.getByTestId("branch-grant-start")).toBeDisabled();

    await activeRow.getByTestId("branch-grant-revoke").click();
    const revoke = page.getByTestId("branch-grant-revoke-confirm");
    await expect(revoke).toContainText("Second Branch");
    await page
      .getByTestId("branch-grant-revoke-confirm-reason")
      .fill("Synthetic cleanup");
    await page.getByTestId("branch-grant-revoke-confirm-yes").click();
    await expect(panel.locator(".form-status")).toContainText("Revoked");
    await expect(activeRow).toHaveCount(0);
    await expect(result.getByTestId("branch-grant-eligibility")).toContainText(
      "Eligible",
    );
  });

  test("a valid grant needs no email, and every other membership state is rejected", async ({
    request,
  }) => {
    const grantee = await member(request, "e2e-branch-grantee");
    expect(grantee.contact.email.value).toBeNull();

    const rejected = [
      {
        subject: "e2e-branch-unverified",
        person: "e2e-branch-unverified",
        code: "branch_membership_unverified",
      },
      {
        subject: "e2e-branch-conflicting",
        person: "e2e-branch-conflicting",
        code: "branch_membership_conflicting",
      },
      {
        subject: "e2e-branch-missing",
        person: "e2e-branch-missing",
        code: "branch_membership_missing",
      },
      {
        subject: "e2e-viewer",
        person: "e2e-branch-subject",
        code: "account_not_linked",
      },
      {
        subject: "e2e-branch-grantee",
        person: "e2e-branch-subject",
        code: "person_link_mismatch",
      },
    ];
    for (const entry of rejected) {
      const response = await api(
        request,
        ADMIN,
        "POST",
        "/api/admin/branch-reviewers/grants",
        {
          issuer: ISSUER,
          subject: entry.subject,
          person_key: entry.person,
          reason: "Synthetic rejection check",
        },
      );
      expect(response.status, JSON.stringify(response.body)).toBe(409);
      expect(response.body.error).toBe(entry.code);
    }

    // The valid grant resolves the person's one verified branch and no other.
    const granted = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers/grants",
      {
        issuer: ISSUER,
        subject: "e2e-branch-grantee",
        person_key: "e2e-branch-grantee",
        reason: "Synthetic recorded branch appointment",
      },
    );
    expect(granted.status, JSON.stringify(granted.body)).toBe(200);
    expect(granted.body.grant.branch_key).toBe("branch-2");
    expect(granted.body.grant.active).toBe(true);

    const duplicate = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/branch-reviewers/grants",
      {
        issuer: ISSUER,
        subject: "e2e-branch-grantee",
        person_key: "e2e-branch-grantee",
        reason: "Synthetic duplicate attempt",
      },
    );
    expect(duplicate.status).toBe(409);
    expect(duplicate.body.error).toBe("branch_grant_exists");

    const listed = await api(
      request,
      ADMIN,
      "GET",
      `/api/admin/branch-reviewers?account_id=${grantee.user_id}`,
    );
    expect(listed.status).toBe(200);
    const branchGrants = (
      listed.body.grants as { branch_key: string; active: boolean }[]
    ).filter((grant) => grant.active && grant.branch_key === "branch-2");
    expect(branchGrants).toHaveLength(1);

    const search = await api(
      request,
      ADMIN,
      "GET",
      "/api/admin/branch-reviewers/persons?q=e2e-branch-grantee",
    );
    expect(search.status).toBe(200);
    const candidate = (
      search.body.people as {
        person: { person_key: string };
        eligibility: string;
        account: { email: string | null } | null;
        contact: Contact | null;
        active_grant: { branch_key: string } | null;
      }[]
    ).find((row) => row.person.person_key === "e2e-branch-grantee");
    expect(candidate?.eligibility).toBe("already_granted");
    expect(candidate?.account?.email).toBeNull();
    expect(candidate?.contact?.email.value).toBeNull();
    expect(candidate?.active_grant?.branch_key).toBe("branch-2");
  });
});
