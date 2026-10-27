import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Stage-six mocked Head of Family suite.
//
// No database, API, OIDC issuer, or credentials are needed. Every person,
// appointment, citation, and nomination below is synthetic.

const SESSION_ADMIN = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};
const SESSION_EDITOR = {
  state: "signed_in",
  role: "editor",
  display_name: "Synthetic editor",
};
const SESSION_VIEWER = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};

const HOF = {
  catalog: "available",
  conclusions: [
    {
      conclusion: "Head of Family",
      record_id: "nguyen-van-an",
      appointment_id: "appointment-7",
      head_label: "Nguyễn Văn An",
      evidence: [
        {
          evidence_id: "ev-1",
          citation: "Synthetic family register p.4",
          document_ref: "synthetic/register.pdf",
          page: 4,
          locator: "entry 4",
          excerpt: "Nguyễn Văn An giữ sổ hộ và ghi chép các thành viên.",
        },
        {
          evidence_id: "ev-2",
          citation: "Synthetic family letter",
          document_ref: "synthetic/letters.pdf",
          page: 2,
          locator: null,
          excerpt: null,
        },
      ],
      evidence_status: "verified",
    },
  ],
};

const HOF_EMPTY = { catalog: "available", conclusions: [] };
const HOF_UNAVAILABLE = {
  catalog: "unavailable",
  reason: "Synthetic catalog outage.",
};

const RECORDS = {
  records: [
    { record_id: "nguyen-van-an", person_label: "Nguyễn Văn An" },
    { record_id: "tran-thi-binh", person_label: "Trần Thị Bình" },
  ],
};

const NOMINATIONS = {
  nominations: [
    { nomination_id: 11, record_id: "nguyen-van-an", status: "pending" },
  ],
};

async function signIn(page: Page, session: unknown) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
    }),
  );
}

async function stubHead(
  page: Page,
  { status = 200, body = HOF }: { status?: number; body?: unknown } = {},
) {
  await page.route("**/api/genealogy/head-of-family", (route) =>
    route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify(body),
    }),
  );
}

