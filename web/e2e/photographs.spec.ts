import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// AP-107: browser coverage for the archive photograph surfaces.
//
// The suite stubs the API so it runs without a seeded database or a live
// backend. It still drives the real Next.js app and the real shell, and it
// asserts the browser receives only approved metadata: no storage key, no
// filesystem path, and no credential.

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

const VIEWER_SESSION = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic Viewer",
};

const ADMIN_SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic Admin",
};

const GALLERY = {
  ok: true,
  photographs: [
    {
      asset_id: 1,
      caption: "Synthetic family group, 1962",
      mime_type: "image/jpeg",
      byte_size: 1024,
      width: 640,
      height: 480,
      document_version_id: 9,
      corpus: "family",
      page_number: 4,
      citation: "PDF page 4",
      created_at: "2026-01-01T00:00:00Z",
      thumbnail_url: "/api/family/photographs/1/thumbnail",
      original_url: "/api/family/photographs/1/original",
      people: [
        { person_key: "synthetic-one", display_name: "Synthetic Person One" },
      ],
    },
    {
      asset_id: 2,
      caption: null,
      mime_type: "image/png",
      byte_size: 2048,
      width: 800,
      height: 600,
      document_version_id: 9,
      corpus: "family",
      page_number: 11,
      citation: "PDF page 11",
      created_at: "2026-01-02T00:00:00Z",
      thumbnail_url: "/api/family/photographs/2/thumbnail",
      original_url: "/api/family/photographs/2/original",
      people: [],
    },
  ],
};

const PROFILE = {
  ok: true,
  state: "published",
  person: { person_key: "synthetic-one", display_name: "Synthetic Person One" },
  fields: [],
  dates: [],
  history: [],
  media: [
    {
      kind: "profile_photo",
      media_id: 7,
      mime_type: "image/png",
      byte_size: 128,
      file_name: "synthetic-profile.png",
      width: 1,
      height: 1,
      approved_revision_number: 1,
      created_at: "2026-01-01T00:00:00Z",
    },
  ],
  archive_photographs: [
    {
      asset_id: 1,
      caption: "Synthetic family group, 1962",
      mime_type: "image/jpeg",
      byte_size: 1024,
      width: 640,
      height: 480,
      document_version_id: 9,
      corpus: "family",
      page_number: 7,
      citation: "PDF page 7",
      created_at: "2026-01-01T00:00:00Z",
      thumbnail_url: "/api/family/photographs/1/thumbnail",
      original_url: "/api/family/photographs/1/original",
    },
  ],
};

const ADMIN_PHOTO = {
  asset_id: 1,
  caption: "Synthetic family group, 1962",
  publication_state: "unpublished",
  mime_type: "image/jpeg",
  byte_size: 1024,
  width: 640,
  height: 480,
  object_id: "12 0",
  classification: "photograph",
  classification_rule: "photograph-inset-opaque",
  document_version_id: 9,
  corpus: "family",
  page_numbers: [4],
  citation: "PDF page 4",
  created_at: "2026-01-01T00:00:00Z",
  thumbnail_url: "/api/admin/photographs/1/thumbnail",
  original_url: "/api/admin/photographs/1/original",
  links: [
    {
      person_id: 3,
      person_key: "synthetic-one",
      display_name: "Synthetic Person One",
      status: "proposed",
      decided_by: null,
      decided_at: null,
      source_page: 4,
      supporting_text: "Synthetic family group, 1962",
      confidence: "high",
      confidence_basis: "an explicit photo caption names this person",
      ambiguity_note: null,
      match_kind: "exact",
      candidate_rank: 1,
      proposal_origin: "layout",
    },
  ],
};

async function stubSession(page: Page, session: typeof VIEWER_SESSION) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
    }),
  );
}

