import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-004E administrator review console.
//
// Every fixture is synthetic. The spec creates submitted revisions and pending
// media through the real owner API, then reviews them as a second administrator.
// The Rust API remains the authorization authority; the browser only renders its
// answer and never supplies a reviewer identity.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000").origin;
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const DEACTIVATED = process.env.E2E_DEACTIVATED_TOKEN || "";
const READY = Boolean(
  ISSUER && ADMIN && APPROVER && VIEWER && DEACTIVATED,
);

const PNG_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAH0lEQVR42mPQCLChCmIYNWjUoFGDRg0aNWjUoIE3CAAHDpUQ1ztOKgAAAABJRU5ErkJggg==",
  "base64",
);

const STORY = "Synthetic review story. This is a long private narrative.";
const SELF_STORY = "Synthetic self-submitted family memory.";
const DRAFT_TEXT = "Synthetic unsubmitted review draft must never appear.";
const REJECT_TEXT = "Synthetic rejected biography.";
const NEEDS_TEXT = "Synthetic preferred name needing changes.";
const CONCURRENT_TEXT = "Synthetic concurrent approval value.";

type Fixture = {
  story: number;
  photo: number;
  photoMedia: number;
  document: number;
  documentMedia: number;
  selfRevision: number;
  draftRevision: number;
  rejected: number;
  needsChanges: number;
  concurrent: number;
};

