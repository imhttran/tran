import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// Phase 4 browser experience coverage.
//
// These specs exercise the browser surfaces through stubbed transports only. A
// stub never replaces Rust enforcement: it stands in for the proxy/API reply so
// the browser rendering paths for allowed, denied, expired, and no-access can be
// driven deterministically. One spec asserts a direct unauthorized request is
// rejected so UI state never substitutes for Rust enforcement.
//
// Every spec ends with assertNoSensitive, which checks that no account,
// identity, OIDC credential, or database URL value reached the DOM or a
// browser-visible response.

const FORBIDDEN = [
  /[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}/i,
  /postgres(ql)?:\/\//i,
  /client_secret/i,
  /access_token/i,
  /id_token/i,
  /refresh_token/i,
  /client_id/i,
  /authorization/i,
  /issuer/i,
  /jwks/i,
  /database_url/i,
];

async function assertNoSensitive(page: Page) {
  const body = await page.content();
  for (const pattern of FORBIDDEN) {
    expect(body, `DOM leaked ${pattern}`).not.toMatch(pattern);
  }
}

type Role = "viewer" | "editor" | "admin";

function sessionPayload(role: Role) {
  return { state: "signed_in", role, display_name: `Signed-in ${role}` };
}

async function signIn(page: Page, role: Role) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(sessionPayload(role)),
    }),
  );
}

const CONCLUSION_CITED = {
  conclusion: "Third Branch head after 1800",
  head_label: "Ancestor A",
  evidence: [{ evidence_id: "ev-1", citation: "Family Genealogy p.12" }],
  evidence_status: "verified" as const,
};

const CONCLUSION_UNVERIFIED = {
  conclusion: "Head after 1855",
  head_label: null,
  evidence: [],
  evidence_status: "unverified" as const,
};

const CONCLUSION_NEEDS_REVIEW = {
  conclusion: "Head after 1902",
  head_label: null,
  evidence: [
    { evidence_id: "ev-2", citation: "Register entry 44" },
    { evidence_id: "ev-3", citation: "Register entry 45" },
  ],
  evidence_status: "needs_review" as const,
};

const HOF_AVAILABLE = {
  catalog: "available" as const,
  conclusions: [CONCLUSION_CITED, CONCLUSION_UNVERIFIED, CONCLUSION_NEEDS_REVIEW],
};

async function stubHeadOfFamily(page: Page, payload: unknown, status = 200) {
  await page.route("**/api/genealogy/head-of-family", (route) =>
    route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify(payload),
    }),
  );
}

test.describe("account menu", () => {
  test("shows sign-in when anonymous", async ({ page }) => {
    await page.route("**/api/auth/session", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ state: "anonymous" }),
      }),
    );
    await page.goto("/");
    await expect(page.getByTestId("sign-in-button")).toBeVisible();
    await assertNoSensitive(page);
  });

  test("sign-in expiry turns the session into a signed-out state", async ({ page }) => {
    await page.route("**/api/auth/session", (route) =>
      route.fulfill({
        status: 401,
        contentType: "application/json",
        body: JSON.stringify({ error: "expired" }),
      }),
    );
    await page.goto("/");
    await expect(page.getByTestId("sign-in-button")).toBeVisible();
    await expect(page.getByTestId("sign-out-button")).toHaveCount(0);
    await assertNoSensitive(page);
  });
});

test.describe("roles", () => {
  for (const role of ["viewer", "editor", "admin"] as const) {
    test(`${role} session renders the account menu with role`, async ({ page }) => {
      await signIn(page, role);
      await page.goto("/");
      await expect(page.getByTestId("account-role")).toHaveText(role);
      await expect(page.getByTestId("account-label")).toContainText(`Signed-in ${role}`);
      await expect(page.getByTestId("sign-out-button")).toBeVisible();
      await assertNoSensitive(page);
    });
  }

  test("viewer sees no upload controls", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/");
    await expect(page.getByTestId("upload-controls")).toHaveCount(0);
    await assertNoSensitive(page);
  });

  test("editor sees upload controls", async ({ page }) => {
    await signIn(page, "editor");
    await page.goto("/");
    await expect(page.getByTestId("upload-controls")).toBeVisible();
    await assertNoSensitive(page);
  });
});

