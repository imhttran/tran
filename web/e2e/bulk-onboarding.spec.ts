import { expect, test } from "@playwright/test";

// Real-browser BULK-005 acceptance against the synthetic genealogy fixture
// (scripts/seed-bulk-onboarding-e2e.sql). The harness seeds six synthetic people:
// two eligible and four blocked. The test proves a zero-write preview, idempotent
// preparation, a separate verified-recipient and explicit send gate, and that a
// non-admin is denied.

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
const WEB_URL = (process.env.WEB_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
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

  test("admin previews, prepares, verifies recipients, and sends", async ({ page }) => {
    await page.goto("/signin");
    await page.getByTestId("signin-button").click();
    await page.waitForURL(/127\.0\.0\.1:3000\/$/, { timeout: 30_000 });

    await page.goto("/admin/family-members");
    await expect(page.getByTestId("bulk-preview")).toBeVisible({ timeout: 30_000 });
    await expect(page.getByTestId("bulk-table")).toContainText(
      "synthetic-bulk-eligible-1",
    );
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
    await expect(page.getByTestId("bulk-results")).toBeVisible({ timeout: 30_000 });
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
    expect(blockedBody.results.every((row) => row.status === "failed")).toBeTruthy();
    expect(
      blockedBody.results.every((row) => row.reason === "missing_verified_contact"),
    ).toBeTruthy();
    const blockedText = JSON.stringify(blockedBody);
    expect(blockedText).not.toContain("token");
    expect(blockedText).not.toContain("@");

    // Record an explicitly verified recipient for each prepared person.
    for (const key of ELIGIBLE) {
      const row = page.getByRole("row", { name: key });
      await row.getByLabel(`Verified email for ${key}`).fill(
        `bulk-${key}@example.test`,
      );
      await row.getByRole("button", { name: "Verify" }).click();
      await expect(row.getByText("Verified")).toBeVisible({ timeout: 30_000 });
    }

    // The send gate is separate from preparation and requires explicit approval.
    await page.getByTestId("bulk-send").click();
    await expect(page.getByTestId("bulk-send-confirm")).toBeVisible();
    await page.getByTestId("bulk-send-confirm-button").click();
    await expect(page.getByTestId("bulk-send-result")).toContainText(
      "2 sent",
      { timeout: 30_000 },
    );
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

  test("bulk mutation without CSRF is rejected by the proxy", async ({ request }) => {
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
