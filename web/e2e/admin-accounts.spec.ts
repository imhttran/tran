import { expect, test, type Page } from "@playwright/test";

// Admin > Family Members > Accounts against the real Rust API and the synthetic
// OIDC issuer. The harness seeds a disposable `e2e-member` account with a
// reserved .test address and configures no password-reset provider, so a reset
// must fail closed. No real person, address, or provider is involved.

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
const WEB_URL = (process.env.WEB_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
const ADMIN_TOKEN = process.env.E2E_ADMIN_TOKEN ?? "";
const VIEWER_TOKEN = process.env.E2E_VIEWER_TOKEN ?? "";

async function signIn(page: Page) {
  await page.goto("/signin");
  await page.getByTestId("signin-button").click();
  await page.waitForURL(/127\.0\.0\.1:3000\/$/, { timeout: 30_000 });
  await expect(page.getByTestId("account-role")).toHaveText("admin");
}

// The actions panel is a <details> element that re-renders with the row after
// every mutation, which resets it to closed. Open it deterministically instead
// of assuming a previous click left it open.
async function openActions(row: ReturnType<Page["locator"]>) {
  const details = row.getByTestId("fm-actions");
  const open = await details.evaluate(
    (element) => (element as HTMLDetailsElement).open,
  );
  if (!open) {
    await details.locator("summary").click();
  }
  await expect(details).toHaveJSProperty("open", true);
}

async function openMember(page: Page) {
  await page.goto("/admin/family-members");
  await page.getByTestId("fm-search").fill("e2e-member");
  await page.getByTestId("fm-apply-filters").click();
  const row = page.locator('[data-testid="fm-member"][data-subject="e2e-member"]');
  await expect(row).toHaveCount(1);
  return row;
}

test.describe.serial("admin accounts", () => {
  test.skip(!ADMIN_TOKEN, "E2E_ADMIN_TOKEN not provided by the harness");

  test("non-admin and anonymous API calls are refused", async ({ request }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    const viewer = { authorization: `Bearer ${VIEWER_TOKEN}` };
    expect((await request.get(`${API_URL}/api/admin/family-members`, { headers: viewer })).status()).toBe(403);
    const reset = await request.post(`${API_URL}/api/admin/family-members/password-reset`, {
      headers: viewer,
      data: { issuer: "https://issuer.invalid", subject: "e2e-member" },
    });
    expect(reset.status()).toBe(403);
    expect((await request.get(`${API_URL}/api/admin/family-members`)).status()).toBe(401);
  });

  test("the proxy rejects an admin mutation without a CSRF token", async ({ page }) => {
    await signIn(page);
    const status = await page.evaluate(async () => {
      const response = await fetch("/api/admin/family-members/password-reset", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ issuer: "x", subject: "e2e-member" }),
      });
      return response.status;
    });
    expect(status).toBe(403);
  });

  test("reset fails closed without a provider and never claims an email was sent", async ({ page }) => {
    await signIn(page);
    const row = await openMember(page);
    await expect(row.getByTestId("fm-member-status")).toHaveText("active");
    await openActions(row);
    await row.getByTestId("fm-reset").click();
    await page.getByTestId("fm-confirm-yes").click();
    await expect(page.getByRole("status")).toContainText("not configured");
    await expect(page.getByRole("status")).not.toContainText("accepted");
  });

  test("admin links a reviewed account to a person and removes the link", async ({ page }) => {
    await signIn(page);
    const row = await openMember(page);
    await expect(row.getByTestId("fm-member-link")).toHaveText("Not linked to a genealogy person");
    await openActions(row);
    await row.getByTestId("fm-link-person").fill("synthetic-e2e-person");
    await row.getByTestId("fm-link").click();
    await page.getByTestId("fm-confirm-yes").click();
    await expect(row.getByTestId("fm-member-link")).toContainText("Linked to Synthetic E2E Person");
    await expect(row.getByTestId("fm-member-link")).toContainText("synthetic-e2e-person");

    await openActions(row);
    await row.getByTestId("fm-unlink").click();
    await page.getByTestId("fm-confirm-yes").click();
    await expect(row.getByTestId("fm-member-link")).toHaveText("Not linked to a genealogy person");
  });

  test("admin grants a role, cancels, then deactivates", async ({ page }) => {
    await signIn(page);
    const row = await openMember(page);
    await openActions(row);
    await row.getByTestId("fm-role-select").selectOption("editor");
    await row.getByTestId("fm-grant-role").click();
    await page.getByTestId("fm-confirm-yes").click();
    await expect(page.getByRole("status")).toContainText("Granted editor");
    await expect(row.getByTestId("fm-member-role")).toContainText(["editor"]);

    await openActions(row);
    await row.getByTestId("fm-deactivate").click();
    await page.getByTestId("fm-confirm-cancel").click();
    await expect(row.getByTestId("fm-member-status")).toHaveText("active");
    await row.getByTestId("fm-deactivate").click();
    await page.getByTestId("fm-confirm-yes").click();
    await expect(row.getByTestId("fm-member-status")).toHaveText("deactivated");
  });

  test("a viewer sees a denial and no account directory", async ({ browser }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    const context = await browser.newContext();
    await context.addCookies([{ name: "rag_session", value: VIEWER_TOKEN, url: WEB_URL, httpOnly: true }]);
    const page = await context.newPage();
    await page.goto("/admin/family-members");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("accounts")).toHaveCount(0);
    await context.close();
  });
});