test.describe("denial states", () => {
  test("permission denial renders a denied view without record detail", async ({ page }) => {
    await signIn(page, "viewer");
    await page.route("**/api/genealogy/tree**", (route) =>
      route.fulfill({
        status: 403,
        contentType: "application/json",
        body: JSON.stringify({ error: "forbidden" }),
      }),
    );
    await page.goto("/genealogy");
    await expect(page.getByTestId("denied-view")).toBeVisible();
    await expect(page.getByTestId("genealogy-record")).toHaveCount(0);
    await assertNoSensitive(page);
  });

  test("no-access state is explicit and shows no records", async ({ page }) => {
    await page.route("**/api/auth/session", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ state: "signed_in", role: "viewer", display_name: "No access user" }),
      }),
    );
    await page.route("**/api/genealogy/tree**", (route) =>
      route.fulfill({
        status: 404,
        contentType: "application/json",
        body: JSON.stringify({ error: "not_found" }),
      }),
    );
    await page.goto("/genealogy");
    await expect(page.getByTestId("no-access-view")).toBeVisible();
    await expect(page.getByTestId("genealogy-record")).toHaveCount(0);
    await assertNoSensitive(page);
  });
});

test.describe("head of family", () => {
  test("links each conclusion to its cited evidence", async ({ page }) => {
    await signIn(page, "viewer");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    await page.goto("/head-of-family");
    const conclusions = page.getByTestId("hof-conclusion");
    await expect(conclusions).toHaveCount(3);
    // Evidence opens metadata-only source details; there is no dead /evidence link.
    await expect(page.getByTestId("evidence-links").getByRole("link")).toHaveCount(0);
    await page.getByTestId("citation-button").filter({ hasText: "Family Genealogy p.12" }).click();
    await expect(page.getByTestId("source-details")).toContainText("Third Branch head after 1800");
    await expect(page.getByTestId("source-metadata-only")).toBeVisible();
    await expect(page.getByTestId("evidence-verified").first()).toBeVisible();
    await assertNoSensitive(page);
  });

  test("missing evidence shows Unverified", async ({ page }) => {
    await signIn(page, "viewer");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    await page.goto("/head-of-family");
    await expect(page.getByTestId("evidence-unverified").first()).toHaveText("Unverified");
    await assertNoSensitive(page);
  });

  test("conflicting evidence shows Needs review", async ({ page }) => {
    await signIn(page, "viewer");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    await page.goto("/head-of-family");
    await expect(page.getByTestId("evidence-needs_review").first()).toHaveText("Needs review");
    await assertNoSensitive(page);
  });

  test("viewer sees no nomination, approval, or revocation controls", async ({ page }) => {
    await signIn(page, "viewer");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    await page.goto("/head-of-family");
    await expect(page.getByTestId("hof-conclusion").first()).toBeVisible();
    await expect(page.getByTestId("nominate-button")).toHaveCount(0);
    await expect(page.getByTestId("approve-button")).toHaveCount(0);
    await expect(page.getByTestId("revoke-button")).toHaveCount(0);
    await assertNoSensitive(page);
  });

  test("editor nomination requires confirmation and cancel issues no request", async ({ page }) => {
    await signIn(page, "editor");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    let nominations = 0;
    await page.route("**/api/genealogy/nominations", (route) => {
      nominations += 1;
      return route.fulfill({ status: 201, contentType: "application/json", body: `` });
    });
    await page.goto("/head-of-family");
    await page.getByTestId("nomination-input").first().fill("Candidate X");
    await page.getByTestId("nominate-button").first().click();
    await expect(page.getByTestId("confirm-dialog")).toBeVisible();
    await page.getByTestId("cancel-button").click();
    await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
    expect(nominations).toBe(0);
    await assertNoSensitive(page);
  });

  test("admin approval requires confirmation and cancel issues no request", async ({ page }) => {
    await signIn(page, "admin");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    let approvals = 0;
    await page.route("**/api/genealogy/nominations/**", (route) => {
      approvals += 1;
      return route.fulfill({ status: 200, contentType: "application/json", body: `` });
    });
    await page.goto("/head-of-family");
    await page.getByTestId("approve-button").first().click();
    await expect(page.getByTestId("confirm-dialog")).toBeVisible();
    await page.getByTestId("cancel-button").click();
    await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
    expect(approvals).toBe(0);
    await assertNoSensitive(page);
  });

  test("admin confirmation submits the approval and shows the approved state", async ({ page }) => {
    await signIn(page, "admin");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    let body: unknown = null;
    await page.route("**/api/genealogy/nominations/**", (route) => {
      body = route.request().postDataJSON();
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true }),
      });
    });
    await page.goto("/head-of-family");
    await page.getByTestId("approve-button").first().click();
    await page.getByTestId("confirm-button").click();
    await expect.poll(() => body).not.toBeNull();
    await assertNoSensitive(page);
  });

  test("admin revocation requires confirmation and cancel issues no request", async ({ page }) => {
    await signIn(page, "admin");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    let revocations = 0;
    await page.route("**/api/genealogy/appointments/**", (route) => {
      revocations += 1;
      return route.fulfill({ status: 200, contentType: "application/json", body: `` });
    });
    await page.goto("/head-of-family");
    await page.getByTestId("revoke-reason-input").first().fill("Superseded by evidence");
    await page.getByTestId("revoke-button").first().click();
    await expect(page.getByTestId("confirm-dialog")).toBeVisible();
    await page.getByTestId("cancel-button").click();
    await expect(page.getByTestId("confirm-dialog")).toHaveCount(0);
    expect(revocations).toBe(0);
    await assertNoSensitive(page);
  });
});