async function stubActions(page: Page) {
  await page.route("**/api/genealogy/records", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(RECORDS),
    }),
  );
  await page.route("**/api/genealogy/nominations", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(NOMINATIONS),
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

async function assertNoSensitive(page: Page) {
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

test.describe("Head of Family", () => {
  test("admin sees the designation, evidence, and existing controls", async ({
    page,
  }) => {
    await signIn(page, SESSION_ADMIN);
    await stubHead(page);
    await stubActions(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-view")).toBeVisible();
    await expect(page.getByTestId("hof-head-name").first()).toContainText(
      "Nguyễn Văn An",
    );
    await expect(page.getByTestId("hof-view")).toContainText(
      /separate from (a )?system administrator/i,
    );
    await expect(page.getByTestId("hof-record-meta").first()).toContainText(
      "nguyen-van-an",
    );
    await expect(page.getByTestId("hof-record-meta").first()).toContainText(
      "appointment-7",
    );
    await expect(page.getByTestId("evidence-verified").first()).toBeVisible();

    await page
      .getByTestId("citation-button")
      .filter({ hasText: "Synthetic family register" })
      .click();
    await expect(page.getByTestId("source-details")).toContainText(
      "Synthetic family register p.4",
    );
    await expect(page.getByTestId("source-metadata-only")).toBeVisible();
    await page.getByRole("button", { name: "Close" }).click();

    await expect(page.getByTestId("nomination-input").first()).toBeVisible();
    await expect(page.getByTestId("nominate-button").first()).toBeVisible();
    await expect(page.getByTestId("approve-button").first()).toBeVisible();
    await expect(page.getByTestId("revoke-button").first()).toBeVisible();

    await expect(page.getByTestId("hof-actions")).toBeVisible();
    await expect(page.getByTestId("hof-record-select")).toBeVisible();
    await expect(page.getByTestId("hof-pending-nomination")).toBeVisible();
    await assertNoSensitive(page);
  });

  test("confirm dialog takes focus and Escape cancels without a request", async ({
    page,
  }) => {
    await signIn(page, SESSION_ADMIN);
    await stubHead(page);
    await stubActions(page);
    let approvals = 0;
    await page.route("**/api/genealogy/nominations/**", (route) => {
      approvals += 1;
      return route.fulfill({ status: 200, body: "{}" });
    });
    await page.goto("/head-of-family");

    await page.getByTestId("approve-button").first().click();
    await expect(page.getByTestId("confirm-dialog")).toBeVisible();
    await expect(page.getByTestId("confirm-button")).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
    expect(approvals).toBe(0);
  });

  test("no active appointment shows an explicit empty state", async ({
    page,
  }) => {
    await signIn(page, SESSION_VIEWER);
    await stubHead(page, { body: HOF_EMPTY });
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-view")).toBeVisible();
    await expect(page.getByTestId("hof-empty")).toBeVisible();
    await expect(page.getByTestId("hof-empty")).toContainText(
      /no (current )?head of family/i,
    );
    await expect(page.getByTestId("hof-head-name")).toHaveCount(0);
    await expect(page.getByTestId("hof-view")).not.toContainText(
      "Nguyễn Văn An",
    );
  });

  test("editor can nominate but cannot approve or revoke", async ({ page }) => {
    await signIn(page, SESSION_EDITOR);
    await stubHead(page);
    await stubActions(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("nominate-button").first()).toBeVisible();
    await expect(page.getByTestId("hof-nominate-submit")).toBeVisible();
    await expect(page.getByTestId("approve-button")).toHaveCount(0);
    await expect(page.getByTestId("revoke-button")).toHaveCount(0);
    await expect(page.getByTestId("hof-approve-submit")).toHaveCount(0);
  });

  test("viewer sees no mutation controls", async ({ page }) => {
    await signIn(page, SESSION_VIEWER);
    await stubHead(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-conclusion").first()).toBeVisible();
    await expect(page.getByTestId("nominate-button")).toHaveCount(0);
    await expect(page.getByTestId("approve-button")).toHaveCount(0);
    await expect(page.getByTestId("revoke-button")).toHaveCount(0);
    await expect(page.getByTestId("hof-actions")).toHaveCount(0);
  });

  test("denied response renders the denied state", async ({ page }) => {
    await signIn(page, SESSION_ADMIN);
    await stubHead(page, { status: 403 });
    await stubActions(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-denied")).toBeVisible();
    await expect(page.getByTestId("hof-view")).toHaveCount(0);
  });

  test("no-access response renders the no-access state", async ({ page }) => {
    await signIn(page, SESSION_ADMIN);
    await stubHead(page, { status: 404 });
    await stubActions(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-no-access")).toBeVisible();
    await expect(page.getByTestId("hof-view")).toHaveCount(0);
  });

  test("expired response offers a sign-in action", async ({ page }) => {
    await signIn(page, SESSION_ADMIN);
    await stubHead(page, { status: 401 });
    await stubActions(page);
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-expired")).toBeVisible();
    await expect(
      page.getByTestId("hof-expired").getByRole("link", { name: /sign in/i }),
    ).toBeVisible();
  });

  test("unavailable catalog is explicit and names no head", async ({
    page,
  }) => {
    await signIn(page, SESSION_VIEWER);
    await stubHead(page, { body: HOF_UNAVAILABLE });
    await page.goto("/head-of-family");

    await expect(page.getByTestId("hof-catalog-unavailable")).toBeVisible();
    await expect(page.getByTestId("hof-catalog-unavailable")).toContainText(
      "Synthetic catalog outage.",
    );
    await expect(page.getByTestId("hof-view")).toHaveCount(0);
  });

  test("loading state is explicit", async ({ page }) => {
    await signIn(page, SESSION_ADMIN);
    await stubActions(page);
    let release: () => void = () => {};
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    await page.route("**/api/genealogy/head-of-family", async (route) => {
      await gate;
      await route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(HOF),
      });
    });
    await page.goto("/head-of-family", { waitUntil: "domcontentloaded" });
    await expect(page.getByTestId("hof-loading")).toBeVisible();
    release();
    await expect(page.getByTestId("hof-view")).toBeVisible();
  });

  test("confirm dialog fits at 390px without overflow", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, SESSION_ADMIN);
    await stubHead(page);
    await stubActions(page);
    await page.goto("/head-of-family");
    await page.getByTestId("approve-button").first().click();
    await expect(page.getByTestId("confirm-dialog")).toBeVisible();
    await assertNoOverflow(page);
  });

  test("keeps the stage-three light and dark tokens", async ({ page }) => {
    await signIn(page, SESSION_VIEWER);
    await stubHead(page);
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
