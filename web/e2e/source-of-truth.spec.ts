import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Admin console for the "current source of truth" decision.
//
// The API is stubbed so the scope controls, the preview, the confirmation
// payload, and the reversal can be checked deterministically. Rust owns the
// authorization, the transactional apply, and the staleness check.

const FORBIDDEN = [
  /postgres(ql)?:\/\//i,
  /client_secret/i,
  /access_token/i,
  /refresh_token/i,
  /database_url/i,
];

async function assertNoSensitive(page: Page) {
  const body = await page.content();
  for (const pattern of FORBIDDEN) {
    expect(body, `DOM leaked ${pattern}`).not.toMatch(pattern);
  }
}

type Role = "viewer" | "editor" | "admin";

async function signIn(page: Page, role: Role) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        state: "signed_in",
        role,
        display_name: `Signed-in ${role}`,
      }),
    }),
  );
}

const SUBMISSION = {
  version_id: 7,
  document_key: "demo-card",
  version_number: 1,
  corpus: "family-docs",
  source: "demo-card-md-1234",
  title: "Synthetic card",
  description: null,
  category: "story",
  origin: "submission",
  access_scope: "family",
  review_status: "approved",
  processing_status: "ready",
  content_sha256: "a".repeat(64),
  byte_size: 200,
  mime_type: "text/markdown",
  original_filename: "card.md",
  reviewed_by: "admin-subject",
  reviewed_at: "2026-01-01T00:00:00Z",
  review_note: null,
  removed_at: null,
  created_by: "admin-subject",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};

function assertion(overrides: Record<string, unknown>) {
  return {
    assertion_id: 1,
    person_key: "P100",
    display_name: "Minh An Tran",
    kind: "display_name",
    key: "display:one",
    value_text: "Minh An Tran",
    review_status: "unverified",
    citations: [
      {
        document_ref: "demo-card-md-1234",
        page: 1,
        locator: "line 1",
        excerpt: "Person: Minh An Tran",
        label: "demo-card-md-1234 — line 1",
      },
    ],
    ...overrides,
  };
}

const PREVIEW = {
  document_version_id: 7,
  document_ref: "demo-card-md-1234",
  content_sha256: "a".repeat(64),
  scope: "document",
  person_key: null,
  items: [
    {
      action: "replaced",
      incoming: assertion({ assertion_id: 11, value_text: "Minh Anh Tran" }),
      replaces: [
        assertion({
          assertion_id: 10,
          value_text: "Minh An Tran",
          review_status: "verified",
        }),
      ],
    },
    {
      action: "added",
      incoming: assertion({
        assertion_id: 12,
        kind: "parent_child",
        key: "parent_child:one",
        value_text: "Hoang Van Tran",
      }),
      replaces: [],
    },
  ],
  omissions: [
    assertion({
      assertion_id: 20,
      kind: "name_variant",
      key: "variant:old",
      value_text: "minhan",
    }),
  ],
  conflicts: [
    {
      code: "replaces_verified",
      assertion: assertion({
        assertion_id: 10,
        value_text: "Minh An Tran",
        review_status: "verified",
      }),
    },
  ],
  preview_hash: "0123456789abcdef",
};

function decision(overrides: Record<string, unknown> = {}) {
  return {
    decision_id: 5,
    scope: "document",
    kind: "designate",
    status: "active",
    document_version_id: 7,
    document_ref: "demo-card-md-1234",
    content_sha256: "a".repeat(64),
    person_key: null,
    reason: "Newer reviewed submission",
    actor: "admin-subject",
    created_at: "2026-01-01T00:00:00Z",
    retired_at: null,
    reverses_decision_id: null,
    supersedes_decision_id: null,
    items: PREVIEW.items,
    needs_attention: false,
    unsupported_assertions: [],
    ...overrides,
  };
}