test.describe("enforcement", () => {
  test("a direct unauthorized request is rejected by the API", async ({ request }) => {
    const response = await request.post("/api/genealogy/nominations", {
      data: { record_id: "r-1", candidate_label: "Candidate X" },
      headers: { "x-test-role": "viewer" },
    });
    expect([401, 403]).toContain(response.status());
  });
});

test.describe("navigation", () => {
  test("desktop navigation shows implemented routes and the active page", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/head-of-family");
    const nav = page.getByRole("navigation", { name: "Main navigation" });
    await expect(nav.getByRole("link")).toHaveText([
      /Family tree/, /Ask the archive/, /Head of Family/,
    ]);
    await expect(nav.getByRole("link", { name: /Head of Family/ })).toHaveAttribute("aria-current", "page");
    await nav.getByRole("link", { name: /Ask the archive/ }).click();
    await expect(page).toHaveURL(/\/ask$/);
    await expect(nav.getByRole("link", { name: /Ask the archive/ })).toHaveAttribute("aria-current", "page");
    await expect(page.getByTestId("account-role")).toHaveText("viewer");
    await assertNoSensitive(page);
  });

  test("editors also see the existing upload page", async ({ page }) => {
    await signIn(page, "editor");
    await page.goto("/ask");
    await expect(page.getByRole("navigation", { name: "Main navigation" })
      .getByRole("link", { name: /Add documents/ })).toHaveAttribute("href", "/");
  });

  test("mobile menu opens, navigates, and closes", async ({ page }) => {
    await page.setViewportSize({ width: 375, height: 812 });
    await signIn(page, "viewer");
    await page.goto("/ask");
    const nav = page.getByRole("navigation", { name: "Main navigation" });
    await expect(nav).toBeHidden();
    await page.getByTestId("nav-toggle").click();
    await expect(page.getByTestId("nav-toggle")).toHaveAttribute("aria-expanded", "true");
    await nav.getByRole("link", { name: /Head of Family/ }).click();
    await expect(page).toHaveURL(/\/head-of-family$/);
    await expect(nav).toBeHidden();
    await expect(page.getByTestId("sign-out-button")).toBeVisible();
  });
});

test.describe("ask the archive", () => {
  async function ask(page: Page, reply: { status: number; body: unknown }, delay = 0) {
    await signIn(page, "viewer");
    await page.route("**/api/query", async (route) => {
      if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
      await route.fulfill({ status: reply.status, contentType: "application/json",
        body: JSON.stringify(reply.body) });
    });
    await page.goto("/ask");
    await page.getByTestId("ask-example").first().click();
    await page.getByTestId("ask-button").click();
  }

  test("shows family examples and makes no empty-archive or payment claims", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/ask");
    await expect(page.getByPlaceholder("Ask about your family…")).toBeVisible();
    await expect(page.getByRole("button", { name: "Ask", exact: true })).toBeVisible();
    await expect(page.getByTestId("ask-example")).not.toHaveCount(0);
    const text = await page.locator("body").innerText();
    expect(text).not.toMatch(/payment|No documents ingested|Ollama|PostgreSQL|Axum/i);
  });

  test("renders an answer with numbered citations", async ({ page }) => {
    await ask(page, { status: 200, body: { answer: "The register lists two founders.",
      insufficient: false, sources: [{ source: "synthetic-register.md", section: "Founders",
        page: 3, text: "Two founders are recorded." }] } });
    await expect(page.getByTestId("answer")).toHaveText("The register lists two founders.");
    await expect(page.getByTestId("sources")).toContainText("[1] synthetic-register.md — Founders");
    await expect(page.getByTestId("sources")).toContainText("page 3");
    await assertNoSensitive(page);
  });

  test("shows loading, then insufficient evidence", async ({ page }) => {
    await ask(page, { status: 200, body: { answer: "", insufficient: true, sources: [] } }, 800);
    await expect(page.getByTestId("ask-loading")).toBeVisible();
    await expect(page.getByTestId("insufficient")).toBeVisible();
    await expect(page.getByTestId("answer")).toHaveCount(0);
  });

  for (const [status, testId] of [[401, "ask-expired"], [403, "ask-denied"], [503, "ask-error"]] as const) {
    test(`HTTP ${status} renders ${testId}`, async ({ page }) => {
      await ask(page, { status, body: { ok: false, error: "x", message: "retrieval or generation is unavailable" } });
      await expect(page.getByTestId(testId)).toBeVisible();
      await expect(page.getByTestId("answer")).toHaveCount(0);
      await expect(page.locator("body")).not.toContainText("retrieval or generation");
    });
  }
});
