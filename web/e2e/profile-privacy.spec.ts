import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-006-P: owner-initiated privacy restriction (decision A2).
//
// The dedicated fixture gives one synthetic member a family-visible biography
// and a linked owner account. The suite proves that the owner can stop family
// presentation immediately, that the family read and page honour it, that the
// owner keeps a private view, that other accounts are refused, and that a new
// independently approved family revision is the only way back.
//
// Case map:
//  1 approved family biography initially visible
//  2 restriction immediately removes it from the family API read
//  3 the family page no longer renders it
//  4 the owner still reads the original content privately
//  5 an unrelated account cannot restrict another person's content
//  6 a revoked link cannot restrict (Rust suite, link revocation)
//  7 a deactivated account cannot restrict
//  8 an unknown/non-family field cannot be restricted
//  9 historical revisions unchanged (Rust suite)
// 10 a stale pre-restriction approval cannot republish (Rust suite)
// 11 a new independently approved family revision restores visibility
// 12 concurrent approval/restriction serialize (Rust suite)
// 13 repeated restriction is idempotent
// 14 failed transactions cannot report success (asserted by status codes)
// 15 errors do not expose private values
// 16 restriction survives a page reload and repeat runs (fixture self-heal)
// 17 no retrieval side effects (Rust suite)

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const OWNER = process.env.E2E_RESTRICT_OWNER_TOKEN || "";
const RESTRICTABLE = process.env.E2E_RESTRICTABLE_MARKER || "";
const PERSON = "e2e-branch-restrictable";
const READY = Boolean(
  ISSUER && ADMIN && APPROVER && REVIEWER && VIEWER && OWNER && RESTRICTABLE,
);

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

