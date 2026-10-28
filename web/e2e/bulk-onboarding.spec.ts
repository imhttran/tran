import { expect, test } from "@playwright/test";
import type { Page, Route } from "@playwright/test";

// Real-browser BULK-005 acceptance against the synthetic genealogy fixture
// (scripts/seed-bulk-onboarding-e2e.sql). The harness seeds six synthetic people:
// two eligible and four blocked. The test proves a zero-write preview, idempotent
// preparation, a separate verified-recipient and explicit send gate, and that a
// non-admin is denied.
//
// BOT-105 adds stubbed coverage for the preview columns and filters: the Person
// and Contact columns, the classification stat cards, the "No contact" filter,
// accent-insensitive search, the Warnings column hiding the contact codes, and
// the labelled-card mobile layout. Every stub row uses a synthetic key and an
// example.test address. The preview stub pages by request offset, so the load
// loop asks for at least two pages.

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(
  /\/+$/,
  "",
);
const WEB_URL = (process.env.WEB_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const ADMIN_TOKEN = process.env.E2E_ADMIN_TOKEN ?? "";
const VIEWER_TOKEN = process.env.E2E_VIEWER_TOKEN ?? "";
const CSRF = process.env.E2E_CSRF_TOKEN ?? "e2e-csrf-token";

const ELIGIBLE = ["synthetic-bulk-eligible-1", "synthetic-bulk-eligible-2"];

function adminHeaders() {
  return {
    authorization: `Bearer ${ADMIN_TOKEN}`,
    "content-type": "application/json",
    "x-csrf-token": CSRF,
    cookie: `rag_csrf=${CSRF}`,
  };
}

test.describe.serial("bulk onboarding", () => {
  test.skip(!ADMIN_TOKEN, "E2E_ADMIN_TOKEN not provided by the harness");

  test("admin previews, prepares, verifies recipients, and sends", async ({
    page,
  }) => {
    await page.goto("/signin");
    await page.getByTestId("signin-button").click();
    await page.waitForURL(/127\.0\.0\.1:3000\/$/, { timeout: 30_000 });

    await page.goto("/admin/family-members");
    await expect(page.getByTestId("bulk-preview")).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByTestId("bulk-table")).toContainText(
      "synthetic-bulk-eligible-1",
    );
    // The default view is eligible + needs-review. Toggle the remaining
    // classifications on so the existing fixture assertions can see every row.
    for (const key of [
      "not_eligible",
      "already_linked",
      "already_prepared",
      "invited",
    ]) {
      await page.getByTestId(`bulk-filter-${key}`).click();
    }
    await expect(page.getByTestId("bulk-table")).toContainText(
      "synthetic-bulk-deceased",
    );
    await expect(page.getByTestId("bulk-table")).toContainText(
      "synthetic-bulk-disputed",
    );
    await expect(page.getByTestId("bulk-table")).toContainText(
      "synthetic-bulk-linked",
    );
    await expect(page.getByTestId("bulk-table")).toContainText(
      "synthetic-bulk-invited",
    );

    // Exactly the two eligible synthetic rows are selectable.
    for (const key of ELIGIBLE) {
      await expect(page.getByLabel(`Select ${key}`)).toBeEnabled();
      await page.getByLabel(`Select ${key}`).check();
    }
    await expect(
      page.getByLabel("Select synthetic-bulk-deceased"),
    ).toBeDisabled();
    await expect(
      page.getByLabel("Select synthetic-bulk-disputed"),
    ).toBeDisabled();

    // Preparation is a second, explicit confirmation. It creates no email.
    await page.getByTestId("bulk-prepare").click();
    await expect(page.getByTestId("bulk-prepare-confirm")).toBeVisible();
    await page.getByTestId("bulk-prepare-confirm-button").click();
    await expect(page.getByTestId("bulk-results")).toBeVisible({
      timeout: 30_000,
    });
    await expect(page.getByTestId("bulk-results")).toContainText("Prepared 2");

    // A repeat submission is idempotent: no new batch and no new preparation.
    const repeat = await page.request.post(
      `${WEB_URL}/api/admin/family-members/bulk/prepare`,
      {
        headers: adminHeaders(),
        data: JSON.stringify({
          person_keys: ELIGIBLE,
          confirmed_count: ELIGIBLE.length,
        }),
      },
    );
    expect(repeat.status()).toBe(200);
    const repeated = (await repeat.json()) as {
      created: boolean;
      prepared: number;
      batch_id: number;
    };
    expect(repeated.created).toBe(false);
    expect(repeated.prepared).toBe(2);
    const batchId = repeated.batch_id;

    // Sending before a verified recipient is blocked without claiming delivery.
    const blocked = await page.request.post(
      `${WEB_URL}/api/admin/family-members/bulk/send`,
      {
        headers: adminHeaders(),
        data: JSON.stringify({
          batch_id: batchId,
          person_keys: ELIGIBLE,
          approve: true,
        }),
      },
    );
    expect(blocked.status()).toBe(200);
    const blockedBody = (await blocked.json()) as {
      results: Array<{ status: string; reason: string | null }>;
    };
    expect(
      blockedBody.results.every((row) => row.status === "failed"),
    ).toBeTruthy();
    expect(
      blockedBody.results.every(
        (row) => row.reason === "missing_verified_contact",
      ),
    ).toBeTruthy();
    const blockedText = JSON.stringify(blockedBody);
    expect(blockedText).not.toContain("token");
    expect(blockedText).not.toContain("@");

    // Record an explicitly verified recipient for each prepared person.
    for (const key of ELIGIBLE) {
      const row = page.getByRole("row", { name: key });
      await row
        .getByLabel(`Verified email for ${key}`)
        .fill(`bulk-${key}@example.test`);
      await row.getByRole("button", { name: "Verify" }).click();
      await expect(row.getByText("Verified")).toBeVisible({ timeout: 30_000 });
    }

    // The send gate is separate from preparation and requires explicit approval.
    await page.getByTestId("bulk-send").click();
    await expect(page.getByTestId("bulk-send-confirm")).toBeVisible();
    await page.getByTestId("bulk-send-confirm-button").click();
    await expect(page.getByTestId("bulk-send-result")).toContainText("2 sent", {
      timeout: 30_000,
    });
    await expect(page.getByTestId("bulk-status-table")).toContainText("sent");

    // The UI never exposes a token value.
    const body = await page.content();
    expect(body).not.toContain("token=");
  });

  test("viewer is denied the page and the API", async ({ page, request }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    await page.route("**/api/auth/session", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          state: "signed_in",
          role: "viewer",
          display_name: "Signed-in viewer",
        }),
      }),
    );
    await page.goto("/admin/family-members");
    await expect(page.getByTestId("denied-view")).toBeVisible();

    const denied = await request.post(
      `${API_URL}/api/admin/family-members/bulk/prepare`,
      {
        headers: {
          authorization: `Bearer ${VIEWER_TOKEN}`,
          "content-type": "application/json",
        },
        data: JSON.stringify({ person_keys: ELIGIBLE, confirmed_count: 2 }),
      },
    );
    expect(denied.status()).toBe(403);
  });

  test("bulk mutation without CSRF is rejected by the proxy", async ({
    request,
  }) => {
    const response = await request.post(
      `${WEB_URL}/api/admin/family-members/bulk/prepare`,
      {
        headers: {
          authorization: `Bearer ${ADMIN_TOKEN}`,
          "content-type": "application/json",
        },
        data: JSON.stringify({ person_keys: ELIGIBLE, confirmed_count: 2 }),
      },
    );
    expect(response.status()).toBe(403);
    const body = (await response.json()) as { error: string };
    expect(body.error).toBe("csrf_failed");
  });
});

