import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Stage-five mocked workspace suite for documents and review consoles.
//
// No database, API, OIDC issuer, or credentials are needed. Every document,
// person, branch, proposal, decision, and citation below is synthetic.

const SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const DOCUMENTS = {
  ok: true,
  submissions: [
    {
      version_id: 101,
      document_key: "synthetic-family-letter",
      version_number: 2,
      corpus: "family",
      source: "synthetic-family-letter.txt",
      title: "Synthetic family letter",
      description: "A synthetic letter for the workspace visual suite.",
      category: "letter",
      origin: "upload",
      access_scope: "family",
      review_status: "approved",
      processing_status: "ready",
      content_sha256: "a".repeat(64),
      byte_size: 1234,
      mime_type: "text/plain",
      original_filename: "synthetic-family-letter.txt",
      reviewed_by: "Synthetic admin",
      reviewed_at: "2026-10-20T12:00:00Z",
      review_note: null,
      removed_at: null,
      created_by: "Synthetic admin",
      created_at: "2026-10-19T12:00:00Z",
      updated_at: "2026-10-20T12:00:00Z",
    },
    {
      version_id: 102,
      document_key: "synthetic-register",
      version_number: 1,
      corpus: "family",
      source: "synthetic-register.pdf",
      title: "Synthetic family register",
      description: "A synthetic register awaiting review.",
      category: "genealogy",
      origin: "upload",
      access_scope: "admin_only",
      review_status: "pending_review",
      processing_status: "queued",
      content_sha256: "b".repeat(64),
      byte_size: 9876,
      mime_type: "application/pdf",
      original_filename: "synthetic-register.pdf",
      reviewed_by: null,
      reviewed_at: null,
      review_note: null,
      removed_at: null,
      created_by: "Synthetic admin",
      created_at: "2026-10-21T12:00:00Z",
      updated_at: "2026-10-21T12:00:00Z",
    },
  ],
};

const JOBS = {
  ok: true,
  jobs: [
    {
      job_id: 11,
      version_id: 101,
      status: "failed",
      attempts: 3,
      max_attempts: 3,
      last_error: "Synthetic processing failure.",
      created_at: "2026-10-20T12:00:00Z",
      updated_at: "2026-10-20T12:05:00Z",
    },
  ],
};

const REVIEW_ENTRY = {
  display_name: "Nguyễn Văn An",
  revision: {
    revision_id: 501,
    field_key: "biography",
    revision_number: 3,
    prior_value_text: "Approved synthetic biography.",
    prior_approved_revision_number: 2,
    value:
      "Nguyễn Văn An kept the family register and recorded the household at Example Hamlet.",
    visibility: "family",
    status: "pending_review",
    changed_at: "2026-10-21T12:00:00Z",
    submitted_at: "2026-10-21T12:05:00Z",
    reviewed_at: null,
    review_note: null,
  },
  current_approved: {
    field_key: "biography",
    value: "Approved synthetic biography.",
    visibility: "family",
    approved_revision_number: 2,
    approved_revision_id: 20,
    updated_at: "2026-10-20T12:00:00Z",
    restricted_from_family: false,
    restriction_generation: null,
  },
  current_media: null,
  media: null,
  can_decide: true,
  decision_block: null,
};

const REVIEW_QUEUE = {
  ok: true,
  status: "pending_review",
  limit: 20,
  next_cursor: null,
  entries: [REVIEW_ENTRY],
};

const REVIEW_DETAIL = {
  ok: true,
  entry: REVIEW_ENTRY,
  events: [
    {
      event: "submitted",
      actor: "Synthetic owner",
      note: null,
      created_at: "2026-10-21T12:05:00Z",
    },
  ],
  history: [
    {
      revision_id: 20,
      field_key: "biography",
      revision_number: 2,
      prior_value_text: null,
      prior_approved_revision_number: 1,
      value: "Approved synthetic biography.",
      visibility: "family",
      status: "approved",
      changed_at: "2026-10-20T12:00:00Z",
      submitted_at: "2026-10-20T12:00:00Z",
      reviewed_at: "2026-10-20T12:05:00Z",
      review_note: "Synthetic approval.",
    },
  ],
};

