// Capture deterministic UI evidence for the stage-one reskin.
//
// The script drives the real Next.js pages but intercepts only the three
// browser-facing endpoints it needs: session, suggestions, and query. It never
// reads a database, calls Ollama, or writes application data. Every label,
// document, and citation below is synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-1/after node scripts/capture-ui-reskin.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-1/after",
);
fs.mkdirSync(outDir, { recursive: true });

const SESSION = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};

const SUGGESTIONS = {
  ok: true,
  questions: [
    'What does the archive say about the "Synthetic register"?',
    "Where did the synthetic family live?",
    "Which family events are recorded in the register?",
  ],
};

const ANSWER = {
  answer:
    "The synthetic register records that Nguyễn Văn An lived in Example Hamlet beside the river. The same passage names two founders, but it does not support a living-status conclusion.",
  insufficient: false,
  sources: [
    {
      source: "synthetic-family-register.md",
      section: "Residence",
      page: 3,
      text: "Nguyễn Văn An lived in the village of Example Hamlet beside the river. Trần Thị Bình built the first house there, and Person D later moved to Sample Town.",
    },
    {
      source: "synthetic-family-register.md",
      section: "Founders",
      page: 4,
      text: "Two founders are recorded in the synthetic register. The entry does not name a Head of Family.",
    },
  ],
};

const INSUFFICIENT = {
  answer: "I cannot answer this question from the provided documents.",
  insufficient: true,
  sources: [],
};

const VIEWPORTS = {
  desktop: { width: 1440, height: 1024 },
  mobile: { width: 390, height: 844 },
};

async function newPage(
  browser,
  { viewport = "desktop", colorScheme = "light" } = {},
) {
  const context = await browser.newContext({
    viewport: VIEWPORTS[viewport],
    colorScheme,
    deviceScaleFactor: 1,
  });
  await context.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(SESSION),
    }),
  );
  await context.route("**/api/query/suggestions", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(SUGGESTIONS),
    }),
  );
  const page = await context.newPage();
  // The dev-server indicator is not part of the application visual.
  await page.addInitScript(() => {
    const style = document.createElement("style");
    style.textContent = "nextjs-portal { display: none !important; }";
    document.addEventListener("DOMContentLoaded", () => {
      document.head.appendChild(style);
    });
  });
  return { context, page };
}

async function routeQuery(page, body, { delayMs = 0, status = 200 } = {}) {
  await page.route("**/api/query", async (route) => {
    if (delayMs > 0)
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    await route.fulfill({
      status,
      contentType: "application/json",
      body: JSON.stringify(body),
    });
  });
}

async function shot(page, name) {
  await page.screenshot({
    path: path.join(outDir, `${name}.png`),
    fullPage: true,
  });
  // eslint-disable-next-line no-console
  console.log(`captured ${name}.png`);
}

async function openAsk(page) {
  await page.goto(`${baseUrl}/ask`, { waitUntil: "networkidle" });
}

async function ask(
  page,
  question = 'What does the archive say about the "Synthetic register"?',
) {
  await page.getByTestId("question-input").fill(question);
  await page.getByTestId("ask-button").click();
}

const browser = await chromium.launch();
try {
  // Desktop light: idle and answered.
  {
    const { context, page } = await newPage(browser);
    await openAsk(page);
    await page.getByTestId("ask-example").first().waitFor();
    await shot(page, "desktop-light-idle");

    await page.getByTestId("ask-example").first().click();
    await routeQuery(page, ANSWER);
    await page.getByTestId("ask-button").click();
    await page.getByTestId("answer").waitFor();
    await page.waitForTimeout(250);
    await shot(page, "desktop-light-answered");
    await context.close();
  }

  // Desktop dark: idle and answered.
  {
    const { context, page } = await newPage(browser, { colorScheme: "dark" });
    await openAsk(page);
    await page.getByTestId("ask-example").first().waitFor();
    await shot(page, "desktop-dark-idle");

    await page.getByTestId("ask-example").first().click();
    await routeQuery(page, ANSWER);
    await page.getByTestId("ask-button").click();
    await page.getByTestId("answer").waitFor();
    await page.waitForTimeout(250);
    await shot(page, "desktop-dark-answered");
    await context.close();
  }

  // Mobile light and dark: answered, plus the open navigation.
  for (const colorScheme of ["light", "dark"]) {
    const { context, page } = await newPage(browser, {
      viewport: "mobile",
      colorScheme,
    });
    await openAsk(page);
    await page.getByTestId("nav-toggle").click();
    await shot(page, `mobile-${colorScheme}-nav-open`);
    await page.getByTestId("nav-toggle").click();

    await page.getByTestId("ask-example").first().click();
    await routeQuery(page, ANSWER);
    await page.getByTestId("ask-button").click();
    await page.getByTestId("answer").waitFor();
    await page.waitForTimeout(250);
    await shot(page, `mobile-${colorScheme}-answered`);
    await context.close();
  }

  // Distinct states on desktop light.
  {
    const { context, page } = await newPage(browser);
    await openAsk(page);
    await page.getByTestId("ask-example").first().waitFor();

    await routeQuery(page, ANSWER, { delayMs: 5000 });
    await ask(page);
    await page.getByTestId("ask-loading").waitFor();
    await shot(page, "desktop-light-loading");
    await context.close();
  }

  {
    const { context, page } = await newPage(browser);
    await page.route("**/api/query/suggestions", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ ok: true, questions: [] }),
      }),
    );
    await openAsk(page);
    await page.getByTestId("ask-suggestions-empty").waitFor();
    await shot(page, "desktop-light-empty");
    await context.close();
  }

  {
    const { context, page } = await newPage(browser);
    await routeQuery(
      page,
      {
        ok: false,
        error: "query_unavailable",
        message: "retrieval or generation is unavailable",
      },
      { status: 503 },
    );
    await openAsk(page);
    await page.getByTestId("ask-example").first().waitFor();
    await ask(page, "Where did the synthetic family live?");
    await page.getByTestId("ask-error").waitFor();
    await page.waitForTimeout(250);
    await shot(page, "desktop-light-error");
    await context.close();
  }

  {
    const { context, page } = await newPage(browser);
    await routeQuery(page, INSUFFICIENT);
    await openAsk(page);
    await page.getByTestId("ask-example").first().waitFor();
    await ask(page, "What is not recorded in the synthetic register?");
    await page.getByTestId("insufficient").waitFor();
    await page.waitForTimeout(250);
    await shot(page, "desktop-light-insufficient");
    await context.close();
  }
} finally {
  await browser.close();
}
