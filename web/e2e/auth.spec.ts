import { test, expect } from "@playwright/test";

// Real authorization-code + PKCE sign-in against the synthetic OIDC provider.
// The provider auto-approves the synthetic admin; Rust still verifies the RS256
// token, and the database resolves the admin role and corpus grant.

test("OIDC sign-in and sign-out establish and clear the session", async ({ page }) => {
  await page.goto("/signin");
  await expect(page.getByTestId("signin-button")).toBeVisible();
  await page.getByTestId("signin-button").click();

  await page.waitForURL(
    /http:\/\/(127\.0\.0\.1|localhost):3000\/$/,
    { timeout: 30_000 },
  );
  await expect(page.getByTestId("account-menu")).toHaveAttribute(
    "data-state",
    "signed-in",
  );
  await expect(page.getByTestId("account-role")).toHaveText("admin");

  await page.getByTestId("sign-out-button").click();
  await expect(page.getByTestId("account-menu")).toHaveAttribute(
    "data-state",
    "signed-out",
  );
});

test("a tampered callback state is rejected", async ({ page, context }) => {
  await context.addCookies([
    {
      name: "rag_oidc_state",
      value: "expected-state",
      url: "http://127.0.0.1:3000/api/auth",
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_oidc_nonce",
      value: "expected-nonce",
      url: "http://127.0.0.1:3000/api/auth",
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_oidc_verifier",
      value: "expected-verifier",
      url: "http://127.0.0.1:3000/api/auth",
      httpOnly: true,
      sameSite: "Lax",
    },
  ]);
  await page.goto("/api/auth/callback?code=synthetic&state=wrong-state");
  await expect(page).toHaveURL(/\/signin\?error=state_mismatch/);
});
