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

    // PDF upload -> page provenance. Ingest the PDF first so the page question
    // is answered from a corpus that contains only the PDF.
    await page
      .getByTestId("file-input")
      .setInputFiles(
        path.join(repo, "fixtures/parity/inputs/text-layer.pdf"),
      );
    await page.getByTestId("ingest-button").click();
    await expect(page.getByTestId("ingest-status")).toContainText(
      "text-layer.pdf",
    );

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

    // Markdown upload -> ingestion status and a cited answer.
    await page.goto("/");
    await page
      .getByTestId("file-input")
      .setInputFiles(path.join(repo, "examples/loan-policy.md"));
    await page.getByTestId("ingest-button").click();
    await expect(page.getByTestId("ingest-status")).toContainText(
      "loan-policy.md",
    );
    await expect(page.getByTestId("ingest-status")).toContainText("chunk");

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

  test("incomplete PDF extraction error is visible", async ({ page }) => {
    await page.goto("/");
    await page
      .getByTestId("file-input")
      .setInputFiles(
        path.join(repo, "fixtures/parity/inputs/mixed-text-scan.pdf"),
      );
    await page.getByTestId("ingest-button").click();
    await expect(page.getByTestId("incomplete-pdf")).toContainText("2");
    await page.screenshot({
      path: path.join(screenshots, "extraction-error.png"),
      fullPage: true,
    });
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
});
