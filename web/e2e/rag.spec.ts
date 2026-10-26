import { test, expect } from "@playwright/test";
import type { Page } from "@playwright/test";
import fs from "node:fs";
import path from "node:path";

// Real-browser verification for RUST-008A. The runner script starts the Axum
// API and the Next.js server; these tests drive the actual UI and real Rust
// services (no mock responses except the explicitly labeled insufficient-state
// UI check, whose real backend refusal is covered by the API contract crate).

const repo = path.resolve(process.cwd(), "..");
const screenshots = path.join(
  repo,
  "docs/reports/rust-migration/web-e2e",
);

test.beforeAll(() => {
  fs.mkdirSync(screenshots, { recursive: true });
});

test.beforeEach(async ({ context }) => {
  const token = process.env.E2E_SESSION_TOKEN;
  if (!token) {
    return;
  }
  const base = process.env.E2E_BASE_URL || "http://127.0.0.1:3000";
  await context.addCookies([
    {
      name: "rag_session",
      value: token,
      url: base,
      httpOnly: true,
      sameSite: "Strict",
    },
    {
      name: "rag_csrf",
      value: process.env.E2E_CSRF_TOKEN || "e2e-csrf-token",
      url: base,
      sameSite: "Strict",
    },
  ]);
});

// A local model can take minutes per answer. Wait for the real /api/query
// response itself instead of a fixed UI timeout, so a slow answer is never
// mistaken for a wrong one and a retry never clicks a still-disabled button.
const ANSWER_TIMEOUT = 5 * 60_000;
const WEB_URL = (process.env.E2E_BASE_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
const ADMIN_TOKEN = process.env.E2E_SESSION_TOKEN || process.env.E2E_ADMIN_TOKEN || "";
const CSRF = process.env.E2E_CSRF_TOKEN || "e2e-csrf-token";

function adminHeaders() {
  return {
    authorization: `Bearer ${ADMIN_TOKEN}`,
    "content-type": "application/json",
    "x-csrf-token": CSRF,
    cookie: `rag_csrf=${CSRF}`,
  };
}

async function uploadForReview(page: Page, file: string): Promise<number> {
  await page.goto("/");
  await page
    .getByTestId("file-input")
    .setInputFiles(path.join(repo, file));
  const [response] = await Promise.all([
    page.waitForResponse(
      (r) =>
        r.url().endsWith("/api/documents/submissions") &&
        r.request().method() === "POST",
    ),
    page.getByTestId("ingest-button").click(),
  ]);
  expect(response.status()).toBe(200);
  const body = (await response.json()) as {
    version_id: number;
    review_status: string;
  };
  expect(body.review_status).toBe("pending_review");
  await expect(page.getByTestId("ingest-status")).toContainText("pending_review");
  return body.version_id;
}

async function publishForRetrieval(page: Page, versionId: number) {
  const review = await page.request.post(
    `${WEB_URL}/api/documents/submissions/${versionId}/review`,
    {
      headers: adminHeaders(),
      data: JSON.stringify({ approve: true, scope: "family" }),
    },
  );
  expect(review.status()).toBe(200);
  const process = await page.request.post(
    `${WEB_URL}/api/documents/submissions/${versionId}/process`,
    { headers: adminHeaders(), data: JSON.stringify({}) },
  );
  expect(process.status()).toBe(200);
  const run = await page.request.post(`${WEB_URL}/api/documents/jobs/run`, {
    headers: adminHeaders(),
    data: JSON.stringify({}),
  });
  expect(run.status()).toBe(200);
  const body = (await run.json()) as {
    processed: Array<{ version_id: number; status: string; error: string | null }>;
  };
  return body.processed.find((entry) => entry.version_id === versionId);
}

async function ask(page: Page) {
  const [response] = await Promise.all([
    page.waitForResponse((r) => r.url().endsWith("/api/query"), { timeout: ANSWER_TIMEOUT }),
    page.getByTestId("ask-button").click(),
  ]);
  expect(response.status()).toBe(200);
}

// Live generation is stochastic; allow a bounded retry for a grounded, cited
// answer before failing the browser check.
async function askUntilCited(page: Page, answer: RegExp, source: RegExp) {
  for (let attempt = 0; attempt < 3; attempt++) {
    await ask(page);
    try {
      await expect(page.getByTestId("answer")).toContainText(answer, { timeout: 5_000 });
      await expect(page.getByTestId("sources")).toContainText(source, { timeout: 5_000 });
      return;
    } catch {
      // Retry the question; the next generation may cite the expected source.
    }
  }
  throw new Error(`no answer matching ${answer} cited ${source} after 3 attempts`);
}

test.describe.serial("RUST-008A browser integration", () => {
  test("upload PDF, verify page; upload Markdown, verify cited answer", async ({
    page,
  }) => {
    test.setTimeout(6 * ANSWER_TIMEOUT); // Up to three attempts for each of two questions.
    const requests: string[] = [];
    page.on("request", (request) => requests.push(request.url()));

    await page.goto("/");
    await expect(page.getByTestId("empty-state")).toHaveCount(0);
    await expect(page.getByLabel("System status")).toContainText("up");

    // PDF submission -> explicit approval -> durable processing -> retrieval.
    const pdfVersion = await uploadForReview(
      page,
      "fixtures/parity/inputs/text-layer.pdf",
    );
    const pdfOutcome = await publishForRetrieval(page, pdfVersion);
    expect(pdfOutcome?.status).toBe("ready");

    // Questions are asked on the family-facing /ask page.
    await page.goto("/ask");
    await page
      .getByTestId("question-input")
      .fill("What text appears on page two of the PDF document?");
    await askUntilCited(page, /page two|page 2/i, /page 2/i);
    await page.screenshot({
      path: path.join(screenshots, "pdf-page-answer.png"),
      fullPage: true,
    });

    // Markdown submission -> approval -> processing -> cited answer.
    const markdownVersion = await uploadForReview(page, "examples/loan-policy.md");
    const markdownOutcome = await publishForRetrieval(page, markdownVersion);
    expect(markdownOutcome?.status).toBe("ready");

    await page.goto("/ask");
    await page
      .getByTestId("question-input")
      .fill("What happens if someone misses a payment?");
    await askUntilCited(page, /grace period/, /loan-policy\.md/);
    await page.screenshot({
      path: path.join(screenshots, "markdown-answer.png"),
      fullPage: true,
    });

    // The browser only calls same-origin Next.js `/api/*`; it never reaches the
    // Axum port directly.
    const apiRequests = requests.filter((url) => url.includes("/api/"));
    expect(apiRequests.length).toBeGreaterThan(0);
    for (const url of apiRequests) {
      expect(url.startsWith("http://127.0.0.1:3000/api/")).toBeTruthy();
    }
    expect(requests.some((url) => url.includes("127.0.0.1:8080"))).toBeFalsy();
  });

  test("loading indicator while a question is pending", async ({ page }) => {
    await page.goto("/ask");
    await page.route("**/api/query", async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 1500));
      await route.continue();
    });
    await page
      .getByTestId("question-input")
      .fill("What happens if someone misses a payment?");
    await page.getByTestId("ask-button").click();
    await expect(page.getByTestId("ask-loading")).toBeVisible();
    await page.screenshot({
      path: path.join(screenshots, "loading-state.png"),
      fullPage: true,
    });
  });

  test("connection failure is visible and actionable", async ({ page }) => {
    await page.route("**/api/health", (route) => route.abort());
    await page.goto("/");
    await expect(page.getByTestId("health-error")).toBeVisible();
    await expect(page.getByLabel("System status")).toContainText("down");
    await page.screenshot({
      path: path.join(screenshots, "connection-failure.png"),
      fullPage: true,
    });
  });

  test("incomplete PDF fails processing without publishing", async ({ page }) => {
    const version = await uploadForReview(
      page,
      "fixtures/parity/inputs/mixed-text-scan.pdf",
    );
    const outcome = await publishForRetrieval(page, version);
    expect(outcome?.status).toBe("failed");
    const submission = await page.request.get(
      `${WEB_URL}/api/documents/submissions/${version}`,
      { headers: adminHeaders() },
    );
    expect(submission.status()).toBe(200);
    const body = (await submission.json()) as {
      submission: { processing_status: string };
    };
    expect(body.submission.processing_status).toBe("failed");
  });

  test("no-context / insufficient-evidence UI state", async ({ page }) => {
    // The real backend refusal is covered by the API contract crate. Here the
    // UI state itself is verified by fulfilling the same contract shape.
    await page.goto("/ask");
    await page.route("**/api/query", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          answer: "I cannot answer this question from the provided documents.",
          insufficient: true,
          sources: [],
        }),
      }),
    );
    await page
      .getByTestId("question-input")
      .fill("What happens if someone misses a payment?");
    await page.getByTestId("ask-button").click();
    await expect(page.getByTestId("insufficient")).toBeVisible();
    await page.screenshot({
      path: path.join(screenshots, "insufficient-evidence.png"),
      fullPage: true,
    });
  });

  test("an approved family fixture supplies a suggested question and a cited answer", async ({
    page,
  }) => {
    test.setTimeout(8 * ANSWER_TIMEOUT);
    const version = await uploadForReview(
      page,
      "fixtures/family/ask-suggestions.md",
    );
    const outcome = await publishForRetrieval(page, version);
    expect(outcome?.status).toBe("ready");

    const response = await page.request.get(
      `${WEB_URL}/api/query/suggestions`,
      { headers: adminHeaders() },
    );
    expect(response.status()).toBe(200);
    const body = (await response.json()) as { questions: string[] };
    // A substantive heading is quoted as-is.
    expect(body.questions).toContain(
      'What does the archive say about "Archive Test Section"?',
    );
    // A page marker is never quoted; the passage supplies the topic instead.
    expect(body.questions).toContain(
      'What does the archive say about "Marigold Ledger"?',
    );
    expect(body.questions.some((question) => /PDF page/i.test(question))).toBe(
      false,
    );

    await page.goto("/ask");
    const suggestion = page
      .getByTestId("ask-example")
      .filter({ hasText: "Archive Test Section" });
    await expect(suggestion).toHaveCount(1);
    await suggestion.click();
    await expect(page.getByTestId("question-input")).toHaveValue(
      'What does the archive say about "Archive Test Section"?',
    );
    await askUntilCited(
      page,
      /ZX-ASK-SUGGESTION-2026/i,
      /ask-suggestions\.md/,
    );

    // The content-derived suggestion answers from the page it was drawn from.
    await page.goto("/ask");
    const derived = page
      .getByTestId("ask-example")
      .filter({ hasText: "Marigold Ledger" });
    await expect(derived).toHaveCount(1);
    await derived.click();
    await expect(page.getByTestId("question-input")).toHaveValue(
      'What does the archive say about "Marigold Ledger"?',
    );
    await askUntilCited(page, /Willowmere/i, /ask-suggestions\.md/);
  });
});

// Suggestions are an aid, never a gate: an empty or failed suggestion read must
// leave the free-text question available.
test.describe("Ask suggestions states", () => {
  test("an empty suggestion list still accepts a question", async ({ page }) => {
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, questions: [] }),
      }),
    );

    await page.goto("/ask");
    await expect(page.getByTestId("ask-suggestions-empty")).toBeVisible();
    await expect(page.getByTestId("ask-example")).toHaveCount(0);

    await page.getByTestId("question-input").fill("Who founded the family?");
    await expect(page.getByTestId("ask-button")).toBeEnabled();
  });

  test("a failed suggestion request still accepts a question", async ({
    page,
  }) => {
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({
          ok: false,
          error: "query_suggestions_unavailable",
        }),
      }),
    );

    await page.goto("/ask");
    await expect(page.getByTestId("ask-suggestions-error")).toBeVisible();
    await expect(page.getByTestId("ask-example")).toHaveCount(0);

    await page.getByTestId("question-input").fill("Who founded the family?");
    await expect(page.getByTestId("ask-button")).toBeEnabled();
  });
});