async function stubImages(page: Page) {
  await page.route("**/api/family/photographs/*/thumbnail", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
  await page.route("**/api/family/photographs/*/original", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
  await page.route("**/api/family/persons/*/media/*", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
}

function expectNoPrivatePath(text: string) {
  expect(text).not.toContain("storage_key");
  expect(text).not.toContain("data/photographs");
  expect(text).not.toContain("/Users/");
  expect(text).not.toContain("content_sha256");
}

test("the gallery lists published photographs, including an unlinked one", async ({
  page,
}) => {
  await stubSession(page, VIEWER_SESSION);
  await stubImages(page);
  await page.route("**/api/family/photographs", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(GALLERY),
    }),
  );

  await page.goto("/gallery");
  await expect(page.getByTestId("photograph-gallery")).toBeVisible();
  await expect(page.getByTestId("gallery-photo-1")).toContainText(
    "Synthetic family group, 1962",
  );
  await expect(page.getByTestId("gallery-photo-1")).toContainText(
    "PDF page 4",
  );
  await expect(page.getByTestId("gallery-photo-1")).toContainText(
    "People: Synthetic Person One",
  );
  await expect(page.getByTestId("gallery-photo-2")).toContainText(
    "No confirmed person link",
  );
  await expect(page.getByTestId("gallery-photo-2")).toContainText(
    "No recorded caption",
  );

  expectNoPrivatePath(await page.locator("body").innerText());
});

test("the profile archive section is separate from uploaded profile media", async ({
  page,
}) => {
  await stubSession(page, VIEWER_SESSION);
  await stubImages(page);
  await page.route("**/api/family/persons/*/profile", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(PROFILE),
    }),
  );

  await page.goto("/family/synthetic-one");
  await expect(page.getByTestId("family-profile")).toBeVisible();

  // The owner-uploaded photo remains in its own section.
  const uploaded = page.getByTestId("family-profile-photos");
  await expect(uploaded).toBeVisible();
  await expect(uploaded.getByTestId("family-profile-photo-7")).toBeVisible();

  // The archive photograph has its own heading, caption, and PDF citation.
  const archive = page.getByTestId("family-profile-archive-photographs");
  await expect(archive).toBeVisible();
  await expect(archive).toContainText("Archive photographs");
  await expect(archive).toContainText("Synthetic family group, 1962");
  await expect(archive).toContainText("PDF page 7");
  await expect(
    page.getByTestId("family-profile-count-archive-photos"),
  ).toHaveText("1");

  expectNoPrivatePath(await page.locator("body").innerText());
});

test("the administrator review screen exposes caption, link, and visibility controls", async ({
  page,
}) => {
  await stubSession(page, ADMIN_SESSION);
  await stubImages(page);
  await page.route("**/api/admin/photographs", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, photographs: [ADMIN_PHOTO] }),
    }),
  );
  await page.route("**/api/admin/photographs/*/thumbnail", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
  await page.route("**/api/admin/photographs/*/original", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
  await page.route("**/api/admin/photographs/*/caption", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        changed: true,
        prior: "",
        next: "Synthetic family group, 1962",
        operation: "caption_edited",
      }),
    }),
  );
  await page.route("**/api/admin/photographs/*/visibility", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        changed: true,
        prior: "unpublished",
        next: "published",
        operation: "visibility_set",
      }),
    }),
  );
  await page.route("**/api/admin/photographs/*/propose", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        candidates: [
          {
            person_id: 3,
            person_key: "synthetic-one",
            display_name: "Synthetic Person One",
            match_kind: "exact",
            confidence: "high",
            source_kind: "caption",
            source_page: 4,
            supporting_text: "Synthetic family group, 1962",
            confidence_basis: "an explicit photo caption names this person",
            ambiguity: null,
            candidate_rank: 1,
          },
        ],
        unlinked_reason: null,
        preserved_decisions: 0,
      }),
    }),
  );
  await page.route("**/api/admin/photographs/*/links/*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        changed: true,
        prior: "proposed",
        next: "confirmed",
        operation: "link_confirmed",
      }),
    }),
  );

  await page.goto("/admin/photographs");
  await expect(page.getByTestId("photograph-review-console")).toBeVisible();
  await expect(page.getByTestId("photograph-detail")).toBeVisible();
  await expect(page.getByTestId("photograph-caption-input")).toHaveValue(
    "Synthetic family group, 1962",
  );
  await expect(page.getByTestId("photograph-visibility-select")).toHaveValue(
    "unpublished",
  );
  await expect(page.getByTestId("photograph-link-3")).toContainText(
    "Synthetic Person One",
  );
  await expect(
    page.getByTestId("photograph-link-3-confirm"),
  ).toBeVisible();
  await expect(page.getByTestId("photograph-propose")).toBeEnabled();

  // A caption save is recorded through the administrator API.
  await page.getByTestId("photograph-caption-save").click();
  await expect(page.getByTestId("photograph-review-message")).toContainText(
    "Caption saved",
  );

  // Publishing is an explicit reviewer action.
  await page.getByTestId("photograph-visibility-select").selectOption(
    "published",
  );
  await page.getByTestId("photograph-visibility-apply").click();
  await expect(page.getByTestId("photograph-review-message")).toContainText(
    "published",
  );

  expectNoPrivatePath(await page.locator("body").innerText());
});

test("a non-administrator sees the denial state, not the review controls", async ({
  page,
}) => {
  await stubSession(page, VIEWER_SESSION);
  await page.goto("/admin/photographs");
  await expect(
    page.getByText("Archive photograph review is restricted to administrators."),
  ).toBeVisible();
  await expect(page.getByTestId("photograph-review-console")).toHaveCount(0);
});
