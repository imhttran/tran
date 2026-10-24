import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Admin review screen for the reviewed document-to-genealogy workflow.
//
// These tests stub the API so the rendering, the decision payloads, and the
// apply step can be checked deterministically. Rust owns the authorization and
// the transactional apply; the mocked replies never stand in for that.

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

function proposal(overrides: Record<string, unknown>) {
  return {
    proposal_id: 0,
    ordinal: 1,
    kind: "person",
    subject_name: "Minh An Tran",
    target_name: null,
    resolution: "ambiguous",
    resolved_person_key: null,
    candidates: [],
    value_text: null,
    support_text: "Person: Minh An Tran",
    status: "proposed",
    decided_by: null,
    applied_result: null,
    ...overrides,
  };
}

const PROPOSALS = [
  proposal({
    proposal_id: 1,
    ordinal: 1,
    candidates: [
      {
        person_key: "P100",
        display_name: "Minh An Tran",
        evidence_status: "verified",
        match_kind: "exact",
        matched_name: "Minh An Tran",
      },
      {
        person_key: "P101",
        display_name: "Minh An Tran",
        evidence_status: "unverified",
        match_kind: "exact",
        matched_name: "Minh An Tran",
      },
    ],
  }),
  proposal({
    proposal_id: 2,
    ordinal: 2,
    kind: "parent_child",
    target_name: "Hoang Van Tran",
    resolution: "matched",
    support_text:
      "Relationship: Son of Hoang Van Tran and Lan Thi Nguyen.",
  }),
  proposal({
    proposal_id: 3,
    ordinal: 3,
    kind: "alias",
    resolution: "matched",
    value_text: "minhan",
    support_text: "Also known as: minhan",
  }),
  proposal({
    proposal_id: 4,
    ordinal: 4,
    kind: "social_link",
    resolution: "matched",
    value_text: "https://example.test/minhan",
    support_text: "Instagram: https://example.test/minhan",
  }),
];


function extractionBody(proposals: unknown[]) {
  return {
    ok: true,
    extraction: {
      extraction_id: 3,
      document_version_id: 7,
      document_source: "demo-card-md-1234",
      content_sha256: "a".repeat(64),
      status: "draft",
      created_by: "admin-subject",
      created_at: "2026-01-01T00:00:00Z",
      applied_by: null,
      applied_at: null,
    },
    proposals,
  };
}

test.describe("family record extraction", () => {
  test("a viewer is denied the extraction screen", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("extraction-page")).toHaveCount(0);
  });

  test("extracts, resolves an ambiguous person, and applies", async ({ page }) => {
    const decisions: Array<{ id: string; body: unknown }> = [];
    let extracted = false;
    let decisionsMade = false;
    let applied = false;

    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
      }),
    );
    await page.route("**/api/genealogy/documents/7/extraction", async (route) => {
      if (route.request().method() === "POST") {
        extracted = true;
        await route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(extractionBody(PROPOSALS)),
        });
        return;
      }
      if (!extracted) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ ok: false, error: "no_extraction", message: "none" }),
        });
        return;
      }
      const decided = decisionsMade
        ? PROPOSALS.map((item) =>
            item.proposal_id === 1
              ? { ...item, status: "approved", resolution: "matched" }
              : item,
          )
        : PROPOSALS;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(
          extractionBody(
            applied
              ? decided.map((item) => ({ ...item, status: "applied" }))
              : decided,
          ),
        ),
      });
    });
    await page.route("**/api/genealogy/document-proposals/*/decision", async (route) => {
      decisionsMade = true;
      const url = route.request().url();
      decisions.push({
        id: url.split("/document-proposals/")[1].split("/")[0],
        body: route.request().postDataJSON(),
      });
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(extractionBody(PROPOSALS)),
      });
    });
    await page.route("**/api/genealogy/documents/7/apply", async (route) => {
      applied = true;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ok: true,
          extraction_id: 3,
          already_applied: false,
          applied: [{ proposal_id: 1, kind: "person", result: {} }],
        }),
      });
    });

    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("extraction-page")).toBeVisible();
    await expect(page.getByTestId("extract-button")).toBeVisible();
    await page.getByTestId("extract-button").click();

    await expect(page.getByTestId("extraction-panel")).toBeVisible();
    await expect(page.getByTestId("proposal")).toHaveCount(4);
    await expect(
      page.getByTestId("proposal-support").filter({ hasText: "Relationship:" }),
    ).toBeVisible();
    await expect(page.getByTestId("proposal-candidate")).toHaveCount(2);
    // The alias is an editable name; the link is an editable URL.
    await expect(
      page.locator('[data-kind="alias"] [data-testid="proposal-value"]'),
    ).toHaveValue("minhan");
    await expect(
      page.locator('[data-kind="social_link"] [data-testid="proposal-value"]'),
    ).toHaveValue("https://example.test/minhan");

    // Radios carry no text, so select by accessible name instead of test id.
    await page.getByRole("radio", { name: /unverified/ }).check();
    await page
      .locator('[data-kind="person"] [data-testid="proposal-approve"]')
      .click();

    await expect(page.getByTestId("extraction-message")).toContainText(
      "approved",
    );
    expect(decisions).toHaveLength(1);
    expect(decisions[0].id).toBe("1");
    expect(decisions[0].body).toMatchObject({
      decision: "approve",
      person_key: "P101",
    });

    await page.getByTestId("apply-extraction").click();
    await expect(page.getByTestId("extraction-message")).toContainText(
      "applied",
    );
    await assertNoSensitive(page);
  });

  test("an empty extraction list explains what to do", async ({ page }) => {
    await signIn(page, "admin");
    await page.route("**/api/documents/submissions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, submissions: [] }),
      }),
    );
    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("extraction-none")).toBeVisible();
    await expect(page.getByTestId("extraction-none")).toContainText(
      "Approve and process",
    );
  });
});