let fixture: Fixture;

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("review console", () => {
  test.beforeAll(async ({ request }) => {
    test.skip(!READY, "review console tokens are unavailable");
    const story = await createPending(
      request,
      APPROVER,
      "my_story",
      STORY,
      "owner_private",
    );
    const photo = await uploadMedia(
      request,
      APPROVER,
      "photo",
      "review-photo.png",
      "image/png",
      PNG_BYTES,
      "family",
    );
    const document = await uploadMedia(
      request,
      APPROVER,
      "document",
      "review-document.txt",
      "text/plain",
      Buffer.from("Synthetic document for review.\n"),
      "owner_private",
    );
    const selfRevision = await createPending(
      request,
      ADMIN,
      "family_memories",
      SELF_STORY,
      "family",
    );
    const draftRevision = await createDraft(
      request,
      APPROVER,
      "current_location",
      DRAFT_TEXT,
      "owner_private",
    );
    const rejected = await createPending(
      request,
      APPROVER,
      "biography",
      REJECT_TEXT,
      "family",
    );
    expect(
      (
        await decide(
          request,
          ADMIN,
          rejected,
          "reject",
          "Synthetic rejection note.",
        )
      ).status,
    ).toBe(200);
    const needsChanges = await createPending(
      request,
      APPROVER,
      "preferred_name",
      NEEDS_TEXT,
      "owner_private",
    );
    expect(
      (
        await decide(
          request,
          ADMIN,
          needsChanges,
          "needs_changes",
          "Synthetic changes note.",
        )
      ).status,
    ).toBe(200);

    const linked = await api(request, ADMIN, "POST", "/api/admin/family-members/links", {
      person_key: "e2e-profile-viewer",
      issuer: ISSUER,
      subject: "e2e-viewer",
      decision: "approve",
    });
    expect([200, 409]).toContain(linked.status);
    const concurrent = await createPending(
      request,
      VIEWER,
      "whatsapp",
      CONCURRENT_TEXT,
      "owner_private",
    );

    fixture = {
      story,
      photo: photo.revision_id,
      photoMedia: photo.media_id,
      document: document.revision_id,
      documentMedia: document.media_id,
      selfRevision,
      draftRevision,
      rejected,
      needsChanges,
      concurrent,
    };
  });

  test("queue shows submitted work, excludes drafts, and supports independent approval", async ({
    page,
  }) => {
    await useToken(page, ADMIN);
    await page.goto("/admin/reviews");
    await expect(page.getByTestId("review-console")).toBeVisible();
    await expect(page.getByTestId("review-filter")).toHaveValue("pending_review");
    await expect(page.getByTestId(`review-row-${fixture.story}`)).toBeVisible();
    await expect(page.getByTestId(`review-row-${fixture.photo}`)).toBeVisible();
    await expect(page.getByTestId(`review-row-${fixture.document}`)).toBeVisible();
    await expect(page.getByTestId(`review-row-${fixture.selfRevision}`)).toBeVisible();
    await expect(page.getByTestId(`review-row-${fixture.draftRevision}`)).toHaveCount(0);
    await expect(page.getByText(DRAFT_TEXT)).toHaveCount(0);

    await page.getByTestId(`review-row-${fixture.story}`).click();
    await expect(page.getByTestId(`review-detail-${fixture.story}`)).toBeVisible();
    await expect(page.getByTestId("review-current")).toContainText(
      "No approved value yet",
    );
    await expect(page.getByTestId("review-proposed")).toContainText(STORY);
    await expect(page.getByTestId("review-note")).toBeVisible();

    await page.getByTestId("review-approve").click();
    await expect(page.getByTestId("review-confirm")).toBeVisible();
    await page.getByTestId("review-confirm-button").click();
    await expect(page.getByTestId("review-message")).toContainText("approved");

    await page.getByTestId("review-filter").selectOption("approved");
    await expect(page.getByTestId(`review-row-${fixture.story}`)).toBeVisible();
    await page.getByTestId(`review-row-${fixture.story}`).click();
    await expect(page.getByTestId("review-current")).toContainText(STORY);
    await expect(page.getByTestId("review-decision-blocked")).toBeVisible();
  });

  test("reviewer notes are required for rejection and requests for changes", async ({
    page,
    request,
  }) => {
    const fresh = await createPending(
      request,
      APPROVER,
      "my_story",
      "Synthetic second narrative for changes.",
      "family",
    );
    await useToken(page, ADMIN);
    await page.goto("/admin/reviews");
    await page.getByTestId(`review-row-${fresh}`).click();
    await expect(page.getByTestId(`review-detail-${fresh}`)).toBeVisible();

    await page.getByTestId("review-needs-changes").click();
    await expect(page.getByTestId("review-confirm")).toBeVisible();
    await expect(page.getByTestId("review-confirm-button")).toBeDisabled();
    await page.getByTestId("review-note").fill("Please add dates.");
    await expect(page.getByTestId("review-confirm-button")).toBeEnabled();
    await page.getByTestId("review-confirm-button").click();
    await expect(page.getByTestId("review-message")).toContainText(
      "Changes requested",
    );

    await page.getByTestId("review-filter").selectOption("needs_changes");
    await expect(page.getByTestId(`review-row-${fresh}`)).toBeVisible();
    await page.getByTestId(`review-row-${fresh}`).click();
    await expect(page.getByTestId("review-events")).toContainText(
      "Please add dates.",
    );

    await page.getByTestId("review-filter").selectOption("rejected");
    await expect(page.getByTestId(`review-row-${fixture.rejected}`)).toBeVisible();
    await page.getByTestId(`review-row-${fixture.rejected}`).click();
    await expect(page.getByTestId("review-events")).toContainText(
      "Synthetic rejection note.",
    );
    await expect(page.getByTestId(`review-detail-${fixture.rejected}`)).toContainText(
      "Rejected",
    );
  });

  test("self-submission is visible but cannot be decided", async ({ page, request }) => {
    await useToken(page, ADMIN);
    await page.goto("/admin/reviews");
    await page.getByTestId(`review-row-${fixture.selfRevision}`).click();
    await expect(
      page.getByTestId(`review-detail-${fixture.selfRevision}`),
    ).toBeVisible();
    await expect(page.getByTestId("review-decision-blocked")).toContainText(
      "independent administrator",
    );
    await expect(page.getByTestId("review-approve")).toHaveCount(0);

    const denied = await decide(
      request,
      ADMIN,
      fixture.selfRevision,
      "approve",
      "",
    );
    expect(denied.status).toBe(403);
  });

  test("non-admin, deactivated, and cross-account access are denied", async ({
    page,
    request,
  }) => {
    await useToken(page, VIEWER);
    await page.goto("/admin/reviews");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("review-console")).toHaveCount(0);

    const viewerQueue = await api(request, VIEWER, "GET", "/api/admin/reviews");
    expect(viewerQueue.status).toBe(403);
    const deactivatedQueue = await api(
      request,
      DEACTIVATED,
      "GET",
      "/api/admin/reviews",
    );
    expect([401, 403, 409]).toContain(deactivatedQueue.status);

    const viewerProfile = await api(request, VIEWER, "GET", "/api/profile");
    expect(viewerProfile.status).toBe(200);
    expect(JSON.stringify(viewerProfile.body)).not.toContain(STORY);
  });

  test("photo and document submissions have review-only authorized previews", async ({
    page,
    request,
  }) => {
    await useToken(page, ADMIN);
    await page.goto("/admin/reviews");

    await page.getByTestId(`review-row-${fixture.photo}`).click();
    await expect(page.getByTestId("review-media-preview")).toBeVisible();
    const pendingProfileMedia = await request.get(
      `${API_URL}/api/profile/media/${fixture.photoMedia}`,
      { headers: auth(ADMIN) },
    );
    expect(pendingProfileMedia.status()).toBe(404);
    const pendingReviewMedia = await request.get(
      `${API_URL}/api/admin/reviews/media/${fixture.photoMedia}`,
      { headers: auth(ADMIN) },
    );
    expect(pendingReviewMedia.status()).toBe(200);

    await page.getByTestId("review-approve").click();
    await page.getByTestId("review-confirm-button").click();
    await expect(page.getByTestId("review-message")).toContainText("approved");
    const closedReviewMedia = await request.get(
      `${API_URL}/api/admin/reviews/media/${fixture.photoMedia}`,
      { headers: auth(ADMIN) },
    );
    expect(closedReviewMedia.status()).toBe(404);
    const approvedFamilyMedia = await request.get(
      `${API_URL}/api/profile/media/${fixture.photoMedia}`,
      { headers: auth(ADMIN) },
    );
    expect(approvedFamilyMedia.status()).toBe(200);

    await page.getByTestId(`review-row-${fixture.document}`).click();
    await expect(page.getByTestId("review-media-download")).toBeVisible();
    await page.getByTestId("review-needs-changes").click();
    await page.getByTestId("review-note").fill("Replace this document.");
    await page.getByTestId("review-confirm-button").click();
    await expect(page.getByTestId("review-message")).toContainText(
      "Changes requested",
    );
    const privateAfterDecision = await request.get(
      `${API_URL}/api/profile/media/${fixture.documentMedia}`,
      { headers: auth(ADMIN) },
    );
    expect(privateAfterDecision.status()).toBe(404);
  });

  test("concurrent decisions have exactly one winner", async ({ page, request }) => {
    const [first, second] = await Promise.all([
      decide(request, ADMIN, fixture.concurrent, "approve", "admin approval"),
      decide(request, APPROVER, fixture.concurrent, "approve", "approver approval"),
    ]);
    expect([first.status, second.status].sort()).toEqual([200, 409]);

    await useToken(page, ADMIN);
    await page.goto("/admin/reviews");
    await page.getByTestId("review-filter").selectOption("approved");
    await expect(page.getByTestId(`review-row-${fixture.concurrent}`)).toBeVisible();
    await page.getByTestId(`review-row-${fixture.concurrent}`).click();
    await expect(page.getByTestId("review-current")).toContainText(CONCURRENT_TEXT);
  });

  test("the review console is usable on mobile and by keyboard", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await useToken(page, ADMIN);
    await page.goto("/admin/reviews");
    await expect(page.getByTestId("review-console")).toBeVisible();
    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);

    const firstRow = page.getByTestId(`review-row-${fixture.selfRevision}`);
    await firstRow.focus();
    await firstRow.press("Enter");
    await expect(
      page.getByTestId(`review-detail-${fixture.selfRevision}`),
    ).toBeVisible();
  });
});

