import { expect, test } from "@playwright/test";

// PIR-012: the link the delivery builder produces must open the welcome page.
//
// The builder emits `{ONBOARDING_ACTIVATION_URL}/{token}` (trailing slashes
// trimmed) so the token is a path segment of `/activate/[token]`. This spec
// mirrors that exact shape against the running web app and checks that the
// intended person is shown and the token never appears in the page text.

const BASE = process.env.E2E_ACTIVATION_BASE ?? "";
const TOKEN = process.env.E2E_LINK_INVITATION_TOKEN ?? "";

test.describe("generated activation link", () => {
  test.skip(!BASE || !TOKEN, "activation link fixture not provided by the harness");

  test("opens the intended activation page with a path token", async ({ page }) => {
    // Same construction as the server: trim trailing slashes, then one segment.
    const url = `${BASE.trim().replace(/\/+$/, "")}/${encodeURIComponent(TOKEN)}`;
    expect(new URL(url).search).toBe("");

    const response = await page.goto(url);
    expect(response?.status()).toBe(200);

    await expect(page.getByTestId("activation-welcome")).toBeVisible();
    await expect(page.getByTestId("activation-person")).toContainText(
      "Synthetic Link Invitee Person",
    );

    const parsed = new URL(page.url());
    expect(parsed.pathname).toBe(`/activate/${TOKEN}`);
    expect(parsed.search).toBe("");
    // The bearer token is never rendered.
    await expect(page.getByText(TOKEN)).toHaveCount(0);
  });
});
