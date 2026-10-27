import { test, expect } from "@playwright/test";

// Real browser mutation flow: sign in through OIDC, nominate a synthetic person,
// have a second admin approve it (a nominator cannot approve their own
// nomination), and revoke the appointment. Every success is server-confirmed by
// reloading the persisted state.
const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
const APPROVER_TOKEN = process.env.E2E_APPROVER_TOKEN;

test("admin nominates, a second admin approves, and the admin revokes", async ({ page, request }) => {
  test.skip(!APPROVER_TOKEN, "E2E_APPROVER_TOKEN not provided by the harness");
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
    .fill("synthetic-e2e-person");
  await page.getByRole("option", { name: /Synthetic E2E Person/ }).click();
  await page.getByTestId("hof-nominate-submit").click();
  await page.getByTestId("hof-confirm-yes").click();

  await expect(page.getByTestId("hof-pending-nomination")).toContainText(
    "synthetic-e2e-person",
  );
  const pending = await request.get(`${API_URL}/api/genealogy/nominations`, {
    headers: { authorization: `Bearer ${APPROVER_TOKEN}` },
  });
  expect(pending.status()).toBe(200);
  const { nominations } = (await pending.json()) as {
    nominations: Array<{ nomination_id: number; record_id: string; status: string }>;
  };
  const nomination = nominations.find((n) =>
    n.record_id === "synthetic-e2e-person" && n.status === "pending");
  expect(nomination).toBeTruthy();
  const approval = await request.post(
    `${API_URL}/api/genealogy/nominations/${nomination!.nomination_id}`,
    { headers: { authorization: `Bearer ${APPROVER_TOKEN}` }, data: { approve: true } },
  );
  expect(approval.status()).toBe(200);

  await page.reload();
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
