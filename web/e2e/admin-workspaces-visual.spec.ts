import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Stage-seven mocked suite for the four admin consoles.
//
// No database, API, OIDC issuer, or credentials are needed. Every document,
// person, member, proposal, passage, and decision below is synthetic.

const ADMIN = { state: "signed_in", role: "admin", display_name: "Synthetic admin" };
const VIEWER = { state: "signed_in", role: "viewer", display_name: "Synthetic viewer" };

const SUBMISSION = {
  version_id: 201,
  document_key: "synthetic-letter",
  version_number: 1,
  corpus: "family",
  source: "synthetic-letter.md",
  title: "Synthetic family letter",
  description: "A synthetic letter.",
  category: "letter",
  origin: "upload",
  access_scope: "family",
  review_status: "approved",
  processing_status: "ready",
  content_sha256: "a".repeat(64),
  byte_size: 1234,
  mime_type: "text/markdown",
  original_filename: "synthetic-letter.md",
  reviewed_by: "Synthetic admin",
  reviewed_at: "2026-10-20T12:00:00Z",
  review_note: null,
  removed_at: null,
  created_by: "Synthetic admin",
  created_at: "2026-10-19T12:00:00Z",
  updated_at: "2026-10-20T12:00:00Z",
};

const CANDIDATE = {
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  evidence_status: "verified",
  match_kind: "exact_name",
  matched_name: "Nguyễn Văn An",
};

const EXTRACTION = {
  ok: true,
  extraction: {
    extraction_id: 31,
    document_version_id: 201,
    document_source: "synthetic-letter.md",
    content_sha256: "a".repeat(64),
    status: "draft",
    created_by: "Synthetic admin",
    created_at: "2026-10-21T12:00:00Z",
    applied_by: null,
    applied_at: null,
  },
  proposals: [
    {
      proposal_id: 1,
      ordinal: 1,
      kind: "person",
      subject_name: "Nguyễn Văn An",
      target_name: null,
      resolution: "matched",
      resolved_person_key: "nguyen-van-an",
      candidates: [CANDIDATE],
      value_text: null,
      support_text: "Nguyễn Văn An giữ sổ hộ và ghi chép các thành viên.",
      status: "proposed",
      decided_by: null,
      applied_result: null,
    },
    {
      proposal_id: 2,
      ordinal: 2,
      kind: "alias",
      subject_name: "Nguyễn Văn An",
      target_name: null,
      resolution: "resolved",
      resolved_person_key: "nguyen-van-an",
      candidates: [],
      value_text: "Ông An",
      support_text: "Người trong làng gọi ông là Ông An.",
      status: "proposed",
      decided_by: null,
      applied_result: null,
    },
  ],
};

const PREVIEW_ROWS = [
  {
    person_key: "nguyen-van-an",
    classification: "eligible",
    reasons: [],
    proposed_username: "nguyen.van.an",
    preparable: true,
    blocked_missing_recipient: false,
    sendable: true,
  },
];

const BULK_PREVIEW = {
  ok: true,
  rows: PREVIEW_ROWS,
  summary: {
    eligible_for_invite: 1,
    needs_review: 0,
    already_linked: 0,
    already_prepared: 0,
    invited: 0,
    not_eligible: 0,
    blocked_missing_recipient: 0,
  },
  review_needed: null,
};

const BULK_PREPARE = {
  ok: true,
  batch_id: 41,
  batch_key: "batch-41",
  created: true,
  prepared: 1,
  skipped: 0,
  needs_review: 0,
  failed: 0,
  items: [{ person_key: "nguyen-van-an", status: "prepared", reason: null, reservation_id: 1 }],
};

const BULK_BATCH = {
  ok: true,
  batch_id: 41,
  status: "prepared",
  confirmed_count: 1,
  created_by: "Synthetic admin",
  created_at: "2026-10-21T12:00:00Z",
  completed_at: null,
  items: [
    {
      person_key: "nguyen-van-an",
      status: "prepared",
      reason: null,
      delivery_status: "pending",
      provider: null,
      attempts: 0,
      username: "nguyen.van.an",
      invitation_status: null,
      has_verified_contact: true,
    },
  ],
};

