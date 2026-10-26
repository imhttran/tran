import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// UI-203: mocked visual suite for the family profile summary and section layout.
//
// This suite needs no database, API, Ollama, or credentials. It intercepts only
// the browser-facing session, profile, and media requests, and every value is
// synthetic.

const SESSION = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};
const PERSON_KEY = "e2e-profile-visual";
const DISPLAY_NAME = "Nguyễn Thị Mai";
const PRIVATE_MARKER = "PRIVATE-VALUE-MUST-NOT-RENDER";

const FIELDS = [
  {
    field_key: "display_name",
    value: DISPLAY_NAME,
    visibility: "family",
    approved_revision_number: 2,
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    field_key: "biography",
    value: "Synthetic approved biography for the visual suite.",
    visibility: "family",
    approved_revision_number: 2,
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    field_key: "my_story",
    value: "A synthetic personal story.",
    visibility: "family",
    approved_revision_number: 1,
    updated_at: "2026-01-01T00:00:00Z",
  },
  {
    field_key: "family_memories",
    value: "A synthetic family memory.",
    visibility: "family",
    approved_revision_number: 1,
    updated_at: "2026-01-01T00:00:00Z",
  },
  // Deliberately outside the page's field allow-list: it must never render.
  {
    field_key: "contact_preferences",
    value: PRIVATE_MARKER,
    visibility: "family",
    approved_revision_number: 1,
    updated_at: "2026-01-01T00:00:00Z",
  },
];

const DATES = [
  {
    kind: "birth_date",
    value: "1940-03-12",
    start: "1940-03-12",
    end: null,
    precision: "day",
    qualifier: "exact",
  },
];

const HISTORY = [
  {
    passage_id: 7,
    language: "vi",
    document_ref: "synthetic/archive.pdf",
    section_title: "Gia phả",
    page_start: 2,
    page_end: 2,
    body: "Synthetic archive passage.",
  },
];

const MEDIA = [
  {
    kind: "profile_photo",
    media_id: 11,
    mime_type: "image/png",
    byte_size: 68,
    file_name: "portrait.png",
    width: 64,
    height: 64,
    approved_revision_number: 1,
    created_at: "2026-01-01T00:00:00Z",
  },
  {
    kind: "personal_document",
    media_id: 12,
    mime_type: "application/pdf",
    byte_size: 100,
    file_name: "synthetic-document.pdf",
    width: null,
    height: null,
    approved_revision_number: 1,
    created_at: "2026-01-01T00:00:00Z",
  },
];

const PROFILE = {
  ok: true,
  state: "published",
  person: { person_key: PERSON_KEY, display_name: DISPLAY_NAME },
  fields: FIELDS,
  media: MEDIA,
  dates: DATES,
  history: HISTORY,
};

const EMPTY = {
  ok: true,
  state: "empty",
  person: { person_key: PERSON_KEY, display_name: DISPLAY_NAME },
  fields: [],
  media: [],
  dates: [],
  history: [],
};

const NO_MEDIA = { ...PROFILE, media: [] };

// 1x1 transparent PNG, enough for the summary <img> to load.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl1sAAAAASUVORK5CYII=",
  "base64",
);

async function stub(page: Page, profile: unknown, status = 200) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(SESSION),
    }),
  );
  await page.route("**/api/family/persons/**/profile", (route) =>
    route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify(profile),
    }),
  );
  await page.route("**/api/family/persons/**/media/**", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
}

test.describe("family profile visual layout", () => {
  test("renders the approved summary and ordered sections", async ({ page }) => {
    await stub(page, PROFILE);
    await page.goto(`/family/${PERSON_KEY}`);

    await expect(page.getByTestId("family-profile")).toBeVisible();
    await expect(page.getByTestId("family-profile-summary")).toBeVisible();
    await expect(page.getByTestId("family-profile-summary-photo")).toBeVisible();
    await expect(page.getByTestId("family-profile-summary")).toContainText(DISPLAY_NAME);
    await expect(page.getByTestId("family-profile-summary-dates")).toContainText(
      "12 Mar 1940",
    );
    await expect(page.getByTestId("family-profile-count-fields")).toHaveText("4");
    await expect(page.getByTestId("family-profile-count-dates")).toHaveText("1");
    await expect(page.getByTestId("family-profile-count-history")).toHaveText("1");
    await expect(page.getByTestId("family-profile-count-photos")).toHaveText("1");
    await expect(page.getByTestId("family-profile-count-documents")).toHaveText("1");
    await expect(page.getByTestId("family-profile-summary-tree-link")).toHaveAttribute(
      "href",
      "/genealogy",
    );

    const order = await page
      .locator(
        '[data-testid^="family-profile-section-"], [data-testid="family-profile-dates"], [data-testid="family-profile-history"], [data-testid="family-profile-photos"], [data-testid="family-profile-documents"]',
      )
      .evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-testid")));
    expect(order).toEqual([
      "family-profile-section-about",
      "family-profile-section-story",
      "family-profile-section-memories",
      "family-profile-dates",
      "family-profile-history",
      "family-profile-photos",
      "family-profile-documents",
    ]);

    await expect(page.getByTestId("family-profile-section-about")).toContainText(
      "Synthetic approved biography",
    );
    await expect(page.getByTestId("family-profile-history")).toContainText(
      "Synthetic archive passage.",
    );
    await expect(page.getByTestId("family-profile-photos")).toContainText(
      "Approved version 1",
    );
    await expect(page.getByTestId("family-profile-documents")).toContainText(
      "synthetic-document.pdf",
    );
    // A stored value outside the page's allow-list never reaches the screen.
    await expect(page.getByText(PRIVATE_MARKER)).toHaveCount(0);
  });

  test("uses initials and renders no img when there is no media", async ({ page }) => {
    await stub(page, NO_MEDIA);
    await page.goto(`/family/${PERSON_KEY}`);

    await expect(page.getByTestId("family-profile-summary-initial")).toBeVisible();
    await expect(page.getByTestId("family-profile-summary-initial")).toHaveText("N");
    await expect(page.getByTestId("family-profile-summary-photo")).toHaveCount(0);
    await expect(page.locator('[data-testid="family-profile"] img')).toHaveCount(0);
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stub(page, PROFILE);
    await page.goto(`/family/${PERSON_KEY}`);
    await expect(page.getByTestId("family-profile")).toBeVisible();

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth - window.innerWidth,
    );
    expect(overflow).toBeLessThanOrEqual(1);
  });

  test("keeps the empty and unavailable states distinct", async ({ page }) => {
    await stub(page, EMPTY);
    await page.goto(`/family/${PERSON_KEY}`);
    await expect(page.getByTestId("family-profile-empty")).toBeVisible();
    await expect(page.getByTestId("family-profile-unavailable")).toHaveCount(0);

    await page.unroute("**/api/family/persons/**/profile");
    await page.route("**/api/family/persons/**/profile", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ ok: false }),
      }),
    );
    await page.reload();
    await expect(page.getByTestId("family-profile-unavailable")).toBeVisible();
    await expect(page.getByTestId("family-profile-retry")).toBeVisible();
    await expect(page.getByTestId("family-profile-empty")).toHaveCount(0);
  });
});