// --- stubbed preview coverage (BOT-105) ------------------------------------
//
// Every value below is synthetic. Keys carry the synthetic-bulk- prefix and
// every address ends in example.test. The preview stub honors the request body
// limit and offset, so the page's load loop asks for a second page.

type StubContact = { type: "email"; masked: string } | { type: "none" };

type StubClassification =
  | "eligible_for_invite"
  | "needs_review"
  | "already_linked"
  | "already_prepared"
  | "invited"
  | "not_eligible";

type StubRow = {
  person_key: string;
  display_name: string;
  contact: StubContact;
  classification: StubClassification;
  reasons: string[];
  proposed_username: string | null;
  preparable: boolean;
  blocked_missing_recipient: boolean;
  sendable: boolean;
};

const STUB_MASKED_EMAIL = "s•••@example.test";

// Accent-insensitive fixtures. The display name carries marks; the query does
// not. Both names are synthetic.
const ACCENTED_DISPLAY_NAME = "Synthetic Trần Bảo Lộc";
const ACCENTED_QUERY = "synthetic tran bao loc";
const ACCENTED_KEY = "synthetic-bulk-accent-person";

function stubRow(
  index: number,
  classification: StubClassification,
  contact: StubContact,
  extra: Partial<StubRow> = {},
): StubRow {
  return {
    person_key: `synthetic-bulk-${classification}-${index}`,
    display_name: `Synthetic ${classification} ${index}`,
    contact,
    classification,
    reasons: [],
    proposed_username: `synthetic_bulk_${classification}_${index}`,
    preparable: classification === "eligible_for_invite",
    blocked_missing_recipient: false,
    sendable: false,
    ...extra,
  };
}

