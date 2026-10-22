import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-006-A: approved-only family profile presentation.
//
// Exercises the real Next.js proxy, the real Axum API, and disposable
// PostgreSQL fixtures seeded by scripts/web-e2e.sh. The suite proves the
// endpoint returns only approved, family-visible values and that the
// owner-private value seeded for the same person never appears — in a success
// body or in an error body.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const MEMBER = process.env.E2E_FAMILY_MEMBER_TOKEN || "";
const PRIVATE_MARKER = process.env.E2E_PRIVATE_PROFILE_MARKER || "";
const FAMILY_MARKER = process.env.E2E_FAMILY_PROFILE_MARKER || "";
const FAMILY_MEMBER_ACCOUNT_ID = Number(
  process.env.E2E_FAMILY_MEMBER_ACCOUNT_ID || 0,
);
const READY = Boolean(
  ISSUER &&
    ADMIN &&
    REVIEWER &&
    MEMBER &&
    PRIVATE_MARKER &&
    FAMILY_MARKER &&
    FAMILY_MEMBER_ACCOUNT_ID,
);

type Field = {
  field_key: string;
  value: string;
  visibility: string;
  approved_revision_number: number;
  updated_at: string;
};

type ProfileBody = {
  ok: boolean;
  state: string;
  person: { person_key: string; display_name: string };
  fields: Field[];
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

/// Reads through the Next.js proxy with the browser's own session cookie, so
/// the session, the proxy, and the Rust authorization all participate.
async function proxiedProfile(page: Page, personKey: string) {
  const response = await page.request.get(
    `${ORIGIN}/api/family/persons/${encodeURIComponent(personKey)}/profile`,
    { headers: { accept: "application/json" } },
  );
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

async function linkIdFor(request: APIRequestContext, subject: string) {
  const directory = await api(
    request,
    ADMIN,
    "GET",
    `/api/admin/family-members?q=${encodeURIComponent(subject)}`,
  );
  expect(directory.status, JSON.stringify(directory.body)).toBe(200);
  const member = (directory.body.members as {
    subject: string;
    linked_person: { link_id: number; person_key: string } | null;
  }[]).find((row) => row.subject === subject);
  expect(member?.linked_person, "the fixture account must be linked").toBeTruthy();
  return { linkId: member!.linked_person!.link_id, personKey: member!.linked_person!.person_key };
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("family profile presentation", () => {
  test("a linked relative sees approved family values only", async ({ page }) => {
    await useToken(page, REVIEWER);
    const { status, body } = await proxiedProfile(page, "e2e-branch-subject");
    expect(status, JSON.stringify(body)).toBe(200);

    const profile = body as ProfileBody;
    expect(profile.ok).toBe(true);
    expect(profile.state).toBe("published");
    expect(profile.person.person_key).toBe("e2e-branch-subject");
    expect(profile.person.display_name).toBe("Synthetic Branch Subject");

    const biography = profile.fields.find((field) => field.field_key === "biography");
    expect(biography?.value).toBe(FAMILY_MARKER);
    expect(biography?.visibility).toBe("family");
    expect(biography?.approved_revision_number).toBe(1);
    expect(biography?.updated_at).toBeTruthy();
    // Only family-visible, non-media fields are published.
    expect(profile.fields.map((field) => field.field_key)).toEqual(["biography"]);

    // The owner-private value seeded for the same person never appears.
    expect(JSON.stringify(body)).not.toContain(PRIVATE_MARKER);
    expect(JSON.stringify(body)).not.toContain("current_location");
    expect(JSON.stringify(body)).not.toContain("profile_photo");
  });

  test("empty, unknown, and unauthorized profiles are distinct and leak nothing", async ({
    page,
    request,
  }) => {
    // The reviewer is linked to e2e-branch-owner, which has no family values.
    await useToken(page, REVIEWER);
    const empty = await proxiedProfile(page, "e2e-branch-owner");
    expect(empty.status, JSON.stringify(empty.body)).toBe(200);
    const emptyBody = empty.body as ProfileBody;
    expect(emptyBody.state).toBe("empty");
    expect(emptyBody.fields).toEqual([]);
    expect(JSON.stringify(empty.body)).not.toContain(PRIVATE_MARKER);

    // A linked member asking for an unknown key gets a plain 404.
    const missing = await proxiedProfile(page, "e2e-no-such-person");
    expect(missing.status).toBe(404);
    expect(JSON.stringify(missing.body)).not.toContain("Synthetic Branch Subject");
    expect(JSON.stringify(missing.body)).not.toContain(PRIVATE_MARKER);

    // An unlinked account is refused before the person is looked up, so an
    // existing and a missing key produce the same 403 body.
    const stranger = await api(
      request,
      process.env.E2E_VIEWER_TOKEN || "",
      "GET",
      "/api/family/persons/e2e-branch-subject/profile",
    );
    expect(stranger.status).toBe(403);
    expect(stranger.body.error).toBe("family_profile_forbidden");
    const strangerMissing = await api(
      request,
      process.env.E2E_VIEWER_TOKEN || "",
      "GET",
      "/api/family/persons/e2e-no-such-person/profile",
    );
    expect(strangerMissing.status).toBe(403);
    expect(JSON.stringify(stranger.body)).not.toContain(PRIVATE_MARKER);
    expect(JSON.stringify(stranger.body)).not.toContain(FAMILY_MARKER);

    // Anonymous requests are refused by the API, not merely hidden by the UI.
    const anonymous = await request.get(
      `${API_URL}/api/family/persons/e2e-branch-subject/profile`,
    );
    expect(anonymous.status()).toBe(401);
  });

  test("revoking the link or deactivating the account blocks the endpoint", async ({
    page,
    request,
  }) => {
    // The dedicated member starts linked and active.
    await useToken(page, MEMBER);
    const before = await proxiedProfile(page, "e2e-branch-subject");
    expect(before.status, JSON.stringify(before.body)).toBe(200);
    expect(JSON.stringify(before.body)).toContain(FAMILY_MARKER);

    const { linkId, personKey } = await linkIdFor(request, "e2e-family-member");
    expect(personKey).toBe("e2e-branch-family-member");

    // Revocation is immediate: the next read is refused.
    const revoked = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/family-members/links/${linkId}/revoke`,
    );
    expect(revoked.status, JSON.stringify(revoked.body)).toBe(200);
    const afterRevoke = await proxiedProfile(page, "e2e-branch-subject");
    expect(afterRevoke.status).toBe(403);
    expect(JSON.stringify(afterRevoke.body)).not.toContain(FAMILY_MARKER);

    // Re-linking restores access, which proves revocation was the cause.
    const relinked = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/family-members/links",
      {
        person_key: "e2e-branch-family-member",
        issuer: ISSUER,
        subject: "e2e-family-member",
        decision: "approve",
      },
    );
    expect(relinked.status, JSON.stringify(relinked.body)).toBe(200);
    const afterRelink = await proxiedProfile(page, "e2e-branch-subject");
    expect(afterRelink.status).toBe(200);

    // A deactivated identity loses the endpoint even while the link exists.
    const deactivated = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/users/e2e-family-member/deactivate?issuer=${encodeURIComponent(ISSUER)}`,
    );
    expect(deactivated.status, JSON.stringify(deactivated.body)).toBe(200);
    const afterDeactivate = await proxiedProfile(page, "e2e-branch-subject");
    expect([401, 403]).toContain(afterDeactivate.status);
    expect(JSON.stringify(afterDeactivate.body)).not.toContain(FAMILY_MARKER);
  });
});
