import fs from "node:fs";
import path from "node:path";

import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// USR-006-B: the family profile and biography page.
//
// Runs against the real Next.js app, the real proxy, the real Axum API, and the
// disposable genealogy database seeded by scripts/web-e2e.sh. The fixtures give
// one person an approved family biography plus an approved owner-private value
// and an unsubmitted draft, so every negative assertion is about a real value.
//
// Case map:
//  1  open a profile from the family tree            -> "opens a profile from the tree"
//  2  approved biography and memories render         -> same test
//  3  owner-private values never appear              -> same + "privacy" test
//  4  drafts/pending/rejected/unprovenanced absent   -> "privacy" test (draft fixture)
//  5  empty profile state                            -> "empty"
//  6  unlinked user denied                           -> "denied"
//  7  deactivated account cannot view                -> "deactivated"
//  8  unknown person not found                       -> "not found"
//  9  network/server errors leave no stale content   -> "errors", "unexpected error"
// 10  switching people shows only the new person     -> "person switch", "transition"
// 11  no media or contact data exposed               -> "privacy"
// 12  mobile layout and keyboard navigation          -> "mobile and keyboard"

const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const VIEWER = process.env.E2E_VIEWER_TOKEN || "";
const DEACTIVATED = process.env.E2E_DEACTIVATED_TOKEN || "";
const PRIVATE_MARKER = process.env.E2E_PRIVATE_PROFILE_MARKER || "";
const FAMILY_MARKER = process.env.E2E_FAMILY_PROFILE_MARKER || "";
const DRAFT_MARKER = process.env.E2E_FAMILY_DRAFT_MARKER || "";
// Synthetic screenshot for the PR and the acceptance record.
const repo = path.resolve(process.cwd(), "..");
const screenshots = path.join(repo, "docs/reports/rust-migration/web-e2e");

const SUBJECT = "e2e-branch-subject";
const SUBJECT_NAME = "Synthetic Branch Subject";
const EMPTY_PERSON = "e2e-branch-owner";
const EMPTY_NAME = "Synthetic Linked Reviewer Person";
const READY = Boolean(
  ISSUER &&
    ADMIN &&
    REVIEWER &&
    VIEWER &&
    DEACTIVATED &&
    PRIVATE_MARKER &&
    FAMILY_MARKER &&
    DRAFT_MARKER,
);

function auth(token: string) {
  return { authorization: `Bearer ${token}` };
}

