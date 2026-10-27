import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Stage-eight mocked suite for the sign-in page and the shared denial panel.
//
// The sign-in page is server-rendered from the configured OIDC environment; the
// isolated build has a configured provider. The session endpoint is stubbed so
// the shared shell renders a deterministic anonymous account menu.

const ANONYMOUS = { state: "anonymous" };
const VIEWER = { state: "signed_in", role: "viewer", display_name: "Synthetic viewer" };

async function stubSession(page: Page, session: unknown) {
  await page.route("/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
    }),
  );
}

async function assertNoOverflow(page: Page) {
  const overflow = await page.evaluate(
    () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
  );
  expect(overflow, "no horizontal overflow").toBeLessThanOrEqual(1);
}

test.describe("sign in", () => {
  test("uses the shared shell and a primary sign-in action", async ({ page }) => {
    await stubSession(page, ANONYMOUS);
    await page.goto("/signin");

    await expect(page.getByTestId("site-header")).toBeVisible();
    await expect(page.getByTestId("primary-nav")).toBeVisible();
    await expect(page.getByTestId("signin-view")).toBeVisible();
    await expect(page.getByTestId("signin-button")).toBeVisible();
    await expect(page.getByTestId("signin-button")).toHaveAttribute(
      "href",
      "/api/auth/login",
    );
    await expect(page.getByTestId("signin-view")).toContainText(
      /provisioned by an administrator/i,
    );
    await expect(page.getByTestId("signin-error")).toHaveCount(0);

    await page.getByTestId("signin-button").focus();
    await expect(page.getByTestId("signin-button")).toBeFocused();
  });

  test("maps identity-provider errors to safe messages", async ({ page }) => {
    await stubSession(page, ANONYMOUS);

    await page.goto("/signin?error=state_mismatch");
    await expect(page.getByTestId("signin-error")).toBeVisible();
    await expect(page.getByTestId("signin-error")).toContainText(
      /could not be verified/i,
    );
    await expect(page.getByTestId("signin-error")).not.toContainText(
      "state_mismatch",
    );

    await page.goto("/signin?error=not_provisioned");
    await expect(page.getByTestId("signin-error")).toContainText(
      /not provisioned/i,
    );

    await page.goto("/signin?error=provider_unavailable");
    await expect(page.getByTestId("signin-error")).toContainText(
      /unavailable/i,
    );

    await page.goto("/signin?error=unexpected_code");
    await expect(page.getByTestId("signin-error")).toContainText(
      /could not be completed/i,
    );
  });

  test("has no horizontal overflow at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stubSession(page, ANONYMOUS);
    await page.goto("/signin");
    await expect(page.getByTestId("signin-view")).toBeVisible();
    await assertNoOverflow(page);
  });

  test("keeps the shared light and dark tokens", async ({ page }) => {
    await stubSession(page, ANONYMOUS);
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto("/signin");
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

test.describe("shared denial state", () => {
  test("denied views use the shared state panel", async ({ page }) => {
    await stubSession(page, VIEWER);
    await page.goto("/admin/document-extraction");

    const denied = page.getByTestId("denied-view");
    await expect(denied).toBeVisible();
    await expect(denied).toHaveClass(/state-panel/);
    await expect(denied).toHaveClass(/state-panel-unavailable/);
    await expect(denied).toHaveAttribute("role", "alert");
    await expect(denied).toContainText(/Access denied/);
  });

  test("denied views fit at 390px", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await stubSession(page, VIEWER);
    await page.goto("/admin/document-extraction");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await assertNoOverflow(page);
  });
});