const EMAIL_CONTACT: StubContact = { type: "email", masked: STUB_MASKED_EMAIL };
const NONE_CONTACT: StubContact = { type: "none" };

// Rows that every stubbed test relies on, defined first so they start on the
// first page. Later rows fill the first page and push two known keys to the
// second page.
const FIXED_ROWS: StubRow[] = [
  stubRow(1, "eligible_for_invite", EMAIL_CONTACT),
  stubRow(2, "eligible_for_invite", NONE_CONTACT),
  stubRow(1, "needs_review", EMAIL_CONTACT, {
    reasons: ["missing_verified_contact", "conflicting_names"],
  }),
  stubRow(1, "already_linked", EMAIL_CONTACT),
  stubRow(1, "already_prepared", NONE_CONTACT),
  stubRow(1, "invited", EMAIL_CONTACT),
  stubRow(1, "not_eligible", NONE_CONTACT),
  {
    person_key: ACCENTED_KEY,
    display_name: ACCENTED_DISPLAY_NAME,
    contact: EMAIL_CONTACT,
    classification: "eligible_for_invite",
    reasons: [],
    proposed_username: "synthetic_bulk_accent",
    preparable: true,
    blocked_missing_recipient: false,
    sendable: false,
  },
];

const FILLER_COUNT = 210;
const STUB_ROW_COUNT = FIXED_ROWS.length + FILLER_COUNT + 2;
const SECOND_PAGE_KEY = "synthetic-bulk-second-page-person";
const SECOND_PAGE_WARNING_KEY = "synthetic-bulk-second-page-warning";

function buildStubRows(
  selector: "all" | "all_eligible" | "explicit",
): StubRow[] {
  const rows: StubRow[] = [...FIXED_ROWS];
  for (let index = 0; index < FILLER_COUNT; index += 1) {
    // The filler keeps the default view populated and the second page reachable.
    const classification: StubClassification =
      index % 4 === 0 ? "needs_review" : "eligible_for_invite";
    const contact: StubContact = index % 3 === 0 ? NONE_CONTACT : EMAIL_CONTACT;
    rows.push(stubRow(index + 1000, classification, contact));
  }
  rows.push({
    person_key: SECOND_PAGE_KEY,
    display_name: "Synthetic second page person",
    contact: EMAIL_CONTACT,
    classification: "eligible_for_invite",
    reasons: [],
    proposed_username: "synthetic_bulk_second_page",
    preparable: true,
    blocked_missing_recipient: false,
    sendable: false,
  });
  rows.push({
    person_key: SECOND_PAGE_WARNING_KEY,
    display_name: "Synthetic second page warning",
    contact: NONE_CONTACT,
    classification: "needs_review",
    reasons: [
      "missing_verified_contact",
      "blocked_missing_recipient",
      "needs_name_review",
    ],
    proposed_username: null,
    preparable: false,
    blocked_missing_recipient: true,
    sendable: false,
  });
  if (selector === "all_eligible") {
    return rows.filter((row) => row.preparable);
  }
  return rows;
}

function stubSummary(rows: StubRow[]) {
  const counts: Record<string, number> = {
    eligible_for_invite: 0,
    needs_review: 0,
    already_linked: 0,
    already_prepared: 0,
    invited: 0,
    not_eligible: 0,
    blocked_missing_recipient: 0,
  };
  for (const row of rows) {
    counts[row.classification] += 1;
    if (row.blocked_missing_recipient) counts.blocked_missing_recipient += 1;
  }
  return counts;
}

async function stubPreview(page: Page) {
  const allRows = buildStubRows("all");
  const summary = stubSummary(allRows);
  await page.route(
    "**/api/admin/family-members/bulk/preview",
    (route: Route) => {
      const body = (route.request().postDataJSON() ?? {}) as {
        selector?: "all" | "all_eligible" | "explicit";
        limit?: number;
        offset?: number;
      };
      const selector = body.selector ?? "all";
      const limit = body.limit && body.limit > 0 ? body.limit : allRows.length;
      const offset = body.offset && body.offset > 0 ? body.offset : 0;
      const rows = buildStubRows(selector).slice(offset, offset + limit);
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ok: true,
          rows,
          summary,
          review_needed: null,
        }),
      });
    },
  );
  return { total: allRows.length, summary };
}

