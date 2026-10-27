import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Stage-four mocked visual suite for the owner profile.
//
// No database, API, Ollama, or credentials are needed. Only the browser session,
// profile, and media requests are intercepted; every name, value, decision, and
// file is synthetic.

const SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const PRIVATE_MARKER = "SYNTHETIC-PRIVATE-MARKER";

const value = (field_key: string, text: string, visibility = "family") => ({
  field_key,
  value: text,
  visibility,
  approved_revision_number: 2,
  approved_revision_id: 20,
  updated_at: "2026-10-20T12:00:00Z",
  restricted_from_family: false,
  restriction_generation: null,
});

const revision = (overrides: Record<string, unknown>) => ({
  revision_id: 7,
  field_key: "biography",
  revision_number: 3,
  prior_value_text: null,
  prior_approved_revision_number: 2,
  value: "",
  visibility: "family",
  status: "draft",
  changed_at: "2026-10-21T12:00:00Z",
  submitted_at: null,
  reviewed_at: null,
  review_note: null,
  ...overrides,
});

const PROFILE = {
  ok: true,
  linked: true,
  profile: {
    display_name: "Nguyễn Văn An",
    approved: [
      value("display_name", "Nguyễn Văn An"),
      value("preferred_name", "Ông An"),
      value(
        "biography",
        "Nguyễn Văn An grew up near the river and kept the family register for three decades.",
      ),
      value("current_location", PRIVATE_MARKER, "owner_private"),
      value(
        "my_story",
        "The family kept the first house beside the river. Every summer, the household gathered for the ancestral remembrance.",
      ),
      value(
        "family_memories",
        "He remembered the lanterns at the river festival and the names of every cousin who visited.",
      ),
    ],
    pending: [
      revision({
        revision_id: 9,
        field_key: "biography",
        revision_number: 4,
        value: "An updated synthetic biography waiting for independent review.",
        status: "pending_review",
        submitted_at: "2026-10-22T12:00:00Z",
      }),
    ],
    history: [
      revision({
        revision_id: 6,
        field_key: "preferred_name",
        revision_number: 2,
        value: "Ông An",
        status: "needs_changes",
        reviewed_at: "2026-10-20T12:00:00Z",
        review_note: "Synthetic reviewer: add the spelling used in the source.",
      }),
      revision({
        revision_id: 5,
        field_key: "biography",
        revision_number: 2,
        value: "Approved synthetic biography.",
        status: "approved",
        submitted_at: "2026-10-19T12:00:00Z",
        reviewed_at: "2026-10-20T12:00:00Z",
      }),
    ],
  },
  media: [
    {
      media_id: 1,
      revision_id: 21,
      kind: "profile_photo",
      mime_type: "image/png",
      byte_size: 4096,
      file_name: "synthetic-profile.png",
      visibility: "family",
      width: 24,
      height: 24,
      status: "approved",
      created_at: "2026-10-20T12:00:00Z",
      restricted_from_family: false,
    },
    {
      media_id: 2,
      revision_id: 22,
      kind: "personal_document",
      mime_type: "application/pdf",
      byte_size: 182000,
      file_name: "synthetic-family-letter.pdf",
      visibility: "family",
      width: null,
      height: null,
      status: "approved",
      created_at: "2026-10-20T12:00:00Z",
      restricted_from_family: false,
    },
  ],
};

const NO_PHOTO = {
  ...PROFILE,
  media: PROFILE.media.filter((asset) => asset.kind !== "profile_photo"),
};

const UNLINKED = { ok: true, linked: false };

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAH0lEQVR42mPQCLChCmIYNWjUoFGDRg0aNWjUoIE3CAAHDpUQ1ztOKgAAAABJRU5ErkJggg==",
  "base64",
);

async function stub(
  page: Page,
  profile: unknown,
  {
    profileStatus = 200,
    session = SESSION,
    delayMs = 0,
  }: { profileStatus?: number; session?: unknown; delayMs?: number } = {},
) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
    }),
  );
  await page.route("**/api/profile/media/**", (route) =>
    route.fulfill({ status: 200, contentType: "image/png", body: PNG }),
  );
  await page.route("**/api/profile", async (route) => {
    if (delayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    await route.fulfill({
      status: profileStatus,
      contentType: "application/json",
      body: JSON.stringify(profile),
    });
  });
}

async function assertNoRawLeak(page: Page) {
  const body = await page.content();
  for (const forbidden of [
    /postgres(ql)?:\/\//i,
    /storage_key/i,
    /sha256/i,
    /Bearer\s+/i,
    /\/data\/media/i,
  ]) {
    expect(body, `DOM leaked ${forbidden}`).not.toMatch(forbidden);
  }
}

