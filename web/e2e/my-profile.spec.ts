import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-004D My Profile browser coverage.
//
// These specs run against the real Axum API, the real Next.js proxy, the real
// PostgreSQL genealogy schema, and the synthetic OIDC issuer started by
// scripts/web-e2e.sh. No test invents authorization: every read and write goes
// through Rust, and the synthetic members/links are disposable.
//
// One spec (network failure) stubs a single response to exercise the retry UI;
// it still uses the real session and never substitutes for server enforcement.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const DEACTIVATED = process.env.E2E_DEACTIVATED_TOKEN || "";
const READY = Boolean(ISSUER && ADMIN && APPROVER && VIEWER && DEACTIVATED);

// A valid 24x24 RGB PNG. The server decodes and re-encodes it, so this proves
// the real image pipeline rather than trusting the client MIME type.
const PNG_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAH0lEQVR42mPQCLChCmIYNWjUoFGDRg0aNWjUoIE3CAAHDpUQ1ztOKgAAAABJRU5ErkJggg==",
  "base64",
);

const PRIVATE_MARKER = "Synthetic Approved Preferred Name";
const PENDING_MARKER = "Synthetic Pending Preferred Name";

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

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

function auth(token: string) {
  return { authorization: `Bearer ${token}` };
}

async function json(
  request: APIRequestContext,
  token: string,
  method: "GET" | "POST" | "PATCH" | "DELETE",
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

async function profile(request: APIRequestContext, token: string) {
  return json(request, token, "GET", "/api/profile");
}

async function createDraft(
  request: APIRequestContext,
  token: string,
  field_key: string,
  value: string,
  visibility = "owner_private",
): Promise<number> {
  const created = await json(request, token, "POST", "/api/profile/drafts", {
    field_key,
    value,
    visibility,
  });
  expect(created.status, JSON.stringify(created.body)).toBe(200);
  return created.body.revision.revision_id as number;
}

async function submitDraft(
  request: APIRequestContext,
  token: string,
  revisionId: number,
) {
  const submitted = await json(
    request,
    token,
    "POST",
    `/api/profile/drafts/${revisionId}/submit`,
  );
  expect(submitted.status, JSON.stringify(submitted.body)).toBe(200);
}

async function review(
  request: APIRequestContext,
  revisionId: number,
  decision: "approve" | "reject" | "needs_changes",
  note: string,
) {
  const reviewed = await json(
    request,
    APPROVER,
    "POST",
    `/api/profile/revisions/${revisionId}/review`,
    { decision, note },
  );
  expect(reviewed.status, JSON.stringify(reviewed.body)).toBe(200);
}

async function approveDraft(
  request: APIRequestContext,
  fieldKey: string,
  value: string,
  visibility = "family",
) {
  const revisionId = await createDraft(
    request,
    ADMIN,
    fieldKey,
    value,
    visibility,
  );
  await submitDraft(request, ADMIN, revisionId);
  await review(request, revisionId, "approve", "Synthetic E2E approval");
  return revisionId;
}

async function assertNoPrivateLeak(page: Page) {
  const body = await page.content();
  for (const forbidden of [
    /postgres(ql)?:\/\//i,
    /storage_key/i,
    /sha256/i,
    /Bearer\s+/,
    /access_token/i,
    /\/data\/media/i,
  ]) {
    expect(body, `DOM leaked ${forbidden}`).not.toMatch(forbidden);
  }
}

async function acceptNextDialog(page: Page) {
  page.once("dialog", (dialog) => {
    void dialog.accept();
  });
}

test.describe.serial("My Profile", () => {
  test("linked member creates, updates, submits, and sees approved values during review", async ({
    page,
    request,
  }) => {
    await useToken(page, ADMIN);

    const approvedRevision = await approveDraft(
      request,
      "preferred_name",
      PRIVATE_MARKER,
      "family",
    );
    expect(approvedRevision).toBeGreaterThan(0);

    await page.goto("/profile");
    await expect(page.getByTestId("profile-header")).toBeVisible();
    await expect(page.getByTestId("profile-name")).toHaveText(
      "Synthetic E2E Admin Member",
    );
    await expect(page.getByText(PRIVATE_MARKER)).toBeVisible();
    await assertNoPrivateLeak(page);

    // Create a draft through the browser.
    await page.getByTestId("profile-add-preferred_name").click();
    const input = page.getByTestId("profile-input-preferred_name");
    await input.fill(PENDING_MARKER);
    await page
      .getByTestId("profile-visibility-preferred_name")
      .selectOption("family");
    await page.getByTestId("profile-save-preferred_name").click();

    const open = page.getByTestId("profile-open-preferred_name");
    await expect(open).toBeVisible();
    await expect(open.getByTestId("status-draft")).toBeVisible();
    // The approved value is still the visible source of truth.
    await expect(page.getByText(PRIVATE_MARKER)).toBeVisible();

    // Update the saved draft, then submit it.
    await page.getByTestId("profile-edit-preferred_name").click();
    await page
      .getByTestId("profile-input-preferred_name")
      .fill(`${PENDING_MARKER} v2`);
    await page.getByTestId("profile-save-preferred_name").click();
    await expect(open.getByText(`${PENDING_MARKER} v2`)).toBeVisible();

    await page.getByTestId("profile-submit-preferred_name").click();
    await expect(open.getByTestId("status-pending_review")).toBeVisible();
    await expect(page.getByText(PRIVATE_MARKER)).toBeVisible();

    // Approve the pending revision independently.
    const current = await profile(request, ADMIN);
    expect(current.status).toBe(200);
    const pending = (
      current.body.profile.pending as { revision_id: number }[]
    ).find((revision) => revision.revision_id > 0);
    expect(pending).toBeTruthy();
    await review(
      request,
      pending!.revision_id,
      "approve",
      "Independent E2E approval",
    );

    await page.reload();
    await expect(page.getByText(`${PENDING_MARKER} v2`)).toBeVisible();
    await expect(page.getByTestId("profile-open-preferred_name")).toHaveCount(
      0,
    );
    await assertNoPrivateLeak(page);
  });

  test("rejected and needs-changes decisions show reviewer feedback and support revision", async ({
    page,
    request,
  }) => {
    await useToken(page, ADMIN);

    const first = await createDraft(
      request,
      ADMIN,
      "biography",
      "First synthetic biography",
    );
    await submitDraft(request, ADMIN, first);
    await review(
      request,
      first,
      "reject",
      "Synthetic rejection: add more detail",
    );

    await page.goto("/profile");
    const feedback = page.getByTestId("profile-feedback-biography");
    await expect(feedback).toBeVisible();
    await expect(feedback.getByTestId("status-rejected")).toBeVisible();
    await expect(feedback).toContainText(
      "Synthetic rejection: add more detail",
    );
    await expect(page.getByText("First synthetic biography")).toBeVisible();
    // The shared navigation flags the decision on every page.
    await page.goto("/genealogy");
    await expect(page.getByTestId("more-toggle")).toHaveAttribute(
      "aria-label",
      /1 profile review decision needs your attention/,
    );
    await page.goto("/profile");

    await page.getByTestId("profile-revise-biography").click();
    await page
      .getByTestId("profile-input-biography")
      .fill("Second synthetic biography");
    await page.getByTestId("profile-save-biography").click();
    await expect(
      page.getByTestId("profile-open-biography").getByTestId("status-draft"),
    ).toBeVisible();
    // A newer draft supersedes the rejection, so nothing is waiting any more.
    await page.reload();
    await expect(page.getByTestId("nav-action-badge")).toHaveCount(0);

    await page.getByTestId("profile-submit-biography").click();
    const pending = await profile(request, ADMIN);
    const revision = (
      pending.body.profile.pending as { revision_id: number }[]
    ).find((item) => item.revision_id > first);
    expect(revision).toBeTruthy();
    await review(
      request,
      revision!.revision_id,
      "needs_changes",
      "Synthetic: add dates",
    );

    await page.reload();
    const needsChanges = page.getByTestId("profile-feedback-biography");
    await expect(
      needsChanges.getByTestId("status-needs_changes"),
    ).toBeVisible();
    await expect(needsChanges).toContainText("Synthetic: add dates");
    await expect(page.getByTestId("more-toggle").getByTestId("nav-action-badge")).toHaveText(
      /1/,
    );
  });

  test("an unlinked member sees a safe explanation and no private values", async ({
    page,
    request,
  }) => {
    await useToken(page, VIEWER);
    await page.goto("/profile");
    await expect(page.getByTestId("profile-unlinked")).toBeVisible();
    await expect(page.getByTestId("profile-header")).toHaveCount(0);
    await expect(page.locator('[data-testid^="profile-field-"]')).toHaveCount(
      0,
    );
    await expect(page.getByText(PRIVATE_MARKER)).toHaveCount(0);

    const direct = await profile(request, VIEWER);
    expect(direct.status).toBe(200);
    expect(direct.body.linked).toBe(false);
    expect(JSON.stringify(direct.body)).not.toContain(PRIVATE_MARKER);
  });

  test("a revoked account-person link removes profile access", async ({
    page,
    request,
  }) => {
    const linked = await json(
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
    expect(linked.body.linked).toBe(true);

    await useToken(page, VIEWER);
    await page.goto("/profile");
    await expect(page.getByTestId("profile-name")).toHaveText(
      "Synthetic E2E Viewer Member",
    );

    const revoked = await json(
      request,
      ADMIN,
      "POST",
      `/api/admin/family-members/links/${linked.body.link_id}/revoke`,
    );
    expect(revoked.status, JSON.stringify(revoked.body)).toBe(200);

    await page.reload();
    await expect(page.getByTestId("profile-unlinked")).toBeVisible();

    const direct = await profile(request, VIEWER);
    expect(direct.body.linked).toBe(false);
  });

  test("another linked account cannot read or modify an owner's private revision", async ({
    page,
    request,
  }) => {
    const privateValue = "Synthetic private location alpha";
    const revisionId = await createDraft(
      request,
      ADMIN,
      "current_location",
      privateValue,
    );

    await useToken(page, APPROVER);
    await page.goto("/profile");
    await expect(page.getByTestId("profile-name")).toHaveText(
      "Synthetic E2E Approver Member",
    );
    await expect(page.getByText(privateValue)).toHaveCount(0);

    const other = await profile(request, APPROVER);
    expect(other.status).toBe(200);
    expect(JSON.stringify(other.body)).not.toContain(privateValue);

    const modified = await json(
      request,
      APPROVER,
      "PATCH",
      `/api/profile/drafts/${revisionId}`,
      { value: "cross-account overwrite" },
    );
    expect([403, 404]).toContain(modified.status);

    const cancelled = await json(
      request,
      APPROVER,
      "POST",
      `/api/profile/drafts/${revisionId}/cancel`,
    );
    expect([403, 404, 405, 409]).toContain(cancelled.status);
  });

  test("a deactivated identity receives no private profile values", async ({
    page,
    request,
  }) => {
    const deactivated = await json(
      request,
      ADMIN,
      "POST",
      `/api/admin/users/e2e-deactivated/deactivate?issuer=${encodeURIComponent(ISSUER)}`,
    );
    expect(deactivated.status, JSON.stringify(deactivated.body)).toBe(200);

    await useToken(page, DEACTIVATED);
    await page.goto("/profile");
    // A deactivated identity is no longer provisioned. The session service
    // reports an anonymous session, so the profile must render the safe sign-in
    // invitation (or an explicit denial) rather than an endless loader.
    await expect(page.getByTestId("profile-signin")).toBeVisible();
    await expect(page.getByTestId("profile-header")).toHaveCount(0);
    await expect(page.getByTestId("profile-loading")).toHaveCount(0);
    await expect(
      page.getByText("Synthetic E2E Deactivated Member"),
    ).toHaveCount(0);

    const direct = await profile(request, DEACTIVATED);
    expect(direct.status).toBe(403);
    expect(JSON.stringify(direct.body)).not.toContain(
      "Synthetic E2E Deactivated Member",
    );
  });

  test("photo upload is private until review, blocks cross-account reads, and can be withdrawn", async ({
    page,
    request,
  }) => {
    await useToken(page, ADMIN);
    await page.goto("/profile");

    await page.getByTestId("profile-photo-input").setInputFiles({
      name: "synthetic-profile.png",
      mimeType: "image/png",
      buffer: PNG_BYTES,
    });

    const current = page.getByTestId("profile-photo-current");
    await expect(current).toBeVisible();
    await expect(current.getByTestId("status-pending_review")).toBeVisible();
    await expect(page.getByTestId("profile-photo-withdraw")).toBeVisible();
    await assertNoPrivateLeak(page);

    const ownerView = await profile(request, ADMIN);
    const media = (
      ownerView.body.media as { media_id: number; status: string }[]
    ).find((asset) => asset.status === "pending_review");
    expect(media).toBeTruthy();

    const ownerDownload = await request.get(
      `${API_URL}/api/profile/media/${media!.media_id}`,
      { headers: auth(ADMIN) },
    );
    expect(ownerDownload.status()).toBe(200);

    const crossAccount = await request.get(
      `${API_URL}/api/profile/media/${media!.media_id}`,
      { headers: auth(APPROVER) },
    );
    expect([403, 404]).toContain(crossAccount.status());

    const anonymous = await request.get(
      `${API_URL}/api/profile/media/${media!.media_id}`,
    );
    expect(anonymous.status()).toBe(401);

    await page.reload();
    await expect(
      page
        .getByTestId("profile-photo-current")
        .getByTestId("status-pending_review"),
    ).toBeVisible();

    await acceptNextDialog(page);
    await page.getByTestId("profile-photo-withdraw").click();
    await expect(page.getByTestId("profile-photo-empty")).toBeVisible();

    const withdrawn = await request.get(
      `${API_URL}/api/profile/media/${media!.media_id}`,
      { headers: auth(ADMIN) },
    );
    expect(withdrawn.status()).toBe(404);
  });

  test("document upload validates types and sizes and supports authorized retrieval", async ({
    page,
    request,
  }) => {
    await useToken(page, ADMIN);
    await page.goto("/profile");

    // Client-side type validation.
    await page.getByTestId("profile-photo-input").setInputFiles({
      name: "not-a-photo.pdf",
      mimeType: "application/pdf",
      buffer: Buffer.from("not an image"),
    });
    await expect(
      page.getByText("Choose a JPEG or PNG photograph."),
    ).toBeVisible();

    // Client-side size validation.
    await page.getByTestId("profile-photo-input").setInputFiles({
      name: "too-large.png",
      mimeType: "image/png",
      buffer: Buffer.alloc(5 * 1024 * 1024 + 1),
    });
    await expect(
      page.getByText("Photographs must be 5 MiB or smaller."),
    ).toBeVisible();

    // Server-side malformed image rejection.
    const malformed = await request.post(`${API_URL}/api/profile/media/photo`, {
      headers: auth(ADMIN),
      multipart: {
        file: {
          name: "malformed.png",
          mimeType: "image/png",
          buffer: Buffer.from("definitely not a png"),
        },
      },
    });
    expect(malformed.status()).toBe(400);

    // A valid personal document uploads and is retrievable only by its owner.
    const text = "Synthetic personal document for the browser E2E suite.\n";
    await page.getByTestId("profile-document-input").setInputFiles({
      name: "synthetic-document.txt",
      mimeType: "text/plain",
      buffer: Buffer.from(text),
    });
    const current = page.getByTestId("profile-document-current");
    await expect(current).toBeVisible();
    await expect(current.getByTestId("status-pending_review")).toBeVisible();
    await expect(
      current.getByTestId("profile-document-download"),
    ).toBeVisible();

    const ownerView = await profile(request, ADMIN);
    const document = (
      ownerView.body.media as { media_id: number; kind: string }[]
    ).find((asset) => asset.kind === "personal_document");
    expect(document).toBeTruthy();

    const download = await request.get(
      `${API_URL}/api/profile/media/${document!.media_id}`,
      { headers: auth(ADMIN) },
    );
    expect(download.status()).toBe(200);
    expect(await download.text()).toBe(text);

    const crossAccount = await request.get(
      `${API_URL}/api/profile/media/${document!.media_id}`,
      { headers: auth(APPROVER) },
    );
    expect([403, 404]).toContain(crossAccount.status());

    await acceptNextDialog(page);
    await page.getByTestId("profile-document-withdraw").click();
    await expect(page.getByTestId("profile-document-empty")).toBeVisible();
  });

  test("an expired session is rejected without profile detail", async ({
    page,
  }) => {
    await useToken(page, "expired-or-invalid-token");
    await page.goto("/profile");
    await expect(page.getByTestId("profile-expired")).toBeVisible();
    await expect(page.getByTestId("profile-header")).toHaveCount(0);
    await expect(page.getByText(PRIVATE_MARKER)).toHaveCount(0);
    await assertNoPrivateLeak(page);
  });

  test("a failed profile response shows a retry path and can recover", async ({
    page,
  }) => {
    await useToken(page, ADMIN);
    await page.route("**/api/profile", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({
          ok: false,
          error: "unavailable",
          message: "The profile service is unavailable.",
        }),
      }),
    );
    await page.goto("/profile");
    await expect(page.getByTestId("profile-retry")).toBeVisible();
    await expect(
      page.getByText("The profile service is unavailable."),
    ).toBeVisible();

    await page.unroute("**/api/profile");
    await page.getByTestId("profile-retry").click();
    await expect(page.getByTestId("profile-header")).toBeVisible();
  });

  test("the profile layout is usable on a mobile viewport and with the keyboard", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await useToken(page, ADMIN);
    await page.goto("/profile");
    await expect(page.getByTestId("nav-toggle")).toBeVisible();
    await expect(page.getByTestId("profile-header")).toBeVisible();

    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow, "no horizontal overflow on mobile").toBeLessThanOrEqual(1);

    const add = page.getByTestId("profile-add-whatsapp");
    await add.focus();
    await add.press("Enter");
    await expect(page.getByTestId("profile-input-whatsapp")).toBeVisible();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
  });
});