async function familyProfile(
  request: APIRequestContext,
  token: string,
) {
  return api(
    request,
    token,
    "GET",
    `/api/family/persons/${PERSON}/profile`,
  );
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("owner privacy restriction", () => {
  test("the owner makes an approved family biography private and the family loses access", async ({
    page,
    request,
  }) => {
    // 1. Initially visible to another family member.
    const before = await familyProfile(request, REVIEWER);
    expect(before.status, JSON.stringify(before.body)).toBe(200);
    expect(JSON.stringify(before.body)).toContain(RESTRICTABLE);

    // The owner sees the field with a "Make private" action.
    await useToken(page, OWNER);
    await page.goto("/profile");
    const card = page.getByTestId("profile-field-biography");
    await expect(card).toBeVisible();
    await expect(page.getByTestId("profile-make-private-biography")).toBeVisible();

    page.once("dialog", (dialog) => void dialog.accept());
    await page.getByTestId("profile-make-private-biography").click();
    await expect(page.getByTestId("profile-restricted-biography")).toBeVisible();
    await expect(page.getByTestId("profile-visibility-biography")).toContainText(
      "private",
    );
    // No toggle back to family is offered.
    await expect(page.getByTestId("profile-make-private-biography")).toHaveCount(0);

    // 2. The family API read excludes it immediately.
    const after = await familyProfile(request, REVIEWER);
    expect(after.status).toBe(200);
    expect(JSON.stringify(after.body)).not.toContain(RESTRICTABLE);
    expect((after.body as { fields: unknown[] }).fields).toEqual([]);

    // 3. The family page renders the empty state, not the biography.
    await useToken(page, REVIEWER);
    await page.goto(`/family/${PERSON}`);
    await expect(page.getByTestId("family-profile-empty")).toBeVisible();
    expect(await page.locator("body").innerText()).not.toContain(RESTRICTABLE);

    // 4. The owner still reads the value privately, marked as restricted.
    const ownerProfile = await api(request, OWNER, "GET", "/api/profile");
    expect(ownerProfile.status).toBe(200);
    const biography = (ownerProfile.body.profile.approved as {
      field_key: string;
      value: string;
      visibility: string;
      restricted_from_family: boolean;
      restriction_generation: number | null;
    }[]).find((row) => row.field_key === "biography");
    expect(biography?.value).toBe(RESTRICTABLE);
    expect(biography?.visibility).toBe("family");
    expect(biography?.restricted_from_family).toBe(true);
    expect(biography?.restriction_generation).toBeGreaterThanOrEqual(1);

    // 16. A reload does not resurrect family visibility.
    await useToken(page, OWNER);
    await page.goto("/profile");
    await expect(page.getByTestId("profile-restricted-biography")).toBeVisible();
    expect((await familyProfile(request, REVIEWER)).body.fields).toEqual([]);

    // 13. Repeating the restriction is an idempotent success.
    const repeat = await api(
      request,
      OWNER,
      "POST",
      "/api/profile/fields/biography/restrict",
    );
    expect(repeat.status, JSON.stringify(repeat.body)).toBe(200);
    expect(repeat.body.already_restricted).toBe(true);

    // 11. Only a new independently approved family revision restores it.
    const draft = await api(request, OWNER, "POST", "/api/profile/drafts", {
      field_key: "biography",
      value: RESTRICTABLE,
      visibility: "family",
    });
    expect(draft.status, JSON.stringify(draft.body)).toBe(200);
    const revisionId = draft.body.revision.revision_id as number;
    const submitted = await api(
      request,
      OWNER,
      "POST",
      `/api/profile/drafts/${revisionId}/submit`,
    );
    expect(submitted.status, JSON.stringify(submitted.body)).toBe(200);
    const approved = await api(
      request,
      APPROVER,
      "POST",
      `/api/profile/revisions/${revisionId}/review`,
      { decision: "approve", note: "Owner asked to share this again." },
    );
    expect(approved.status, JSON.stringify(approved.body)).toBe(200);

    const restored = await familyProfile(request, REVIEWER);
    expect(restored.status).toBe(200);
    expect(JSON.stringify(restored.body)).toContain(RESTRICTABLE);
  });

  test("other accounts and invalid requests cannot restrict the field", async ({
    page,
    request,
  }) => {
    // 5. An unlinked account cannot restrict anything.
    const stranger = await api(
      request,
      VIEWER,
      "POST",
      "/api/profile/fields/biography/restrict",
    );
    expect(stranger.status).toBe(403);
    expect(JSON.stringify(stranger.body)).not.toContain(RESTRICTABLE);

    // 8. Unknown and non-family fields are refused without revealing content.
    for (const field of ["not_a_field", "my_story"]) {
      const invalid = await api(
        request,
        OWNER,
        "POST",
        `/api/profile/fields/${field}/restrict`,
      );
      expect(invalid.status).toBe(422);
      expect(JSON.stringify(invalid.body)).not.toContain(RESTRICTABLE);
    }

    // 7. A deactivated identity cannot restrict, even with a live link.
    const deactivate = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/users/e2e-restrict-owner/deactivate?issuer=${encodeURIComponent(ISSUER)}`,
    );
    expect(deactivate.status, JSON.stringify(deactivate.body)).toBe(200);
    const afterDeactivation = await api(
      request,
      OWNER,
      "POST",
      "/api/profile/fields/biography/restrict",
    );
    expect([401, 403]).toContain(afterDeactivation.status);
    expect(JSON.stringify(afterDeactivation.body)).not.toContain(RESTRICTABLE);

    await useToken(page, OWNER);
    await page.goto("/profile");
    await expect(
      page.getByTestId("profile-signin").or(page.getByTestId("denied-view")),
    ).toBeVisible();
    await expect(page.getByTestId("profile-field-biography")).toHaveCount(0);
  });
});

// Restore the dedicated owner even when a test fails, so later specs and the
// next harness run see the fixture as seeded.
test.afterAll(async ({ request }) => {
  if (!READY) return;
  await api(request, ADMIN, "POST", "/api/admin/users", {
    issuer: ISSUER,
    subject: "e2e-restrict-owner",
  });
});
