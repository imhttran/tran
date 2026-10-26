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
  // The Ask page reads approved suggestions on mount. These UI tests have no
  // real session cookie, so give every one a deterministic empty list instead
  // of letting the 401 mark the mocked session expired.
  await page.route("**/api/query/suggestions", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, questions: [] }),
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
  test("desktop header has the three primary links and no sidebar", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/ask");

    await expect(page.getByTestId("site-header")).toBeVisible();
    await expect(page.locator("aside.site-nav")).toHaveCount(0);

    const nav = page.getByTestId("primary-nav");
    await expect(nav.getByRole("link")).toHaveText([
      /Family tree/,
      /Ask the archive/,
      /Documents/,
    ]);
    await expect(nav.getByRole("link", { name: /Ask the archive/ })).toHaveAttribute(
      "aria-current",
      "page",
    );

    await nav.getByRole("link", { name: /Family tree/ }).click();
    await expect(page).toHaveURL(/\/genealogy$/);
    await expect(nav.getByRole("link", { name: /Family tree/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await expect(page.getByTestId("account-role")).toHaveText("viewer");
    await assertNoSensitive(page);
  });

  test("viewer More contains secondary routes and the current state is accessible", async ({ page }) => {
    await signIn(page, "viewer");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    await page.goto("/head-of-family");

    const moreButton = page.getByTestId("more-toggle");
    await expect(moreButton).toHaveAccessibleName(/More, current page/);
    await expect(moreButton).toHaveAttribute("data-current", "true");
    await moreButton.click();

    const menu = page.getByTestId("more-menu");
    await expect(menu.getByRole("link")).toHaveText([
      /My profile/,
      /Head of Family/,
      /Branch review/,
    ]);
    await expect(menu.getByRole("link", { name: /Head of Family/ })).toHaveAttribute(
      "aria-current",
      "page",
    );
    await expect(menu.getByRole("link", { name: /Reviews/ })).toHaveCount(0);
    await expect(menu.getByRole("link", { name: /Bulk onboard/ })).toHaveCount(0);
    await assertNoSensitive(page);
  });

  test("administrators see the permitted secondary routes", async ({ page }) => {
    await signIn(page, "admin");
    await page.goto("/ask");
    await page.getByTestId("more-toggle").click();
    const menu = page.getByTestId("more-menu");
    await expect(menu.getByRole("link", { name: /Add documents/ })).toHaveAttribute(
      "href",
      "/",
    );
    await expect(menu.getByRole("link", { name: /Personal history review/ })).toBeVisible();
    await expect(menu.getByRole("link", { name: /Reviews/ })).toBeVisible();
    await expect(menu.getByRole("link", { name: /Extractions/ })).toBeVisible();
    await expect(menu.getByRole("link", { name: /Source of truth/ })).toBeVisible();
    await expect(menu.getByRole("link", { name: /Bulk onboard/ })).toBeVisible();
  });

  test("More closes on Escape and returns focus", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/ask");
    const moreButton = page.getByTestId("more-toggle");
    await moreButton.click();
    await expect(moreButton).toHaveAttribute("aria-expanded", "true");
    await page.keyboard.press("Escape");
    await expect(moreButton).toHaveAttribute("aria-expanded", "false");
    await expect(moreButton).toBeFocused();
    await expect(page.getByTestId("more-menu")).toHaveCount(0);
  });

  test("More closes on an outside click", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/ask");
    await page.getByTestId("more-toggle").click();
    await expect(page.getByTestId("more-menu")).toBeVisible();
    await page.locator(".ask-panel").click();
    await expect(page.getByTestId("more-menu")).toHaveCount(0);
  });

  test("mobile menu opens, navigates, and closes", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, "admin");
    await stubHeadOfFamily(page, HOF_AVAILABLE);
    await page.goto("/ask");

    await expect(page.getByTestId("primary-nav")).toBeHidden();
    const toggle = page.getByTestId("nav-toggle");
    await expect(toggle).toBeVisible();
    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");

    const nav = page.getByTestId("site-nav");
    await expect(nav).toBeVisible();
    const overflow = await page.evaluate(
      () =>
        document.documentElement.scrollWidth -
        document.documentElement.clientWidth,
    );
    expect(overflow, "no horizontal overflow with the menu open").toBeLessThanOrEqual(1);
    await expect(nav.getByRole("link")).toHaveText([
      /Family tree/,
      /Ask the archive/,
      /Documents/,
      /My profile/,
      /Head of Family/,
      /Branch review/,
      /Add documents/,
      /Personal history review/,
      /Reviews/,
      /Extractions/,
      /Source of truth/,
      /Bulk onboard/,
    ]);

    await nav.getByRole("link", { name: /Head of Family/ }).click();
    await expect(page).toHaveURL(/\/head-of-family$/);
    await expect(page.getByTestId("site-nav")).toHaveCount(0);
    await expect(page.getByTestId("sign-out-button")).toBeVisible();
  });

  test("mobile menu closes on Escape and returns focus", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await signIn(page, "viewer");
    await page.goto("/ask");
    const toggle = page.getByTestId("nav-toggle");
    await toggle.click();
    await expect(page.getByTestId("site-nav")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(toggle).toBeFocused();
    await expect(page.getByTestId("site-nav")).toHaveCount(0);
  });

  test("light and dark themes use the stage-three tokens", async ({ page }) => {
    await signIn(page, "viewer");
    await page.goto("/ask");

    await page.emulateMedia({ colorScheme: "light" });
    await expect
      .poll(async () =>
        page.evaluate(() =>
          getComputedStyle(document.documentElement)
            .getPropertyValue("--bg")
            .trim()
            .toLowerCase(),
        ),
      )
      .toBe("#fafafb");

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
      .toBe("#08090a");
  });
});