async function stubAdmin(page: Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        state: "signed_in",
        role: "admin",
        display_name: "Signed-in admin",
      }),
    }),
  );
  // The page also renders the account directory. Stub it so its real request
  // cannot return 401 and trip the global session-expiry state while these
  // tests are exercising the preview table.
  await page.route("**/api/admin/family-members", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        members: [],
        next_after: null,
        linkage: "available",
      }),
    }),
  );
}

async function openStubbedPreview(page: Page) {
  await stubAdmin(page);
  await stubPreview(page);
  await page.goto("/admin/family-members");
  await expect(page.getByTestId("bulk-preview")).toBeVisible({
    timeout: 30_000,
  });
  await expect(page.getByTestId("bulk-showing")).toBeVisible();
  // Wait for the paging loop to finish with the full row count.
  await expect(page.getByTestId("bulk-showing")).toContainText(
    `of ${STUB_ROW_COUNT} people`,
    { timeout: 30_000 },
  );
}

function previewRow(page: Page, personKey: string) {
  return page
    .getByTestId("bulk-table")
    .locator(`tr[data-person-key="${personKey}"]`);
}

const DEFAULT_VISIBLE_KEYS = [
  "synthetic-bulk-eligible_for_invite-1",
  "synthetic-bulk-eligible_for_invite-2",
  "synthetic-bulk-needs_review-1",
  ACCENTED_KEY,
];

const NON_DEFAULT_KEYS = [
  "synthetic-bulk-already_linked-1",
  "synthetic-bulk-already_prepared-1",
  "synthetic-bulk-invited-1",
  "synthetic-bulk-not_eligible-1",
];