test.describe("current source of truth", () => {
  test("a viewer is denied the console", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/admin/source-of-truth");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("authority-intro")).toHaveCount(0);
  });

  test("previews the scoped effect and applies only after confirmation", async ({
    page,
  }) => {
    const applies: unknown[] = [];
    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
      }),
    );
    await page.route("**/api/genealogy/authority", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, decisions: [] }),
      }),
    );
    await page.route(
      "**/api/genealogy/documents/7/authority/preview",
      (route) =>
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ ok: true, preview: PREVIEW }),
        }),
    );
    await page.route(
      "**/api/genealogy/documents/7/authority/apply",
      async (route) => {
        applies.push(route.request().postDataJSON());
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({
            ok: true,
            decision: {
              decision_id: 5,
              status: "active",
              applied: 2,
              superseded: 1,
              removed: 0,
            },
          }),
        });
      },
    );

    await page.goto("/admin/source-of-truth");
    await expect(page.getByTestId("authority-intro")).toBeVisible();
    await expect(page.getByTestId("authority-intro")).toContainText(
      "Authoritative is not verified",
    );
    await page.getByTestId("authority-preview").click();

    await expect(page.getByTestId("authority-preview-panel")).toBeVisible();
    await expect(page.getByTestId("authority-item")).toHaveCount(2);
    await expect(page.locator('[data-action="replaced"]')).toContainText(
      "Replaces",
    );
    await expect(page.getByTestId("authority-conflicts")).toContainText(
      "replaces verified",
    );
    // The omission is listed but stays a removal only when ticked.
    await expect(page.getByTestId("authority-omission")).toHaveCount(1);

    const apply = page.getByTestId("authority-apply");
    await expect(apply).toBeDisabled();
    await page.getByTestId("authority-reason").fill("Newer reviewed source");
    await page.getByTestId("authority-confirm").check();
    await expect(apply).toBeDisabled();
    await page.getByTestId("authority-acknowledge").check();
    await expect(apply).toBeEnabled();
    await apply.click();

    await expect(page.getByTestId("authority-message")).toContainText(
      "Decision 5 applied",
    );
    expect(applies).toHaveLength(1);
    expect(applies[0]).toMatchObject({
      scope: "document",
      reason: "Newer reviewed source",
      preview_hash: "0123456789abcdef",
      confirm: true,
      acknowledge_conflicts: true,
      remove_assertion_ids: [],
    });
    await assertNoSensitive(page);
  });

  test("a checked omission is sent as an explicit removal", async ({
    page,
  }) => {
    const previews: unknown[] = [];
    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
      }),
    );
    await page.route("**/api/genealogy/authority", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, decisions: [] }),
      }),
    );
    await page.route(
      "**/api/genealogy/documents/7/authority/preview",
      async (route) => {
        previews.push(route.request().postDataJSON());
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify({ ok: true, preview: PREVIEW }),
        });
      },
    );

    await page.goto("/admin/source-of-truth");
    await page.getByTestId("authority-preview").click();
    await page.getByTestId("authority-omission").check();
    await expect(page.getByTestId("authority-removals-pending")).toContainText(
      "1 record(s) selected for removal",
    );
    await page.getByTestId("authority-preview").click();
    expect(previews).toHaveLength(2);
    expect(previews[1]).toMatchObject({ remove_assertion_ids: [20] });
  });

  test("the person scope requires a stable person key", async ({ page }) => {
    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
      }),
    );
    await page.route("**/api/genealogy/authority", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, decisions: [] }),
      }),
    );
    await page.route(
      "**/api/genealogy/documents/7/authority/preview",
      (route) =>
        route.fulfill({
          status: 422,
          contentType: "application/json",
          body: JSON.stringify({
            ok: false,
            error: "authority_invalid",
            message: "the person scope requires a person key",
          }),
        }),
    );

    await page.goto("/admin/source-of-truth");
    await page.getByTestId("authority-scope-person").check();
    await expect(page.getByTestId("authority-person")).toBeVisible();
    await page.getByTestId("authority-preview").click();
    await expect(page.getByTestId("authority-error")).toContainText(
      "person scope requires a person key",
    );
  });

  test("a withdrawn source is reported as needing attention", async ({
    page,
  }) => {
    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
      }),
    );
    await page.route("**/api/genealogy/authority", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ok: true,
          decisions: [
            decision({
              needs_attention: true,
              unsupported_assertions: [
                assertion({
                  assertion_id: 12,
                  kind: "name_variant",
                  value_text: "hoaarchive",
                  citations: [
                    {
                      document_ref: "demo-card-md-1234",
                      page: 1,
                      locator: "line 2",
                      excerpt: "Also known as: hoaarchive",
                      label: "demo-card-md-1234 — line 2",
                      withdrawn: true,
                    },
                  ],
                }),
              ],
            }),
          ],
        }),
      }),
    );

    await page.goto("/admin/source-of-truth");
    await expect(page.getByTestId("authority-attention-badge")).toBeVisible();
    await expect(page.getByTestId("authority-needs-attention")).toContainText(
      "no longer have supporting evidence",
    );
    await expect(page.getByTestId("authority-needs-attention")).toContainText(
      "Reverse this decision",
    );
    // The withdrawn citation is labelled rather than hidden.
    await expect(page.getByTestId("authority-needs-attention")).toContainText(
      "withdrawn",
    );
    await assertNoSensitive(page);
  });

  test("an active decision can be reversed with a recorded reason", async ({
    page,
  }) => {
    const reversals: unknown[] = [];
    let reversed = false;
    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
      }),
    );
    await page.route("**/api/genealogy/authority", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ok: true,
          decisions: reversed
            ? [
                decision({
                  decision_id: 6,
                  kind: "reversal",
                  status: "active",
                  reverses_decision_id: 5,
                  reason: "The earlier value was correct after all",
                  items: [],
                }),
                decision({
                  decision_id: 5,
                  status: "reversed",
                  retired_at: "2026-01-02T00:00:00Z",
                }),
              ]
            : [decision()],
        }),
      }),
    );
    await page.route("**/api/genealogy/authority/5/reverse", async (route) => {
      reversals.push(route.request().postDataJSON());
      reversed = true;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ok: true,
          decision: { decision_id: 6, status: "active" },
        }),
      });
    });

    await page.goto("/admin/source-of-truth");
    await expect(page.getByTestId("authority-decision")).toHaveCount(1);
    await expect(page.getByTestId("authority-decision").first()).toContainText(
      "Reverse this decision",
    );
    const reason = page.getByTestId("authority-reverse-reason").first();
    await reason.fill("The earlier value was correct after all");
    await page.getByTestId("authority-reverse").first().click();

    await expect(page.getByTestId("authority-message")).toContainText(
      "reversed by decision 6",
    );
    expect(reversals).toHaveLength(1);
    expect(reversals[0]).toMatchObject({
      reason: "The earlier value was correct after all",
    });
    await assertNoSensitive(page);
  });
});
