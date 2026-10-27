import { expect, test } from "@playwright/test";

// UI-601 — fully mocked browser coverage for /head-of-family. Every API route is
// intercepted; no real backend or database is used. The assertions cover the
// hero hierarchy, the six states, keyboard operation of both confirm dialogs,
// 390px overflow, and the viewer/editor/administrator role gates.

const HEAD_URL = "/api/genealogy/head-of-family";
const PERSON_SEARCH_URL = "**/api/genealogy/person-search*";
const NOMINATIONS_URL = "/api/genealogy/nominations";

const availableBody = {
  catalog: "available",
  conclusions: [
    {
      conclusion: "Trần Văn An is the recorded head of the family.",
      record_id: "rec-hof-0001",
      appointment_id: "appt-hof-0009",
      nomination_id: "3",
      head_label: "Trần Văn An",
      evidence_status: "verified",
      evidence: [
        {
          evidence_id: "ev-1",
          citation: "Gia phả họ Trần, quyển 1",
          document_ref: "doc-1",
          page: 12,
          excerpt: "Trần Văn An được ghi là trưởng tộc.",
        },
      ],
    },
  ],
};

const emptyBody = { catalog: "available", conclusions: [] };

async function mockSession(
  page: import("@playwright/test").Page,
  role: "viewer" | "editor" | "admin" | null,
) {
  await page.route("/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(
        role
          ? { state: "signed_in", role, display_name: "Mock User" }
          : { state: "anonymous" },
      ),
    }),
  );
}

async function mockHead(
  page: import("@playwright/test").Page,
  status: number,
  body: unknown,
) {
  await page.route(HEAD_URL, (route) =>
    route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify(body),
    }),
  );
}

type MockRecord = {
  record_id: string;
  person_label: string;
  branch?: string;
  birth_year?: number;
  generation?: number;
};

function fold(value: string) {
  return value
    .normalize("NFD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/đ/g, "d")
    .toLowerCase();
}

async function mockGenealogy(
  page: import("@playwright/test").Page,
  options: {
    records?: MockRecord[];
    nominations?: unknown[];
    searchStatus?: number;
    searchDelayMs?: number;
  } = {},
) {
  const candidates = (options.records ?? []).map((record) => ({
    person_key: record.record_id,
    display_name: record.person_label,
    generation: record.generation ?? null,
    evidence_status: "unverified",
    birth_year: record.birth_year ?? null,
    branch: record.branch ?? null,
    match: "partial",
    matched_name: record.person_label,
    recorded_parents: [],
  }));
  await page.route(PERSON_SEARCH_URL, async (route) => {
    if (options.searchDelayMs) {
      await new Promise((resolve) => setTimeout(resolve, options.searchDelayMs));
    }
    const status = options.searchStatus ?? 200;
    if (status !== 200) {
      return route.fulfill({
        status,
        contentType: "application/json",
        body: JSON.stringify({ message: "search unavailable" }),
      });
    }
    const query = new URL(route.request().url()).searchParams.get("q") ?? "";
    const normalized = fold(query.trim());
    const matches = normalized
      ? candidates.filter((candidate) =>
          fold(candidate.display_name).includes(normalized),
        )
      : [];
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        candidates: matches,
        candidates_truncated: false,
        catalog_truncated: false,
      }),
    });
  });
  await page.route(NOMINATIONS_URL, (route) => {
    if (route.request().method() === "POST") {
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true }),
      });
    }
    return route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ nominations: options.nominations ?? [] }),
    });
  });
}

function expectNoHorizontalOverflow(page: import("@playwright/test").Page) {
  return page.evaluate(
    () =>
      document.documentElement.scrollWidth <=
      document.documentElement.clientWidth + 1,
  );
}

async function expectNoSensitiveLeak(page: import("@playwright/test").Page) {
  const text = (await page.locator("body").innerText()).toLowerCase();
  for (const needle of [
    "password",
    "client_secret",
    "database_url",
    "postgres://",
    "jwks",
    "refresh_token",
  ]) {
    expect(text).not.toContain(needle);
  }
}

