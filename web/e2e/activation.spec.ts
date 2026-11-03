import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// PIR-006: the invitation welcome and activation screen.
//
// Every request is stubbed: the suite proves the welcome, activation, and
// failure states of this page, and that the invitation token is never rendered.
// Synthetic names and tokens are used; no real invitation exists.

const TOKEN =
  "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const AUTH_PATH = `/activate/${TOKEN}`;

const SIGNED_IN = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic Member",
};
const ANONYMOUS = { state: "anonymous" };

function json(status: number, body: unknown) {
  return {
    status,
    contentType: "application/json",
    body: JSON.stringify(body),
  };
}

async function stubSession(page: Page, session: unknown) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill(json(200, session)),
  );
}

type AcceptResult = { status: number; body: unknown };

async function stubInvitation(
  page: Page,
  preview: { status: number; body: unknown },
  accept?: AcceptResult,
) {
  const calls: string[] = [];
  await page.route("**/api/invitations/**", async (route) => {
    const path = new URL(route.request().url()).pathname;
    calls.push(path);
    if (path.endsWith("/accept")) {
      await route.fulfill(
        json(
          accept?.status ?? 200,
          accept?.body ?? {
            ok: true,
            person_key: "synthetic-person",
            display_name: "Synthetic Person",
            state: "accepted",
            link_state: "pending_review",
          },
        ),
      );
      return;
    }
    if (path.endsWith("/provision")) {
      await route.fulfill(json(200, { ok: true, provisioned: true }));
      return;
    }
    await route.fulfill(json(preview.status, preview.body));
  });
  return calls;
}

function pendingPreview(overrides: Record<string, unknown> = {}) {
  return {
    status: 200,
    body: {
      ok: true,
      person_key: "synthetic-person",
      display_name: "Synthetic Person",
      state: "pending",
      person_linked: false,
      request_state: "none",
      ...overrides,
    },
  };
}

