import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// PH-202: real-browser coverage for the administrator narrative review screen.
//
// Uses the synthetic fixture seeded by scripts/seed-branch-review-e2e.sql. The
// subject and namesake share a name and both have an ambiguous link, so the
// reviewer must select the correct person explicitly before approving.

const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000").origin;
const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const SUBJECT = "e2e-narrative-review-subject";
const NAMESAKE = "e2e-narrative-review-namesake";
const TEXT = "SYNTHETIC-NARRATIVE-REVIEW-PENDING";
const SOURCE = "synthetic/e2e-branch-review-source.md";

const READY = Boolean(ADMIN && REVIEWER);

type HistoryBody = {
  history: {
    language: string;
    page_start: number;
    page_end: number;
    document_ref: string;
    body: string;
  }[];
};

async function useToken(page: Page, token: string) {
  await page.context().addCookies([
    { name: "rag_session", value: token, url: ORIGIN, httpOnly: true, sameSite: "Lax" },
    { name: "rag_csrf", value: "e2e-csrf-token", url: ORIGIN, sameSite: "Lax" },
  ]);
}

async function api(
  request: APIRequestContext,
  token: string,
  path: string,
): Promise<{ status: number; body: unknown }> {
  const response = await request.get(`${API_URL}${path}`, {
    headers: { authorization: `Bearer ${token}`, accept: "application/json" },
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test("an ordinary viewer cannot list narrative review passages", async ({ request }) => {
  const denied = await api(
    request,
    REVIEWER,
    "/api/admin/narrative/passages?status=pending&limit=20",
  );
  expect([401, 403]).toContain(denied.status);
  expect(JSON.stringify(denied.body)).not.toContain(TEXT);
});

test("a pending narrative passage is hidden from the family history read", async ({ request }) => {
  const history = await api(request, REVIEWER, `/api/family/persons/${SUBJECT}/history`);
  expect(history.status).toBe(200);
  expect(JSON.stringify(history.body)).not.toContain(TEXT);
});

test("an admin confirms the correct ambiguous person and approves for family", async ({
  page,
  request,
}) => {
  await useToken(page, ADMIN);
  await page.goto("/admin/narrative-review");
  await expect(page.getByTestId("narrative-review-console")).toBeVisible();

  await page.getByTestId("narrative-person-filter").fill(SUBJECT);
  await page.getByTestId("narrative-apply-filters").click();

  const row = page
    .locator('[data-testid^="narrative-review-row-"]')
    .filter({ hasText: "Synthetic narrative review section" });
  await expect(row).toHaveCount(1);
  await row.first().click();

  await expect(page.getByTestId("narrative-review-detail")).toBeVisible();
  await expect(page.getByTestId("narrative-passage-body")).toContainText(TEXT);
  await expect(page.getByTestId("narrative-source-note")).toContainText(
    "Only citation metadata",
  );
  await expect(page.getByTestId(`narrative-link-candidate-${SUBJECT}`)).toBeVisible();
  await expect(page.getByTestId(`narrative-link-candidate-${NAMESAKE}`)).toBeVisible();
  await expect(page.getByTestId("narrative-family-blocked")).toBeVisible();
  await expect(page.getByTestId("narrative-passage-status")).toContainText("pending");

  const approveFamily = page.getByTestId("narrative-approve-family");
  await expect(approveFamily).toBeDisabled();

  await page.getByTestId(`narrative-link-radio-${SUBJECT}`).check();
  await page.getByTestId("narrative-link-note").fill(
    "Synthetic browser review: selected the correct namesake",
  );
  await page.getByTestId("narrative-link-confirm").click();

  await expect(
    page
      .getByTestId(`narrative-link-candidate-${SUBJECT}`)
      .getByTestId("narrative-link-status-confirmed"),
  ).toBeVisible();
  await expect(
    page
      .getByTestId(`narrative-link-candidate-${NAMESAKE}`)
      .getByTestId("narrative-link-status-ambiguous"),
  ).toBeVisible();
  // Confirming the link must not approve the passage.
  await expect(page.getByTestId("narrative-passage-status")).toContainText("pending");
  await expect(approveFamily).toBeEnabled();

  await page.getByTestId("narrative-passage-note").fill(
    "Synthetic browser review: approved the source-backed passage",
  );
  await page.getByTestId("narrative-approve-family").click();

  await expect(page.getByTestId("narrative-passage-status")).toContainText("approved");
  await expect(page.getByTestId("narrative-passage-visibility")).toContainText("family");
  await expect(page.getByTestId("narrative-review-events")).toContainText("link review");
  await expect(page.getByTestId("narrative-review-events")).toContainText("passage review");

  const history = await api(request, REVIEWER, `/api/family/persons/${SUBJECT}/history`);
  expect(history.status).toBe(200);
  const body = history.body as HistoryBody;
  expect(
    body.history.some(
      (passage) =>
        passage.body.includes(TEXT) &&
        passage.language === "en" &&
        passage.page_start === 20 &&
        passage.page_end === 20 &&
        passage.document_ref === SOURCE,
    ),
  ).toBe(true);

  // The subject's tree node now exposes Personal history. Use the catalog row
  // with the exact record id to focus the correct same-named person.
  await page.goto("/genealogy");
  await expect(page.getByTestId("genealogy-tree")).toBeVisible();
  const catalogItem = page.locator(
    `[data-testid="genealogy-record"][data-record-id="${SUBJECT}"]`,
  );
  await expect(catalogItem).toHaveCount(1);
  await catalogItem.getByRole("button").click();
  await page.waitForTimeout(500);
  await page.keyboard.press("Escape");

  const link = page.locator(
    '[data-testid="family-person-history"][aria-label="Personal history for Synthetic Narrative Review Subject"]',
  );
  await expect(link).toHaveCount(1);
  await link.scrollIntoViewIfNeeded();
  await link.click();

  const dialog = page.getByTestId("personal-history-dialog");
  await expect(dialog).toBeVisible();
  await expect(page.getByTestId("personal-history-passage")).toHaveCount(1);
  await expect(dialog).toContainText(TEXT);
  await expect(dialog).toContainText(SOURCE);
  await expect(dialog).toContainText("page 20");
});