const BRANCH_PROPOSAL = {
  proposal_id: 701,
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  branch_key: "branch-1",
  assertion_kind: "name",
  assertion_key: "display_name",
  prior_value_text: "Nguyễn Văn An",
  prior_review_status: "verified",
  proposed_value_text: "Nguyễn Văn An (synthetic correction)",
  evidence: [
    {
      source_evidence_id: 1,
      document_ref: "synthetic/register.pdf",
      page: 4,
    },
  ],
  submitted_at: "2026-10-21T12:00:00Z",
  status: "pending_review",
  can_decide: true,
  decision_block: null,
};

const BRANCH_QUEUE = {
  ok: true,
  status: "pending_review",
  limit: 20,
  next_cursor: null,
  submissions: [BRANCH_PROPOSAL],
};

const BRANCH_DETAIL = {
  ok: true,
  proposal: BRANCH_PROPOSAL,
  current_assertion: {
    assertion_id: 11,
    value_text: "Nguyễn Văn An",
    review_status: "verified",
    reviewed_at: "2026-10-20T12:00:00Z",
  },
  events: [
    {
      event: "submitted",
      actor: "Synthetic reviewer",
      note: null,
      created_at: "2026-10-21T12:00:00Z",
    },
  ],
};

const BRANCH_EVIDENCE = {
  ok: true,
  evidence: [
    {
      source_evidence_id: 2,
      document_ref: "synthetic/letters.pdf",
      page: 2,
      locator: "entry 4",
      assertion_kind: "name",
      assertion_key: "alias",
    },
  ],
};

async function session(page: Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(SESSION),
    }),
  );
}

async function stubDocuments(page: Page) {
  await session(page);
  await page.route("**/api/documents/submissions**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(DOCUMENTS),
    }),
  );
  await page.route("**/api/documents/jobs", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(JOBS),
    }),
  );
}

async function stubReview(page: Page) {
  await session(page);
  await page.route("**/api/admin/reviews?*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(REVIEW_QUEUE),
    }),
  );
  await page.route("**/api/admin/reviews/501", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(REVIEW_DETAIL),
    }),
  );
}

async function stubBranch(page: Page) {
  await session(page);
  await page.route("**/api/branch-review/grants/me", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        grants: [{ branch_key: "branch-1", branch_label: "First Branch" }],
      }),
    }),
  );
  await page.route("**/api/branch-review/submissions?*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BRANCH_QUEUE),
    }),
  );
  await page.route("**/api/branch-review/submissions/701", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BRANCH_DETAIL),
    }),
  );
  await page.route("**/api/branch-review/persons/**/evidence", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BRANCH_EVIDENCE),
    }),
  );
}

async function assertNoOverflow(page: Page) {
  const overflow = await page.evaluate(
    () =>
      document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, "no horizontal overflow").toBeLessThanOrEqual(1);
}

test.describe("documents workspace", () => {
  test("renders the upload panel, processing queue, submissions, and actions", async ({
    page,
  }) => {
    await stubDocuments(page);
    await page.goto("/documents");

    await expect(page.getByTestId("document-upload")).toBeVisible();
    await expect(page.getByTestId("document-file")).toBeVisible();
    await expect(page.getByTestId("submit-document")).toBeVisible();
    await expect(page.getByTestId("document-jobs")).toContainText(
      "Processing queue",
    );
    await expect(page.getByTestId("document-jobs")).toContainText("Retry");

    const approved = page.getByTestId("document-row-101");
    await expect(approved).toBeVisible();
    await expect(approved).toContainText("Synthetic family letter");
    await expect(approved).toContainText("approved");
    await expect(approved).toContainText("ready");
    await expect(approved).toContainText("family");
    await expect(
      approved.getByRole("link", { name: "Preview" }),
    ).toHaveAttribute("href", "/api/documents/submissions/101/content");
    await expect(
      approved.getByRole("button", { name: "Approve" }),
    ).toBeVisible();

    const pending = page.getByTestId("document-row-102");
    await expect(pending).toContainText("pending_review");
    await expect(pending).toContainText("queued");
    await expect(page.locator(".status-chip").first()).toBeVisible();
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stubDocuments(page);
    await page.goto("/documents");
    await expect(page.getByTestId("document-row-101")).toBeVisible();
    await assertNoOverflow(page);
  });
});