test.describe("Head of Family surface", () => {
  test("renders the hero, metadata, and evidence hierarchy", async ({
    page,
  }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-view")).toBeVisible();
    await expect(page.getByTestId("hof-head-name")).toHaveText("Trần Văn An");
    await expect(page.getByTestId("hof-appointment-count")).toContainText(
      "1 appointment",
    );
    await expect(page.getByTestId("hof-record-id")).toHaveText("rec-hof-0001");
    await expect(page.getByTestId("hof-appointment-id")).toHaveText(
      "appt-hof-0009",
    );
    await expect(page.getByTestId("hof-view")).toContainText(
      "separate from application administrator permissions",
    );
    await expect(page.getByTestId("hof-conclusion")).toBeVisible();
    await expect(page.getByTestId("evidence-verified")).toBeVisible();

    await page.getByTestId("citation-button").first().click();
    await expect(page.getByTestId("source-details")).toBeVisible();
    await expect(page.getByTestId("source-metadata-only")).toBeVisible();
    await page.getByRole("button", { name: "Close" }).click();
    await expectNoSensitiveLeak(page);
  });

  test("shows the loading panel with role=status and aria-busy", async ({
    page,
  }) => {
    await mockSession(page, "viewer");
    await page.route(HEAD_URL, async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 800));
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(availableBody),
      });
    });
    await mockGenealogy(page);
    await page.goto("/head-of-family");

    const loading = page.getByTestId("hof-loading");
    await expect(loading).toBeVisible();
    await expect(loading).toHaveAttribute("role", "status");
    await expect(loading).toHaveAttribute("aria-busy", "true");
    await expect(page.getByTestId("hof-view")).toBeVisible();
  });

  test("shows an empty panel that names no successor", async ({ page }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 200, emptyBody);
    await mockGenealogy(page);
    await page.goto("/head-of-family");

    const empty = page.getByTestId("hof-empty");
    await expect(empty).toBeVisible();
    await expect(empty).toContainText("No active appointment");
    await expect(empty).toContainText("No successor is");
    await expect(page.getByTestId("hof-head-name")).not.toBeVisible();
    await expect(page.getByTestId("hof-conclusion")).toHaveCount(0);
  });

  test("shows distinct denied and no-access panels", async ({ page }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 403, { message: "forbidden" });
    await mockGenealogy(page);
    await page.goto("/head-of-family");
    const denied = page.getByTestId("hof-denied");
    await expect(denied).toBeVisible();
    await expect(denied).toHaveAttribute("role", "alert");
    await expect(
      denied.getByRole("link", { name: "Back to the family tree" }),
    ).toBeVisible();
    await expect(page.getByTestId("hof-no-access")).toHaveCount(0);

    await mockHead(page, 404, { message: "not found" });
    await page.goto("/head-of-family");
    const noAccess = page.getByTestId("hof-no-access");
    await expect(noAccess).toBeVisible();
    await expect(noAccess).toHaveAttribute("role", "alert");
    await expect(
      noAccess.getByRole("link", { name: "Back to the family tree" }),
    ).toBeVisible();
    await expect(page.getByTestId("hof-denied")).toHaveCount(0);
  });

  test("shows the expired panel with a sign-in action", async ({ page }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 401, { message: "expired" });
    await mockGenealogy(page);
    await page.goto("/head-of-family");

    const expired = page.getByTestId("hof-expired");
    await expect(expired).toBeVisible();
    await expect(expired).toHaveAttribute("role", "alert");
    await expect(
      expired.getByRole("link", { name: "Sign in again" }),
    ).toBeVisible();
  });

  test("shows the catalog-unavailable panel with no inferred head", async ({
    page,
  }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 200, {
      catalog: "unavailable",
      reason: "The index is rebuilding.",
    });
    await mockGenealogy(page);
    await page.goto("/head-of-family");

    const unavailable = page.getByTestId("hof-catalog-unavailable");
    await expect(unavailable).toBeVisible();
    await expect(unavailable).toContainText("not available");
    await expect(page.getByTestId("hof-head-name")).not.toBeVisible();
  });

  test("viewers see no mutation controls", async ({ page }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-view")).toBeVisible();
    await expect(page.getByTestId("nominate-button")).toHaveCount(0);
    await expect(page.getByTestId("approve-button")).toHaveCount(0);
    await expect(page.getByTestId("revoke-button")).toHaveCount(0);
    await expect(page.getByTestId("hof-actions")).toHaveCount(0);
  });

  test("editors nominate with a keyboard-operable confirm dialog", async ({
    page,
  }) => {
    await mockSession(page, "editor");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page, {
      records: [{ record_id: "rec-hof-0001", person_label: "Trần Văn An" }],
    });
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-actions")).toBeVisible();
    await expect(page.getByTestId("hof-approve-submit")).toHaveCount(0);
    await expect(page.getByTestId("approve-button")).toHaveCount(0);

    await page.getByTestId("nomination-input").first().fill("Trần Thị Bình");
    await page.getByTestId("nominate-button").first().click();

    const dialog = page.getByTestId("confirm-dialog");
    await expect(dialog).toBeVisible();
    await expect(page.getByTestId("confirm-button")).toBeFocused();

    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    await expect(page.getByTestId("nominate-button").first()).toBeFocused();
  });

  test("administrators approve and revoke with confirmation", async ({
    page,
  }) => {
    await mockSession(page, "admin");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page, {
      records: [{ record_id: "rec-hof-0001", person_label: "Trần Văn An" }],
      nominations: [
        { nomination_id: 3, record_id: "rec-hof-0002", status: "pending" },
      ],
    });
    await page.goto("/head-of-family");

    await expect(page.getByTestId("approve-button")).toBeVisible();
    await expect(page.getByTestId("revoke-button")).toBeVisible();
    await expect(page.getByTestId("hof-approve-submit")).toBeVisible();
    await expect(page.getByTestId("hof-record-select")).toBeVisible();
    await expect(page.getByTestId("hof-pending-nomination")).toBeVisible();

    await page.getByTestId("approve-button").first().click();
    const approveDialog = page.getByTestId("confirm-dialog");
    await expect(approveDialog).toBeVisible();
    await expect(page.getByTestId("confirm-button")).toBeFocused();
    await page.getByTestId("cancel-button").click();
    await expect(approveDialog).toHaveCount(0);
    await expect(page.getByTestId("approve-button").first()).toBeFocused();

    await page.getByTestId("hof-approve-submit").click();
    const actionsDialog = page.getByTestId("hof-confirm");
    await expect(actionsDialog).toBeVisible();
    await expect(page.getByTestId("hof-confirm-yes")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(actionsDialog).toHaveCount(0);
    await expect(page.getByTestId("hof-approve-submit")).toBeFocused();
  });

  test("searches all people, supports keyboard selection, and auto-fills the label", async ({
    page,
  }) => {
    await mockSession(page, "editor");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page, {
      records: [
        {
          record_id: "person-101",
          person_label: "Trần Văn An",
          branch: "Branch 1",
          birth_year: 1911,
        },
        {
          record_id: "person-102",
          person_label: "Trần Văn Ân",
          branch: "Branch 2",
          birth_year: 1912,
        },
      ],
    });
    await page.goto("/head-of-family");

    const input = page.getByTestId("hof-record-select");
    await input.fill("Tran");
    await expect(page.getByTestId("hof-record-option")).toHaveCount(2);
    await expect(page.getByTestId("hof-record-option").first()).toContainText(
      "Trần Văn An",
    );
    await expect(page.getByTestId("hof-record-option").first()).toContainText(
      "Branch 1",
    );
    await expect(page.getByTestId("hof-record-option").first()).toContainText(
      "Born 1911",
    );

    await input.press("ArrowDown");
    await input.press("Enter");
    await expect(page.getByTestId("hof-nomination-label")).toHaveValue(
      "Trần Văn Ân",
    );
    await expect(page.getByTestId("hof-selected-person")).toContainText(
      "Trần Văn Ân",
    );
    await expect(page.getByTestId("hof-nominate-submit")).toBeEnabled();

    await input.fill("Trần Văn A");
    await expect(page.getByTestId("hof-nomination-label")).toHaveValue("");
    await expect(page.getByTestId("hof-selected-person")).toHaveCount(0);
    await page.getByTestId("hof-nomination-label").fill("Free text only");
    await expect(page.getByTestId("hof-nominate-submit")).toBeDisabled();
  });

  test("shows loading, empty, and error search states", async ({ page }) => {
    await mockSession(page, "editor");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page, {
      records: [],
      searchDelayMs: 500,
    });
    await page.goto("/head-of-family");

    const input = page.getByTestId("hof-record-select");
    await input.fill("Unknown");
    await expect(page.getByTestId("hof-search-loading")).toBeVisible();
    await expect(page.getByTestId("hof-search-empty")).toBeVisible();

    await mockGenealogy(page, {
      records: [],
      searchStatus: 500,
    });
    await input.fill("Another");
    await expect(page.getByTestId("hof-search-error")).toBeVisible();
  });

  test("no horizontal overflow at 390px and desktop", async ({ page }) => {
    await mockSession(page, "admin");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page, {
      records: [
        {
          record_id: "rec-hof-0001",
          person_label: "Trần Văn An với tên rất dài để kiểm tra diacritics",
        },
      ],
      nominations: [
        { nomination_id: 3, record_id: "rec-hof-0002", status: "pending" },
      ],
    });

    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto("/head-of-family");
    await expect(page.getByTestId("hof-view")).toBeVisible();
    expect(await expectNoHorizontalOverflow(page)).toBe(true);

    await page.setViewportSize({ width: 390, height: 844 });
    await expect(page.getByTestId("hof-view")).toBeVisible();
    expect(await expectNoHorizontalOverflow(page)).toBe(true);

    await page.getByTestId("approve-button").first().click();
    await expect(page.getByTestId("confirm-dialog")).toBeVisible();
    expect(await expectNoHorizontalOverflow(page)).toBe(true);
  });

  test("keeps the shared light and dark tokens", async ({ page }) => {
    await mockSession(page, "viewer");
    await mockHead(page, 200, availableBody);
    await mockGenealogy(page);
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto("/head-of-family");
    await expect
      .poll(async () =>
        page.evaluate(() =>
          getComputedStyle(document.documentElement)
            .getPropertyValue("--bg")
            .trim()
            .toLowerCase(),
        ),
      )
      .toBe("#f6f6f7");

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
      .toBe("#121214");
  });
});