test.describe("invitation activation", () => {
  test("an anonymous visitor continues through the identity provider", async ({
    page,
  }) => {
    await stubSession(page, ANONYMOUS);
    await stubInvitation(page, pendingPreview());

    await page.goto(AUTH_PATH);

    await expect(page.getByTestId("activation-welcome")).toBeVisible();
    await expect(page.getByTestId("activation-person")).toContainText(
      "Synthetic Person",
    );
    await expect(page.getByTestId("activation-accept")).toHaveCount(0);
    const signIn = page.getByTestId("activation-signin");
    await expect(signIn).toBeVisible();
    await expect(signIn).toHaveAttribute(
      "href",
      `/api/auth/login?returnTo=${encodeURIComponent(AUTH_PATH)}`,
    );
    await expect(page.getByTestId("activation-not-me")).toContainText(
      "This is not you?",
    );
    // The bearer token never reaches the rendered page.
    await expect(page.getByText(TOKEN)).toHaveCount(0);
  });

  test("a signed-in invitee accepts and waits for the reviewed link", async ({
    page,
  }) => {
    await stubSession(page, SIGNED_IN);
    const calls = await stubInvitation(page, pendingPreview());

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-signin")).toHaveCount(0);
    await page.getByTestId("activation-accept").click();

    // Token possession records a request; it never approves the link itself.
    await expect(page.getByTestId("activation-pending-review")).toBeVisible();
    await expect(page.getByTestId("activation-pending-review")).toContainText(
      "Synthetic Person",
    );
    // Access follows the administrator's review, not the acceptance alone.
    await expect(page.getByTestId("activation-pending-review")).toContainText(
      "before archive access opens",
    );
    await expect(page.getByTestId("activation-open-profile")).toHaveCount(0);
    expect(calls.some((path) => path.endsWith("/accept"))).toBe(true);
  });

  test("an administrator-approved link completes the welcome", async ({
    page,
  }) => {
    await stubSession(page, SIGNED_IN);
    await stubInvitation(page, pendingPreview(), {
      status: 200,
      body: {
        ok: true,
        person_key: "synthetic-person",
        display_name: "Synthetic Person",
        state: "accepted",
        link_state: "linked",
      },
    });

    await page.goto(AUTH_PATH);
    await page.getByTestId("activation-accept").click();

    await expect(page.getByTestId("activation-accepted")).toBeVisible();
    await expect(page.getByTestId("activation-accepted")).toContainText(
      "Synthetic Person",
    );
    await expect(page.getByTestId("activation-accepted")).toContainText(
      "Member access",
    );
    await expect(page.getByTestId("activation-open-profile")).toHaveAttribute(
      "href",
      "/family/synthetic-person",
    );
  });

  test("a returning invitee sees the recorded review state", async ({
    page,
  }) => {
    await stubSession(page, ANONYMOUS);
    await stubInvitation(
      page,
      pendingPreview({ state: "accepted", request_state: "pending" }),
    );

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-pending-review")).toBeVisible();
  });

  test("an expired invitation explains the next step", async ({ page }) => {
    await stubSession(page, ANONYMOUS);
    await stubInvitation(page, pendingPreview({ state: "expired" }));

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-problem")).toContainText(
      "This invitation has expired",
    );
    await expect(page.getByTestId("activation-welcome")).toHaveCount(0);
  });

  test("a revoked invitation explains the next step", async ({ page }) => {
    await stubSession(page, ANONYMOUS);
    await stubInvitation(page, pendingPreview({ state: "revoked" }));

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-problem")).toContainText(
      "This invitation was cancelled",
    );
  });

  test("an accepted invitation cannot be used again", async ({ page }) => {
    await stubSession(page, ANONYMOUS);
    await stubInvitation(page, pendingPreview({ state: "accepted" }));

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-problem")).toContainText(
      "already used",
    );
  });

  test("an invalid link is reported without probing a person", async ({
    page,
  }) => {
    await stubSession(page, ANONYMOUS);
    await stubInvitation(page, {
      status: 404,
      body: {
        ok: false,
        error: "invalid_invitation",
        message: "this invitation link is not valid",
      },
    });

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-problem")).toContainText(
      "This invitation link is not valid",
    );
    await expect(page.getByText("invalid_invitation")).toHaveCount(0);
  });

  test("a person who is already linked shows the existing-account state", async ({
    page,
  }) => {
    await stubSession(page, SIGNED_IN);
    await stubInvitation(page, pendingPreview({ person_linked: true }));

    await page.goto(AUTH_PATH);
    await expect(page.getByTestId("activation-problem")).toContainText(
      "This person already has an account",
    );
    await expect(page.getByTestId("activation-accept")).toHaveCount(0);
  });

  test("an account linked to another person cannot activate this link", async ({
    page,
  }) => {
    await stubSession(page, SIGNED_IN);
    await stubInvitation(page, pendingPreview(), {
      status: 409,
      body: {
        ok: false,
        error: "wrong_person",
        message: "your account is linked to a different person",
      },
    });

    await page.goto(AUTH_PATH);
    await page.getByTestId("activation-accept").click();
    await expect(page.getByTestId("activation-problem")).toContainText(
      "This invitation is for a different person",
    );
    await expect(page.getByText("wrong_person")).toHaveCount(0);
  });

  test("an unprovisioned account waits for review", async ({ page }) => {
    await stubSession(page, SIGNED_IN);
    await stubInvitation(page, pendingPreview(), {
      status: 403,
      body: {
        ok: false,
        error: "account_not_provisioned",
        message: "the verified identity has no active account",
      },
    });

    await page.goto(AUTH_PATH);
    await page.getByTestId("activation-accept").click();
    await expect(page.getByTestId("activation-problem")).toContainText(
      "Pending review",
    );
    await expect(page.getByText("account_not_provisioned")).toHaveCount(0);
  });
});