test.describe("admin review console", () => {
  test("shows queue, comparison, required note, decisions, and history", async ({
    page,
  }) => {
    await stubReview(page);
    await page.goto("/admin/reviews");

    await expect(page.getByTestId("review-console")).toBeVisible();
    await expect(page.getByTestId("review-filter")).toHaveValue(
      "pending_review",
    );
    await page.getByTestId("review-row-501").click();
    await expect(page.getByTestId("review-detail-501")).toBeVisible();
    await expect(page.getByTestId("review-current")).toContainText(
      "Approved synthetic biography",
    );
    await expect(page.getByTestId("review-proposed")).toContainText(
      "kept the family register",
    );
    await expect(page.getByTestId("review-events")).toContainText(
      "Synthetic owner",
    );

    await page.getByTestId("review-needs-changes").click();
    await expect(page.getByTestId("review-confirm")).toBeVisible();
    await expect(page.getByTestId("review-confirm-button")).toBeDisabled();
    await page.getByTestId("review-note").fill("Synthetic reviewer note.");
    await expect(page.getByTestId("review-confirm-button")).toBeEnabled();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(page.getByTestId("review-confirm")).toHaveCount(0);
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stubReview(page);
    await page.goto("/admin/reviews");
    await page.getByTestId("review-row-501").click();
    await expect(page.getByTestId("review-detail-501")).toBeVisible();
    await assertNoOverflow(page);
  });
});

test.describe("branch review console", () => {
  test("shows filters, queue, evidence, note, decisions, and history", async ({
    page,
  }) => {
    await stubBranch(page);
    await page.goto("/branch-review");

    await expect(page.getByTestId("branch-review-console")).toBeVisible();
    await expect(page.getByTestId("branch-review-branch-filter")).toContainText(
      "First Branch",
    );
    await expect(
      page.getByTestId("branch-review-status-filter"),
    ).toHaveValue("pending_review");
    await page.getByTestId("branch-review-row-701").click();
    await expect(page.getByTestId("branch-review-detail-701")).toBeVisible();
    await expect(page.getByTestId("branch-review-current")).toContainText(
      "Nguyễn Văn An",
    );
    await expect(page.getByTestId("branch-review-proposed")).toContainText(
      "synthetic correction",
    );
    await expect(
      page.getByTestId("branch-review-proposal-evidence"),
    ).toContainText("synthetic/register.pdf");
    await expect(
      page.getByTestId("branch-review-person-evidence"),
    ).toContainText("synthetic/letters.pdf");

    await page.getByTestId("branch-review-reject").click();
    await expect(page.getByTestId("branch-review-confirm")).toBeVisible();
    await expect(
      page.getByTestId("branch-review-confirm-button"),
    ).toBeDisabled();
    await page
      .getByTestId("branch-review-note")
      .fill("Synthetic branch reviewer note.");
    await expect(
      page.getByTestId("branch-review-confirm-button"),
    ).toBeEnabled();
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stubBranch(page);
    await page.goto("/branch-review");
    await page.getByTestId("branch-review-row-701").click();
    await expect(page.getByTestId("branch-review-detail-701")).toBeVisible();
    await assertNoOverflow(page);
  });
});

test("keeps the stage-three light and dark tokens", async ({ page }) => {
  await stubDocuments(page);
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/documents");
  await expect
    .poll(async () =>
      page.evaluate(() =>
        getComputedStyle(document.documentElement)
          .getPropertyValue("--bg")
          .trim()
          .toLowerCase(),
      ),
    )
    .toBe("#fafafb");

  await page.emulateMedia({ colorScheme: "dark" });
  await expect
    .poll(async () =>
      page.evaluate(() =>
        getComputedStyle(document.documentElement)
          .getPropertyValue("--bg")
          .trim()
          .toLowerCase(),
      ),
    )
    .toBe("#08090a");
});