test.describe("owner profile visual layout", () => {
  test("renders approved values, revision state, media, and history", async ({
    page,
  }) => {
    await stub(page, PROFILE);
    await page.goto("/profile");

    await expect(page.getByTestId("profile-header")).toBeVisible();
    await expect(page.getByTestId("profile-name")).toHaveText("Nguyễn Văn An");
    await expect(page.getByTestId("profile-completion")).toContainText(
      "8 of 13",
    );
    const completion = page.locator(
      "progress[aria-label='Profile completion']",
    );
    await expect(completion).toHaveJSProperty("value", 8);
    await expect(completion).toHaveJSProperty("max", 13);
    await expect(page.getByTestId("profile-pending-count")).toContainText(
      "1 open submission",
    );
    await expect(page.getByTestId("profile-feedback-count")).toContainText(
      "1 review decision",
    );
    await expect(page.getByTestId("profile-section-about")).toContainText(
      "kept the family register",
    );
    await expect(page.getByTestId("profile-open-biography")).toContainText(
      "An updated synthetic biography",
    );
    await expect(page.getByTestId("profile-open-biography")).toContainText(
      "Pending review",
    );
    await expect(
      page.getByTestId("profile-feedback-preferred_name"),
    ).toContainText("add the spelling used in the source");
    await expect(page.getByTestId("profile-photo-current")).toBeVisible();
    await expect(page.getByTestId("profile-document-current")).toContainText(
      "synthetic-family-letter.pdf",
    );
    await expect(page.getByTestId("profile-history")).toContainText(
      "Preferred name or nickname",
    );
    await assertNoRawLeak(page);
  });

  test("uses initials when no photograph exists", async ({ page }) => {
    await stub(page, NO_PHOTO);
    await page.goto("/profile");

    await expect(page.getByTestId("profile-photo-initials")).toBeVisible();
    await expect(page.getByTestId("profile-photo-initials")).toHaveText("N");
    await expect(page.getByTestId("profile-photo-preview")).toHaveCount(0);
    await expect(page.getByTestId("profile-document-current")).toContainText(
      "synthetic-family-letter.pdf",
    );
  });

  test("keeps loading, unavailable, expired, sign-in, and unlinked states distinct", async ({
    page,
  }) => {
    await test.step("loading", async () => {
      await stub(page, PROFILE, { delayMs: 1500 });
      await page.goto("/profile", { waitUntil: "domcontentloaded" });
      await expect(page.getByTestId("profile-loading")).toBeVisible();
      await expect(page.getByTestId("profile-loading")).toHaveAttribute(
        "aria-busy",
        "true",
      );
      await page.unrouteAll();
    });

    await test.step("unavailable then retry", async () => {
      await stub(
        page,
        { ok: false, error: "unavailable" },
        {
          profileStatus: 503,
        },
      );
      await page.goto("/profile");
      await expect(page.getByTestId("profile-error")).toBeVisible();
      await expect(page.getByTestId("profile-retry")).toBeVisible();
      await page.unroute("**/api/profile");
      await page.route("**/api/profile", (route) =>
        route.fulfill({
          status: 200,
          contentType: "application/json",
          body: JSON.stringify(PROFILE),
        }),
      );
      await page.getByTestId("profile-retry").click();
      await expect(page.getByTestId("profile-header")).toBeVisible();
      await page.unrouteAll();
    });

    await test.step("expired", async () => {
      await stub(page, { ok: false, error: "expired" }, { profileStatus: 401 });
      await page.goto("/profile");
      await expect(page.getByTestId("profile-expired")).toBeVisible();
      await expect(page.getByTestId("profile-signin-button")).toBeVisible();
      await page.unrouteAll();
    });

    await test.step("sign-in", async () => {
      await stub(
        page,
        { ok: true, linked: false },
        {
          session: { state: "anonymous" },
        },
      );
      await page.goto("/profile");
      await expect(page.getByTestId("profile-signin")).toBeVisible();
      await expect(page.getByTestId("profile-header")).toHaveCount(0);
      await page.unrouteAll();
    });

    await test.step("unlinked", async () => {
      await stub(page, UNLINKED);
      await page.goto("/profile");
      await expect(page.getByTestId("profile-unlinked")).toBeVisible();
      await expect(page.getByTestId("profile-header")).toHaveCount(0);
    });
  });

  test("remains usable at 390px and with the editor open", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stub(page, PROFILE);
    await page.goto("/profile");

    await expect(page.getByTestId("nav-toggle")).toBeVisible();
    await expect(page.getByTestId("profile-header")).toBeVisible();
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow, "no horizontal overflow").toBeLessThanOrEqual(1);

    await page.getByTestId("profile-add-whatsapp").click();
    await expect(page.getByTestId("profile-input-whatsapp")).toBeVisible();
    const buttonBox = await page
      .getByTestId("profile-save-whatsapp")
      .boundingBox();
    expect(buttonBox?.height ?? 0).toBeGreaterThanOrEqual(44);
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
    await expect(page.getByTestId("profile-input-whatsapp")).toHaveCount(0);
  });

  test("applies the stage-three light and dark tokens", async ({ page }) => {
    await stub(page, PROFILE);
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto("/profile");
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
});