async function api(
  request: APIRequestContext,
  token: string,
  method: "GET" | "POST",
  path: string,
  data?: unknown,
) {
  const response = await request.fetch(`${API_URL}${path}`, {
    method,
    headers: auth(token),
    data,
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

async function useToken(page: Page, token: string) {
  await page.context().addCookies([
    {
      name: "rag_session",
      value: token,
      url: ORIGIN,
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_csrf",
      value: "e2e-csrf-token",
      url: ORIGIN,
      sameSite: "Lax",
    },
  ]);
}

async function selectPersonInTree(page: Page, name: string) {
  await page.goto("/genealogy");
  const record = page
    .getByTestId("genealogy-records")
    .getByRole("button", { name });
  await expect(record).toBeVisible();
  await record.click();
  await expect(page.getByTestId("family-person-details")).toContainText(name);
  return page.getByTestId("open-family-profile");
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.beforeAll(() => {
  fs.mkdirSync(screenshots, { recursive: true });
});

// The deactivation case uses the shared deactivated fixture only inside this
// file; it is provisioned again afterwards so later specs see it active.
test.afterAll(async ({ request }) => {
  if (!READY) return;
  await api(request, ADMIN, "POST", "/api/admin/users", {
    issuer: ISSUER,
    subject: "e2e-deactivated",
  });
});

test.describe.serial("family profile page", () => {
  test("opens a profile from the family tree and renders approved content", async ({
    page,
  }) => {
    await useToken(page, REVIEWER);
    const link = await selectPersonInTree(page, SUBJECT_NAME);
    await expect(link).toHaveAttribute("href", `/family/${SUBJECT}`);
    await link.click();

    await expect(page).toHaveURL(new RegExp(`/family/${SUBJECT}$`));
    await expect(page.getByRole("heading", { level: 1 })).toHaveText(SUBJECT_NAME);
    await expect(page.getByTestId("family-profile")).toBeVisible();
    await expect(
      page.getByTestId("family-profile-field-biography"),
    ).toContainText(FAMILY_MARKER);
    // Provenance is subtle and carries no reviewer identity.
    await expect(
      page.getByTestId("family-profile-field-biography"),
    ).toContainText("Approved version 1");
    await expect(page.getByTestId("family-profile-back")).toHaveAttribute(
      "href",
      "/genealogy",
    );
    await page.screenshot({
      path: path.join(screenshots, "family-profile.png"),
      fullPage: true,
    });
  });

  test("never renders owner-private values, drafts, or contact fields", async ({
    page,
  }) => {
    const requests: string[] = [];
    page.on("request", (request) => requests.push(request.url()));
    await useToken(page, REVIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-field-biography")).toContainText(
      FAMILY_MARKER,
    );

    const body = await page.locator("body").innerText();
    expect(body).not.toContain(PRIVATE_MARKER);
    expect(body).not.toContain(DRAFT_MARKER);
    for (const forbidden of [
      "current_location",
      "Contact preferences",
      "Facebook",
      "LinkedIn",
      "Snapchat",
      "WhatsApp",
      "Current location",
    ]) {
      expect(body).not.toContain(forbidden);
    }
    // No media and no private profile requests from this page.
    expect(await page.locator("img").count()).toBe(0);
    expect(requests.filter((url) => url.includes("/api/profile"))).toEqual([]);
    expect(requests.filter((url) => url.includes("/media/"))).toEqual([]);
  });

  test("an authorized profile with nothing published shows the empty state", async ({
    page,
  }) => {
    await useToken(page, REVIEWER);
    await page.goto(`/family/${EMPTY_PERSON}`);
    await expect(page.getByTestId("family-profile-empty")).toBeVisible();
    await expect(page.getByTestId("family-profile-empty")).toContainText(
      EMPTY_NAME,
    );
    await expect(page.getByTestId("family-profile")).toBeVisible();
    const body = await page.locator("body").innerText();
    expect(body).not.toContain(PRIVATE_MARKER);
    expect(body).not.toContain(FAMILY_MARKER);
  });

  test("an unlinked account gets a safe denial and no profile data", async ({
    page,
  }) => {
    await useToken(page, VIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("family-profile")).toHaveCount(0);
    const body = await page.locator("body").innerText();
    expect(body).not.toContain(SUBJECT_NAME);
    expect(body).not.toContain(FAMILY_MARKER);
    expect(body).not.toContain(PRIVATE_MARKER);
  });

  test("a deactivated account cannot view family content", async ({
    page,
    request,
  }) => {
    // The same linked account works before deactivation, so the denial below is
    // caused by deactivation and not by a missing membership.
    const before = await api(
      request,
      DEACTIVATED,
      "GET",
      `/api/family/persons/${SUBJECT}/profile`,
    );
    expect(before.status, JSON.stringify(before.body)).toBe(200);
    // The fixture person may have published values; the point is only that the
    // same request succeeds before deactivation.
    expect(["published", "empty"]).toContain(before.body.state);

    const deactivated = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/users/e2e-deactivated/deactivate?issuer=${encodeURIComponent(ISSUER)}`,
    );
    expect(deactivated.status, JSON.stringify(deactivated.body)).toBe(200);

    const after = await api(
      request,
      DEACTIVATED,
      "GET",
      `/api/family/persons/${SUBJECT}/profile`,
    );
    expect([401, 403]).toContain(after.status);

    await useToken(page, DEACTIVATED);
    await page.goto(`/family/${SUBJECT}`);
    await expect(
      page.getByTestId("denied-view").or(page.getByTestId("family-profile-signin")),
    ).toBeVisible();
    await expect(page.getByTestId("family-profile")).toHaveCount(0);
    const body = await page.locator("body").innerText();
    expect(body).not.toContain(FAMILY_MARKER);
    expect(body).not.toContain(PRIVATE_MARKER);
  });

  test("an unknown person shows a neutral not-found state", async ({ page }) => {
    await useToken(page, REVIEWER);
    await page.goto("/family/does-not-exist");
    await expect(page.getByTestId("family-profile-not-found")).toBeVisible();
    await expect(page.getByTestId("family-profile")).toHaveCount(0);
  });

  test("server and network failures replace content and offer a retry", async ({
    page,
  }) => {
    await useToken(page, REVIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-field-biography")).toContainText(
      FAMILY_MARKER,
    );

    // A 503 on the next request must not leave the previous content visible.
    await page.route("**/api/family/persons/**", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({
          ok: false,
          error: "family_profile_unavailable",
          message: "the family profile store is unavailable",
        }),
      }),
    );
    await page.goto(`/family/${EMPTY_PERSON}`);
    await expect(page.getByTestId("family-profile-unavailable")).toBeVisible();
    await expect(page.getByTestId("family-profile")).toHaveCount(0);
    expect(await page.locator("body").innerText()).not.toContain(FAMILY_MARKER);

    // Recovering from the failure shows real content again.
    await page.unroute("**/api/family/persons/**");
    await page.getByTestId("family-profile-retry").click();
    await expect(page.getByTestId("family-profile-empty")).toBeVisible();

    // A network failure is also a safe state with a retry.
    await page.route("**/api/family/persons/**", (route) => route.abort());
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-unavailable")).toBeVisible();
    expect(await page.locator("body").innerText()).not.toContain(FAMILY_MARKER);
    await page.unroute("**/api/family/persons/**");
  });

  test("switching people never shows the previous person's profile", async ({
    page,
  }) => {
    await useToken(page, REVIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-field-biography")).toContainText(
      FAMILY_MARKER,
    );

    // Navigate through the tree, which is a client-side transition.
    await page.getByTestId("family-profile-back").click();
    await expect(page).toHaveURL(/\/genealogy$/);
    const record = page
      .getByTestId("genealogy-records")
      .getByRole("button", { name: EMPTY_NAME });
    await record.click();
    await page.getByTestId("open-family-profile").click();

    await expect(page).toHaveURL(new RegExp(`/family/${EMPTY_PERSON}$`));
    await expect(page.getByTestId("family-profile-empty")).toBeVisible();
    const body = await page.locator("body").innerText();
    expect(body).not.toContain(FAMILY_MARKER);
    expect(body).not.toContain(SUBJECT_NAME);
  });


  test("an unexpected server error renders neither its body nor stale content", async ({
    page,
  }) => {
    await useToken(page, REVIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-field-biography")).toContainText(
      FAMILY_MARKER,
    );

    // A 500 carrying a distinctive private marker in its body. The page must
    // show fixed text and neither the marker nor the previous profile.
    await page.route("**/api/family/persons/**", (route) =>
      route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          ok: false,
          error: "internal_error",
          message: `unexpected failure ${PRIVATE_MARKER}`,
        }),
      }),
    );

    // Same-route reload is a fresh mount; the client-side transition below is
    // the interesting one.
    await page.goto(`/family/${EMPTY_PERSON}`);
    await expect(page.getByTestId("family-profile-unavailable")).toBeVisible();
    let body = await page.locator("body").innerText();
    expect(body).not.toContain(PRIVATE_MARKER);
    expect(body).not.toContain(FAMILY_MARKER);
    expect(body).toContain("could not be loaded");

    // Client-side transition into an error must also replace the old profile.
    await page.unroute("**/api/family/persons/**");
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-field-biography")).toContainText(
      FAMILY_MARKER,
    );
    await page.route("**/api/family/persons/**", (route) =>
      route.fulfill({
        status: 500,
        contentType: "application/json",
        body: JSON.stringify({
          ok: false,
          error: "internal_error",
          message: `unexpected failure ${PRIVATE_MARKER}`,
        }),
      }),
    );
    await page.getByTestId("family-profile-back").click();
    await page
      .getByTestId("genealogy-records")
      .getByRole("button", { name: EMPTY_NAME })
      .click();
    await page.getByTestId("open-family-profile").click();
    await expect(page.getByTestId("family-profile-unavailable")).toBeVisible();
    body = await page.locator("body").innerText();
    expect(body).not.toContain(PRIVATE_MARKER);
    expect(body).not.toContain(FAMILY_MARKER);
    await page.unroute("**/api/family/persons/**");
  });

  test("a person change clears the previous profile while the new one loads", async ({
    page,
  }) => {
    await useToken(page, REVIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile-field-biography")).toContainText(
      FAMILY_MARKER,
    );

    // Hold the next profile response so the transition can be observed. The
    // previous person's content must be gone before the new data arrives.
    await page.route("**/api/family/persons/**", async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1_500));
      await route.continue();
    });
    await page.getByTestId("family-profile-back").click();
    await page
      .getByTestId("genealogy-records")
      .getByRole("button", { name: EMPTY_NAME })
      .click();
    await page.getByTestId("open-family-profile").click();

    await expect(page.getByTestId("family-profile-loading")).toBeVisible();
    const during = await page.locator("body").innerText();
    expect(during).not.toContain(FAMILY_MARKER);
    expect(during).not.toContain(SUBJECT_NAME);

    await expect(page.getByTestId("family-profile-empty")).toBeVisible();
    const after = await page.locator("body").innerText();
    expect(after).not.toContain(FAMILY_MARKER);
    await page.unroute("**/api/family/persons/**");
  });

  test("the page is usable on mobile and by keyboard", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await useToken(page, REVIEWER);
    await page.goto(`/family/${SUBJECT}`);
    await expect(page.getByTestId("family-profile")).toBeVisible();
    await expect(page.getByRole("heading", { level: 1 })).toHaveText(SUBJECT_NAME);
    await expect(page.getByTestId("family-profile-section-about")).toBeVisible();

    // The back link is reachable and activates with the keyboard.
    const back = page.getByTestId("family-profile-back");
    await back.focus();
    await expect(back).toBeFocused();
    await page.keyboard.press("Enter");
    await expect(page).toHaveURL(/\/genealogy$/);

    // No horizontal overflow at the mobile width.
    const [scrollWidth, clientWidth] = await page.evaluate(() => [
      document.documentElement.scrollWidth,
      document.documentElement.clientWidth,
    ]);
    expect(scrollWidth).toBeLessThanOrEqual(clientWidth + 1);
  });
});