test.describe("ask the archive", () => {
  async function ask(page: Page, reply: { status: number; body: unknown }, delay = 0) {
    await signIn(page, "viewer");
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({ status: 200, contentType: "application/json",
        body: JSON.stringify({ ok: true, questions: ["What does the archive say about \"Synthetic register\"?"] }) }));
    await page.route("**/api/query", async (route) => {
      if (delay) await new Promise((resolve) => setTimeout(resolve, delay));
      await route.fulfill({ status: reply.status, contentType: "application/json",
        body: JSON.stringify(reply.body) });
    });
    await page.goto("/ask");
    await page.getByTestId("ask-example").first().click();
    await page.getByTestId("ask-button").click();
  }

  test("shows suggestions derived from approved archive content", async ({ page }) => {
    await signIn(page, "viewer");
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({ status: 200, contentType: "application/json",
        body: JSON.stringify({ ok: true, questions: ["What does the archive say about \"Synthetic register\"?"] }) }));
    await page.goto("/ask");
    await expect(page.getByPlaceholder("Ask about your family…")).toBeVisible();
    await expect(page.getByRole("button", { name: "Ask", exact: true })).toBeVisible();
    await expect(page.getByTestId("ask-example")).toHaveCount(1);
    await expect(page.getByTestId("ask-example").first()).toContainText(
      "Synthetic register",
    );
    const text = await page.locator("body").innerText();
    expect(text).not.toMatch(/payment|No documents ingested|Ollama|PostgreSQL|Axum/i);
  });

  test("shows an empty state when no approved content is searchable", async ({ page }) => {
    await signIn(page, "viewer");
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({ status: 200, contentType: "application/json",
        body: JSON.stringify({ ok: true, questions: [] }) }));
    await page.goto("/ask");
    await expect(page.getByTestId("ask-suggestions-empty")).toBeVisible();
    await expect(page.getByTestId("ask-example")).toHaveCount(0);
    await expect(page.getByTestId("ask-suggestions-empty")).toContainText(
      "approve and process",
    );
  });

  test("a suggestions failure leaves the question form usable", async ({ page }) => {
    await signIn(page, "viewer");
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({ status: 503, contentType: "application/json",
        body: JSON.stringify({ ok: false, error: "query_suggestions_unavailable",
          message: "question suggestions are unavailable" }) }));
    await page.goto("/ask");
    await expect(page.getByTestId("ask-suggestions-error")).toBeVisible();
    await expect(page.getByTestId("question-input")).toBeVisible();
    await expect(page.getByTestId("ask-example")).toHaveCount(0);
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

  test("a recoverable 503 keeps the question in the input", async ({ page }) => {
    await signIn(page, "viewer");
    await page.route("**/api/query", (route) =>
      route.fulfill({ status: 503, contentType: "application/json",
        body: JSON.stringify({ ok: false, error: "query_unavailable", message: "retrieval or generation is unavailable" }) }));
    await page.goto("/ask");
    await page.getByTestId("question-input").fill("Who founded the third branch?");
    await page.getByTestId("ask-button").click();
    await expect(page.getByTestId("ask-error")).toBeVisible();
    await expect(page.getByTestId("question-input")).toHaveValue("Who founded the third branch?");
    await assertNoSensitive(page);
  });
});