const MEMBERS = {
  ok: true,
  members: [
    {
      user_id: 1,
      issuer: "synthetic-issuer",
      subject: "subject-1",
      email: "an@example.test",
      display_name: "Nguyễn Văn An",
      roles: ["admin" as const],
      status: "active" as const,
      created_at: "2026-10-20T12:00:00Z",
      linked_person: { link_id: 9, person_key: "nguyen-van-an", username: "an" },
    },
    {
      user_id: 2,
      issuer: "synthetic-issuer",
      subject: "subject-2",
      email: null,
      display_name: "Trần Thị Bình",
      roles: ["viewer" as const],
      status: "deactivated" as const,
      created_at: "2026-10-20T12:00:00Z",
      linked_person: null,
    },
  ],
  next_after: null,
  linkage: "available",
};

const PASSAGE = {
  passage_id: 901,
  language: "vi",
  section_title: "Hồi ký gia đình",
  document_ref: "synthetic-memoir.md",
  page_start: 2,
  page_end: 3,
  review_status: "pending",
  visibility: "admin_only",
  body_excerpt: "Đoạn trích hồi ký gia đình.",
  body: "Đoạn một của hồi ký.\n\nĐoạn hai của hồi ký.",
  updated_at: "2026-10-21T12:00:00Z",
};

const NARRATIVE_LINK = {
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  link_status: "resolved",
  match_kind: "exact_name",
  evidence_note: "Synthetic evidence note.",
  reviewed_by: null,
  reviewed_at: null,
};

const NARRATIVE_PERSON = {
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  aliases: ["Ông An"],
  birth: null,
  death: null,
  parents: [],
  spouses: [],
  children: [],
};

const NARRATIVE_QUEUE = {
  ok: true,
  status: "pending",
  language: null,
  person: null,
  unmatched: false,
  link: "all",
  visibility: null,
  document: null,
  page_from: null,
  page_to: null,
  search: null,
  offset: 0,
  limit: 20,
  total: 1,
  truncated: false,
  documents: ["synthetic-memoir.md"],
  entries: [{ passage: PASSAGE, links: [NARRATIVE_LINK] }],
};

const NARRATIVE_DETAIL = {
  ok: true,
  passage: PASSAGE,
  links: [NARRATIVE_LINK],
  people: [NARRATIVE_PERSON],
  events: [
    {
      event_id: 1,
      person_key: "nguyen-van-an",
      actor: "Synthetic admin",
      action: "submitted",
      from_status: null,
      to_status: "pending",
      visibility: null,
      note: null,
      created_at: "2026-10-21T12:00:00Z",
    },
  ],
};

const ASSERTION = {
  assertion_id: 11,
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  kind: "name",
  key: "display_name",
  value_text: "Nguyễn Văn An",
  review_status: "verified",
  citations: [
    {
      document_ref: "synthetic-letter.md",
      page: 1,
      locator: null,
      excerpt: "Nguyễn Văn An.",
      label: "Synthetic letter p.1",
    },
  ],
};

const AUTHORITY_PREVIEW = {
  ok: true,
  preview: {
    document_version_id: 201,
    document_ref: "synthetic-letter.md",
    content_sha256: "a".repeat(64),
    scope: "document",
    person_key: null,
    items: [
      {
        action: "replaced",
        incoming: ASSERTION,
        replaces: [{ ...ASSERTION, assertion_id: 10, value_text: "Nguyễn Văn A" }],
      },
    ],
    omissions: [{ ...ASSERTION, assertion_id: 12 }],
    conflicts: [{ code: "loses_authority", assertion: { ...ASSERTION, assertion_id: 13 } }],
    preview_hash: "preview-hash-abc",
  },
};