function auth(token: string) {
  return { authorization: `Bearer ${token}` };
}

async function api(
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

async function createDraft(
  request: APIRequestContext,
  token: string,
  fieldKey: string,
  value: string,
  visibility: "owner_private" | "family",
): Promise<number> {
  const created = await api(request, token, "POST", "/api/profile/drafts", {
    field_key: fieldKey,
    value,
    visibility,
  });
  expect(created.status, JSON.stringify(created.body)).toBe(200);
  return created.body.revision.revision_id as number;
}

async function createPending(
  request: APIRequestContext,
  token: string,
  fieldKey: string,
  value: string,
  visibility: "owner_private" | "family",
): Promise<number> {
  const revisionId = await createDraft(
    request,
    token,
    fieldKey,
    value,
    visibility,
  );
  const submitted = await api(
    request,
    token,
    "POST",
    `/api/profile/drafts/${revisionId}/submit`,
  );
  expect(submitted.status, JSON.stringify(submitted.body)).toBe(200);
  return revisionId;
}

async function decide(
  request: APIRequestContext,
  token: string,
  revisionId: number,
  decision: "approve" | "reject" | "needs_changes",
  note: string,
) {
  return api(
    request,
    token,
    "POST",
    `/api/profile/revisions/${revisionId}/review`,
    { decision, note },
  );
}

async function uploadMedia(
  request: APIRequestContext,
  token: string,
  kind: "photo" | "document",
  fileName: string,
  mimeType: string,
  buffer: Buffer,
  visibility: "owner_private" | "family",
): Promise<{ revision_id: number; media_id: number }> {
  const path =
    kind === "photo"
      ? "/api/profile/media/photo"
      : "/api/profile/media/personal-document";
  const response = await request.post(`${API_URL}${path}`, {
    headers: auth(token),
    multipart: {
      file: { name: fileName, mimeType, buffer },
      visibility,
    },
  });
  expect(response.status()).toBe(200);
  const body = (await response.json()) as {
    media: { revision_id: number; media_id: number };
  };
  return body.media;
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