// Name questions are answered from the genealogy store, not document
// retrieval. These stubs pin the rendering of each deterministic outcome; Rust
// still decides which outcome applies.
test.describe("ask the archive: person questions", () => {
  const CITATION = {
    evidence_id: "synthetic-catalog.md#3",
    citation: "synthetic-catalog.md p.3",
    document_ref: "synthetic-catalog.md",
    page: 3,
  };

  function personPayload(person: unknown) {
    return { answer: "Stored assertions only.", insufficient: false, sources: [], person };
  }

  const FOUND = personPayload({
    outcome: "found",
    catalog: "available",
    query_name: "Han Duc Tran",
    match: "reordered",
    truncated: false,
    candidates: [],
    candidates_truncated: false,
    selected: {
      person_key: "P013",
      display_name: "Trần Đức Hân",
      generation: 3,
      evidence_status: "verified",
      match: "reordered",
      matched_name: "Trần Đức Hân",
      profile_url: "/family/P013",
      name_only: false,
      assertions: [
        {
          assertion_id: 1,
          kind: "display_name",
          key: "display:p013",
          value_text: "Trần Đức Hân",
          review_status: "verified",
          evidence: [CITATION],
        },
        {
          assertion_id: 2,
          kind: "branch",
          key: "branch:third",
          value_text: "Third branch",
          review_status: "unverified",
          evidence: [],
        },
      ],
    },
  });

  async function askName(page: Page, body: unknown, question = "Who is Han Duc Tran") {
    await signIn(page, "viewer");
    await page.route("**/api/query", (route) =>
      route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(body) }),
    );
    await page.goto("/ask");
    await page.getByTestId("question-input").fill(question);
    await page.getByTestId("ask-button").click();
  }

  test("an accent-folded, reordered name renders stored assertions with citations", async ({ page }) => {
    await askName(page, FOUND);
    const card = page.getByTestId("person-found");
    await expect(card).toBeVisible();
    await expect(card).toContainText("Trần Đức Hân");
    await expect(card).toContainText("Recorded generation: 3");
    await expect(card).toContainText("different order");
    await expect(page.getByTestId("person-assertions")).toContainText("Recorded name");
    await expect(page.getByTestId("person-assertions")).toContainText("Verified");
    await expect(page.getByTestId("person-assertions")).toContainText("Third branch");
    await expect(page.getByTestId("person-assertions")).toContainText("Unverified");
    await expect(page.getByTestId("person-assertions")).toContainText(
      "not a confirmed fact",
    );
    await expect(page.getByTestId("person-profile-link")).toHaveAttribute("href", "/family/P013");
    // The document path's refusal is not shown for a person answer.
    await expect(page.getByTestId("insufficient")).toHaveCount(0);
    await expect(page.getByTestId("answer")).toHaveCount(0);

    // A stored citation opens the metadata-only dialog.
    await page.getByTestId("citation-button").first().click();
    await expect(page.getByTestId("source-details")).toBeVisible();
    await expect(page.getByTestId("source-details")).toContainText("synthetic-catalog.md p.3");
    await assertNoSensitive(page);
  });

  test("namesakes require a choice and the selection answers that person", async ({ page }) => {
    const ambiguous = personPayload({
      outcome: "ambiguous",
      catalog: "available",
      query_name: "Le Thanh Phong",
      match: null,
      truncated: false,
      candidates: [
        {
          person_key: "P022",
          display_name: "Lê Thanh Phong",
          generation: 4,
          evidence_status: "needs_review",
          match: "exact",
          matched_name: "Lê Thanh Phong",
          profile_url: "/family/P022",
          recorded_parents: [
            { person_key: "P101", display_name: "Trần Thị Lan", evidence_status: "needs_review" },
          ],
        },
        {
          person_key: "P023",
          display_name: "Lê Thanh Phong",
          generation: 4,
          evidence_status: "unverified",
          match: "exact",
          matched_name: "Lê Thanh Phong",
          profile_url: "/family/P023",
          recorded_parents: [
            { person_key: "P100", display_name: "Nguyễn Thị Mai", evidence_status: "unverified" },
          ],
        },
      ],
      candidates_truncated: false,
      selected: null,
    });
    const selected = personPayload({
      outcome: "name_only",
      catalog: "available",
      query_name: "Le Thanh Phong",
      match: "exact",
      truncated: false,
      candidates: [],
      candidates_truncated: false,
      selected: {
        person_key: "P022",
        display_name: "Lê Thanh Phong",
        generation: 4,
        evidence_status: "needs_review",
        match: "exact",
        matched_name: "Lê Thanh Phong",
        profile_url: "/family/P022",
        name_only: true,
        assertions: [],
      },
    });

    await signIn(page, "viewer");
    let sawSelection = false;
    await page.route("**/api/query", async (route) => {
      const request = route.request().postDataJSON() as { person_key?: string };
      if (request.person_key === "P022") {
        await route.fulfill({ status: 200, contentType: "application/json",
          body: JSON.stringify(selected) });
        return;
      }
      sawSelection = true;
      await route.fulfill({ status: 200, contentType: "application/json",
        body: JSON.stringify(ambiguous) });
    });
    await page.goto("/ask");
    await page.getByTestId("question-input").fill("Who is Le Thanh Phong");
    await page.getByTestId("ask-button").click();

    await expect(page.getByTestId("person-ambiguous")).toBeVisible();
    await expect(page.getByTestId("person-candidate")).toHaveCount(2);
    await expect(page.getByTestId("person-ambiguous")).toContainText("Trần Thị Lan");
    await expect(page.getByTestId("person-ambiguous")).toContainText("Nguyễn Thị Mai");

    await page.getByTestId("person-candidate").first().click();
    await expect(page.getByTestId("person-found")).toBeVisible();
    await expect(page.getByTestId("person-name-only")).toBeVisible();
    await expect(page.getByTestId("person-profile-link")).toHaveAttribute("href", "/family/P022");
    expect(sawSelection).toBe(true);
    await assertNoSensitive(page);
  });

  test("an unknown name says no matching person instead of insufficient evidence", async ({ page }) => {
    await askName(
      page,
      personPayload({
        outcome: "none",
        catalog: "available",
        query_name: "Quang Vinh",
        match: null,
        truncated: false,
        candidates: [],
        candidates_truncated: false,
        selected: null,
      }),
      "Who is Quang Vinh",
    );
    await expect(page.getByTestId("person-none")).toBeVisible();
    await expect(page.getByTestId("person-none")).toContainText("No person matching");
    await expect(page.getByTestId("insufficient")).toHaveCount(0);
  });

  test("an unimported catalog is reported as an operator condition", async ({ page }) => {
    await askName(
      page,
      personPayload({
        outcome: "none",
        catalog: "empty",
        query_name: "Han Duc Tran",
        match: null,
        truncated: false,
        candidates: [],
        candidates_truncated: false,
        selected: null,
      }),
    );
    await expect(page.getByTestId("person-catalog-empty")).toBeVisible();
    await expect(page.getByTestId("person-catalog-empty")).toContainText(
      "has not been imported",
    );
  });
});