const AUTHORITY_DECISIONS = {
  ok: true,
  decisions: [
    {
      decision_id: 5,
      scope: "document",
      kind: "designate",
      status: "active",
      document_version_id: 201,
      document_ref: "synthetic-letter.md",
      content_sha256: "a".repeat(64),
      person_key: null,
      reason: "Synthetic decision reason.",
      actor: "Synthetic admin",
      created_at: "2026-10-21T12:00:00Z",
      retired_at: null,
      reverses_decision_id: null,
      supersedes_decision_id: null,
      items: [{ action: "replaced", incoming: ASSERTION, replaces: [] }],
      needs_attention: true,
      unsupported_assertions: [ASSERTION],
    },
  ],
};

async function signIn(page: Page, session: unknown) {
  await page.route("/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
    }),
  );
}

async function stubSubmissions(page: Page, submissions: unknown[] = [SUBMISSION]) {
  await page.route("**/api/documents/submissions**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, submissions }),
    }),
  );
}

async function stubExtraction(page: Page) {
  await page.route("**/api/genealogy/documents/201/extraction", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(EXTRACTION),
    }),
  );
  await page.route("**/api/genealogy/document-proposals/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(EXTRACTION),
    }),
  );
  await page.route("**/api/genealogy/documents/201/apply", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, extraction_id: 31, already_applied: false, applied: [] }),
    }),
  );
}

async function stubFamilyMembers(page: Page) {
  await page.route("**/api/admin/family-members/bulk/preview", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BULK_PREVIEW),
    }),
  );
  await page.route("**/api/admin/family-members/bulk/prepare", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BULK_PREPARE),
    }),
  );
  await page.route("**/api/admin/family-members/bulk/batches/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BULK_BATCH),
    }),
  );
  await page.route("**/api/admin/family-members/bulk/send", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        configured: true,
        results: [{ person_key: "nguyen-van-an", status: "sent", provider: "synthetic", reason: null }],
      }),
    }),
  );
  await page.route("**/api/admin/family-members", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(MEMBERS),
    }),
  );
  await page.route("**/api/admin/family-members?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(MEMBERS),
    }),
  );
}

async function stubNarrative(page: Page, queue: unknown = NARRATIVE_QUEUE) {
  await page.route("**/api/admin/narrative/passages?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(queue),
    }),
  );
  await page.route("**/api/admin/narrative/passages/901", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(NARRATIVE_DETAIL),
    }),
  );
  await page.route("**/api/admin/narrative/persons?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, people: [NARRATIVE_PERSON] }),
    }),
  );
}

async function stubAuthority(page: Page) {
  await page.route("**/api/genealogy/documents/201/authority/preview", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(AUTHORITY_PREVIEW),
    }),
  );
  await page.route("**/api/genealogy/documents/201/authority/apply", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        decision: { decision_id: 6, status: "active", applied: 1, superseded: 1, removed: 0 },
      }),
    }),
  );
  await page.route("**/api/genealogy/authority**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(AUTHORITY_DECISIONS),
    }),
  );
}

async function assertNoOverflow(page: Page) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, "no horizontal overflow").toBeLessThanOrEqual(1);
}

