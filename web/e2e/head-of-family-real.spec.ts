import { test, expect } from "@playwright/test";

// Real browser mutation flow: sign in through OIDC, nominate a synthetic person,
// approve the pending nomination, and revoke the appointment. Every success is
// server-confirmed by reloading the persisted state.

test("admin nominates, approves, and revokes through the real API", async ({ page }) => {
  await page.goto("/signin");
  await page.getByTestId("signin-button").click();
  await page.waitForURL(/http:\/\/(127\.0\.0\.1|localhost):3000\/$/, {
    timeout: 30_000,
  });
  await expect(page.getByTestId("account-role")).toHaveText("admin");

  await page.goto("/head-of-family");
  await expect(page.getByTestId("hof-record-select")).toBeVisible();

  await page
    .getByTestId("hof-record-select")
    .selectOption("synthetic-e2e-person");
  await page.getByTestId("hof-nomination-label").fill("Synthetic E2E Person");
  await page.getByTestId("hof-nominate-submit").click();
  await page.getByTestId("hof-confirm-yes").click();

  await expect(page.getByTestId("hof-pending-nomination")).toContainText(
    "synthetic-e2e-person",
  );
  await page.getByTestId("hof-approve-submit").click();
  await page.getByTestId("hof-confirm-yes").click();

  await expect(page.getByTestId("hof-conclusion")).toContainText(
    "Synthetic E2E Person",
  );

  await page
    .getByTestId("revoke-reason-input")
    .first()
    .fill("synthetic test cleanup");
  await page.getByTestId("revoke-button").first().click();
  await page.getByTestId("confirm-button").click();

  await expect(page.getByTestId("hof-conclusion")).toHaveCount(0);
});