test.describe("stubbed preview filters", () => {
  test("the default filter shows only eligible and needs-review rows", async ({
    page,
  }) => {
    await openStubbedPreview(page);

    const eligibleCount = (await buildStubRows("all")).filter(
      (row) => row.classification === "eligible_for_invite",
    ).length;
    const needsReviewCount = (await buildStubRows("all")).filter(
      (row) => row.classification === "needs_review",
    ).length;
    await expect(page.getByTestId("bulk-showing")).toHaveText(
      `Showing ${eligibleCount + needsReviewCount} of ${STUB_ROW_COUNT} people`,
    );

    for (const key of DEFAULT_VISIBLE_KEYS) {
      await expect(previewRow(page, key)).toBeVisible();
    }
    for (const key of NON_DEFAULT_KEYS) {
      await expect(previewRow(page, key)).toHaveCount(0);
    }

    await expect(
      page.getByTestId("bulk-filter-eligible_for_invite"),
    ).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("bulk-filter-needs_review")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(
      page.getByTestId("bulk-filter-already_linked"),
    ).toHaveAttribute("aria-pressed", "false");
  });

  test("clicking a stat card changes the visible rows", async ({ page }) => {
    await openStubbedPreview(page);
    const eligibleCard = page.getByTestId("bulk-filter-eligible_for_invite");
    const linkedCard = page.getByTestId("bulk-filter-already_linked");
    await expect(eligibleCard).toHaveAttribute("aria-pressed", "true");

    // Turn off eligible, turn on already linked.
    await eligibleCard.click();
    await expect(eligibleCard).toHaveAttribute("aria-pressed", "false");
    await linkedCard.click();
    await expect(linkedCard).toHaveAttribute("aria-pressed", "true");

    await expect(
      previewRow(page, "synthetic-bulk-already_linked-1"),
    ).toBeVisible();
    await expect(
      previewRow(page, "synthetic-bulk-needs_review-1"),
    ).toBeVisible();
    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-1"),
    ).toHaveCount(0);

    // Reset to the default view: both default cards pressed.
    await eligibleCard.click();
    await linkedCard.click();
    await expect(eligibleCard).toHaveAttribute("aria-pressed", "true");
    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-1"),
    ).toBeVisible();
    await expect(
      previewRow(page, "synthetic-bulk-already_linked-1"),
    ).toHaveCount(0);
  });

  test("the No contact filter shows only rows with no contact", async ({
    page,
  }) => {
    await openStubbedPreview(page);
    const noContact = page.getByTestId("bulk-filter-no-contact");
    await expect(noContact).toHaveAttribute("aria-pressed", "false");
    await noContact.click();
    await expect(noContact).toHaveAttribute("aria-pressed", "true");

    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-2"),
    ).toBeVisible();
    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-1"),
    ).toHaveCount(0);
    await expect(previewRow(page, "synthetic-bulk-needs_review-1")).toHaveCount(
      0,
    );

    // The eligible+none row carries no email: its Contact cell says None.
    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-2").locator(
        'td[data-label="Contact"]',
      ),
    ).toHaveText("None");
  });

  test("search matches a display name typed without accents", async ({
    page,
  }) => {
    await openStubbedPreview(page);
    await expect(previewRow(page, ACCENTED_KEY)).toBeVisible();

    const search = page.getByTestId("bulk-filter-search");
    await search.fill(ACCENTED_QUERY);
    await expect(page.getByTestId("bulk-showing")).toContainText(
      "Showing 1 of",
    );
    await expect(previewRow(page, ACCENTED_KEY)).toBeVisible();
    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-1"),
    ).toHaveCount(0);

    await search.fill("");
    await expect(
      previewRow(page, "synthetic-bulk-eligible_for_invite-1"),
    ).toBeVisible();
  });

  test("the Contact column shows the masked email or None", async ({
    page,
  }) => {
    await openStubbedPreview(page);

    const emailCell = previewRow(
      page,
      "synthetic-bulk-eligible_for_invite-1",
    ).locator('td[data-label="Contact"]');
    await expect(emailCell).toContainText("Email");
    await expect(emailCell).toContainText(STUB_MASKED_EMAIL);
    await expect(emailCell).not.toContainText("synthetic");

    const noneCell = previewRow(
      page,
      "synthetic-bulk-eligible_for_invite-2",
    ).locator('td[data-label="Contact"]');
    await expect(noneCell).toHaveText("None");
  });

  test("the Warnings column hides contact codes and pages load later rows", async ({
    page,
  }) => {
    await openStubbedPreview(page);

    // A row whose reasons include the contact codes shows only the non-contact
    // code in its Warnings cell.
    const contactReasonsRow = previewRow(page, "synthetic-bulk-needs_review-1");
    const warnings = contactReasonsRow.locator('td[data-label="Warnings"]');
    await expect(warnings).toContainText("conflicting_names");
    await expect(warnings).not.toContainText("missing_verified_contact");
    await expect(warnings).not.toContainText("blocked_missing_recipient");

    // A row that only appears on the second preview page is present after the
    // load loop finishes.
    const secondPageRow = previewRow(page, SECOND_PAGE_KEY);
    await expect(secondPageRow).toBeVisible();
    const secondPageWarnings = previewRow(
      page,
      SECOND_PAGE_WARNING_KEY,
    ).locator('td[data-label="Warnings"]');
    await expect(secondPageWarnings).toContainText("needs_name_review");

    // The warning row is also on the second page; it is a needs-review row.
    await expect(
      previewRow(page, "synthetic-bulk-needs_review-1"),
    ).toBeVisible();
    await expect(page.getByTestId("bulk-filter-needs_review")).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    await expect(secondPageWarnings).toContainText("needs_name_review");

    // No contact reason string appears anywhere in the rendered table.
    await expect(page.getByTestId("bulk-table")).not.toContainText(
      "missing_verified_contact",
    );
    await expect(page.getByTestId("bulk-table")).not.toContainText(
      "blocked_missing_recipient",
    );
  });

  test("a mobile viewport renders labelled preview cards", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await openStubbedPreview(page);

    const row = previewRow(page, "synthetic-bulk-eligible_for_invite-1");
    await expect(
      row.locator('td[data-label="Select"] input[type="checkbox"]'),
    ).toBeEnabled();
    const person = row.locator('td[data-label="Person"]');
    await expect(person).toContainText("Synthetic eligible_for_invite 1");
    await expect(person).toContainText("synthetic-bulk-eligible_for_invite-1");
    await expect(row.locator('td[data-label="Contact"]')).toContainText(
      STUB_MASKED_EMAIL,
    );
    await expect(row.locator('td[data-label="Status"]')).toContainText(
      "eligible_for_invite",
    );
    await expect(
      row.locator('td[data-label="Proposed username"]'),
    ).toContainText("synthetic_bulk_eligible_for_invite_1");
    await expect(row.locator('td[data-label="Warnings"]')).toHaveText("—");

    const noneRow = previewRow(page, "synthetic-bulk-eligible_for_invite-2");
    await expect(noneRow.locator('td[data-label="Contact"]')).toHaveText(
      "None",
    );

    // Every preview row exposes its data-label attributes.
    const firstRow = page.getByTestId("bulk-table").locator("tbody tr").first();
    for (const label of [
      "Select",
      "Person",
      "Contact",
      "Status",
      "Proposed username",
      "Warnings",
    ]) {
      await expect(firstRow.locator(`td[data-label="${label}"]`)).toHaveCount(
        1,
      );
    }
  });
});