test.describe("document extraction", () => {
  test("renders the control panel, proposals, and actions", async ({ page }) => {
    await signIn(page, ADMIN);
    await stubSubmissions(page);
    await stubExtraction(page);
    await page.goto("/admin/document-extraction");

    await expect(page.getByTestId("extraction-page")).toBeVisible();
    await expect(page.getByTestId("extraction-document")).toBeVisible();
    await expect(page.getByTestId("extraction-source")).toContainText(
      "synthetic-letter.md",
    );
    await expect(page.getByTestId("extraction-panel")).toBeVisible();
    await expect(page.getByTestId("extraction-status")).toBeVisible();
    await expect(page.getByTestId("proposal")).toHaveCount(2);
    await expect(page.getByTestId("proposal-kind").first()).toBeVisible();
    await expect(page.getByTestId("proposal-status").first()).toContainText(
      "proposed",
    );
    await expect(page.getByTestId("proposal-support").first()).toBeVisible();
    await expect(page.getByTestId("proposal-match")).toBeVisible();
    await expect(page.getByTestId("proposal-value")).toBeVisible();
    await expect(page.getByTestId("proposal-approve").first()).toBeVisible();
    await expect(page.getByTestId("proposal-reject").first()).toBeVisible();
    await expect(page.getByTestId("apply-extraction")).toBeVisible();
  });

  test("shows an explicit empty state and denies viewers", async ({ page }) => {
    await signIn(page, ADMIN);
    await stubSubmissions(page, []);
    await page.route("**/api/genealogy/documents/201/extraction", (route) =>
      route.fulfill({ status: 404, body: "{}" }),
    );
    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("extraction-none")).toBeVisible();

    await signIn(page, VIEWER);
    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("denied-view")).toBeVisible();
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, ADMIN);
    await stubSubmissions(page);
    await stubExtraction(page);
    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("proposal").first()).toBeVisible();
    await assertNoOverflow(page);
  });
});

test.describe("family members and accounts", () => {
  test("confirms prepare and send with Cancel focused and Escape", async ({
    page,
  }) => {
    await signIn(page, ADMIN);
    await stubFamilyMembers(page);
    await page.goto("/admin/family-members");

    await expect(page.getByTestId("bulk-preview")).toBeVisible();
    await expect(page.getByTestId("bulk-table")).toBeVisible();
    await page.getByRole("button", { name: "Select eligible" }).click();
    await page.getByTestId("bulk-prepare").click();

    const prepare = page.getByTestId("bulk-prepare-confirm");
    await expect(prepare).toBeVisible();
    await expect(prepare.getByRole("button", { name: "Cancel" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(prepare).toHaveCount(0);
    await expect(page.getByTestId("bulk-prepare")).toBeFocused();

    await page.getByTestId("bulk-prepare").click();
    await page.getByTestId("bulk-prepare-confirm-button").click();
    await expect(page.getByTestId("bulk-results")).toBeVisible();
    await expect(page.getByTestId("bulk-status-table")).toBeVisible();

    await page.getByTestId("bulk-send").click();
    const send = page.getByTestId("bulk-send-confirm");
    await expect(send).toBeVisible();
    await expect(send.getByRole("button", { name: "Cancel" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(send).toHaveCount(0);
    await expect(page.getByTestId("bulk-send")).toBeFocused();
  });

  test("renders member cards and confirms actions from Cancel", async ({
    page,
  }) => {
    await signIn(page, ADMIN);
    await stubFamilyMembers(page);
    await page.goto("/admin/family-members");

    await expect(page.getByTestId("accounts")).toBeVisible();
    await expect(page.getByTestId("fm-filters")).toBeVisible();
    await expect(page.getByTestId("fm-member")).toHaveCount(2);
    await expect(page.getByTestId("fm-member-status").first()).toBeVisible();
    await expect(page.getByTestId("fm-member-role").first()).toBeVisible();

    await page.getByTestId("fm-actions").first().locator("summary").click();
    await page.getByTestId("fm-reset").first().click();
    const dialog = page.getByTestId("fm-confirm");
    await expect(dialog).toBeVisible();
    await expect(dialog.getByTestId("fm-confirm-cancel")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId("fm-reset").first()).toBeFocused();
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, ADMIN);
    await stubFamilyMembers(page);
    await page.goto("/admin/family-members");
    await expect(page.getByTestId("bulk-preview")).toBeVisible();
    await assertNoOverflow(page);
  });
});

test.describe("narrative review", () => {
  test("renders filters, queue, detail, and decisions", async ({ page }) => {
    await signIn(page, ADMIN);
    await stubNarrative(page);
    await page.goto("/admin/narrative-review");

    await expect(page.getByTestId("narrative-review-console")).toBeVisible();
    await expect(page.getByTestId("narrative-review-filters")).toBeVisible();
    await expect(page.getByTestId("narrative-status-filter")).toBeVisible();
    await page.getByTestId("narrative-review-row-901").click();
    await expect(page.getByTestId("narrative-review-detail-901")).toBeVisible();
    await expect(page.getByTestId("narrative-passage-body")).toContainText(
      "Đoạn một",
    );
    await expect(page.getByTestId("narrative-link-candidates")).toBeVisible();
    await expect(page.getByTestId("narrative-link-confirm")).toBeVisible();
    await expect(page.getByTestId("narrative-link-reject")).toBeVisible();
    await expect(page.getByTestId("narrative-approve-family")).toBeVisible();
    await expect(page.getByTestId("narrative-approve-admin")).toBeVisible();
    await expect(page.getByTestId("narrative-reject-passage")).toBeVisible();
    await expect(page.getByTestId("narrative-pagination")).toBeVisible();
  });

  test("shows an explicit empty state and denies viewers", async ({ page }) => {
    await signIn(page, ADMIN);
    await stubNarrative(page, {
      ...NARRATIVE_QUEUE,
      total: 0,
      entries: [],
    });
    await page.goto("/admin/narrative-review");
    await expect(page.getByTestId("narrative-review-empty")).toBeVisible();

    await signIn(page, VIEWER);
    await page.goto("/admin/narrative-review");
    await expect(page.getByTestId("denied-view")).toBeVisible();
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, ADMIN);
    await stubNarrative(page);
    await page.goto("/admin/narrative-review");
    await page.getByTestId("narrative-review-row-901").click();
    await expect(page.getByTestId("narrative-review-detail-901")).toBeVisible();
    await assertNoOverflow(page);
  });
});

test.describe("source of truth", () => {
  test("renders scope controls, preview, apply gates, and history", async ({
    page,
  }) => {
    await signIn(page, ADMIN);
    await stubSubmissions(page);
    await stubAuthority(page);
    await page.goto("/admin/source-of-truth");

    await expect(page.getByTestId("authority-intro")).toBeVisible();
    await expect(page.getByTestId("authority-document")).toBeVisible();
    await expect(page.getByTestId("authority-scope-document")).toBeVisible();
    await expect(page.getByTestId("authority-history")).toBeVisible();
    await expect(page.getByTestId("authority-decision")).toBeVisible();
    await expect(page.getByTestId("authority-attention-badge")).toBeVisible();

    await page.getByTestId("authority-preview").click();
    await expect(page.getByTestId("authority-preview-panel")).toBeVisible();
    await expect(page.getByTestId("authority-item")).toBeVisible();
    await expect(page.getByTestId("authority-conflicts")).toBeVisible();
    await expect(page.getByTestId("authority-omission")).toBeVisible();

    const apply = page.getByTestId("authority-apply");
    await expect(apply).toBeDisabled();
    await page.getByTestId("authority-reason").fill("Synthetic decision reason.");
    await page.getByTestId("authority-confirm").check();
    await page.getByTestId("authority-acknowledge").check();
    await expect(apply).toBeEnabled();
  });

  test("shows an explicit empty state and denies viewers", async ({ page }) => {
    await signIn(page, ADMIN);
    await stubSubmissions(page, []);
    await stubAuthority(page);
    await page.goto("/admin/source-of-truth");
    await expect(page.getByTestId("authority-none")).toBeVisible();

    await signIn(page, VIEWER);
    await page.goto("/admin/source-of-truth");
    await expect(page.getByTestId("denied-view")).toBeVisible();
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, ADMIN);
    await stubSubmissions(page);
    await stubAuthority(page);
    await page.goto("/admin/source-of-truth");
    await page.getByTestId("authority-preview").click();
    await expect(page.getByTestId("authority-preview-panel")).toBeVisible();
    await assertNoOverflow(page);
  });
});

test("keeps the stage-three light and dark tokens", async ({ page }) => {
  await signIn(page, ADMIN);
  await stubSubmissions(page);
  await stubExtraction(page);
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/admin/document-extraction");
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
