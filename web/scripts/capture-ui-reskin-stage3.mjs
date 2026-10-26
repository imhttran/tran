// Capture deterministic stage-three UI evidence for the shared top navigation
// and light/dark palettes.
//
// The script drives the real Next.js pages but intercepts only the browser
// session, Ask suggestions, and the empty admin review queue. It never reads a
// database, calls Ollama, or writes application data. Every label and record is
// synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin-stage3.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-3/before node scripts/capture-ui-reskin-stage3.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-3/after",
);
fs.mkdirSync(outDir, { recursive: true });

const SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const SUGGESTIONS = {
  ok: true,
  questions: [
    'What does the archive say about the "Synthetic register"?',
    "Where did the synthetic family live?",
  ],
};

const REVIEW_QUEUE = {
  ok: true,
  status: "pending_review",
  limit: 20,
  next_cursor: null,
  entries: [],
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
  await context.route("**/api/admin/reviews?*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(REVIEW_QUEUE),
    }),
  );
  const page = await context.newPage();
  await page.addInitScript(() => {
    const style = document.createElement("style");
    style.textContent = "nextjs-portal { display: none !important; }";
    document.addEventListener("DOMContentLoaded", () => {
      document.head.appendChild(style);
    });
  });
  return { context, page };
}

async function shot(page, name) {
  await page.screenshot({
    path: path.join(outDir, `${name}.png`),
    fullPage: true,
  });
  // eslint-disable-next-line no-console
  console.log(`captured ${name}.png`);
}

async function open(page, route) {
  await page.goto(`${baseUrl}${route}`, { waitUntil: "networkidle" });
}

const browser = await chromium.launch();
try {
  for (const colorScheme of ["light", "dark"]) {
    // Desktop: Ask the archive and the header.
    const desktop = await newPage(browser, { colorScheme });
    await open(desktop.page, "/ask");
    await desktop.page.waitForTimeout(250);
    await shot(desktop.page, `desktop-${colorScheme}-ask`);

    // More only exists after stage three; capture it once it is present.
    const more = desktop.page.getByTestId("more-toggle");
    if ((await more.count()) > 0) {
      await more.click();
      await shot(desktop.page, `desktop-${colorScheme}-more-open`);
      await desktop.page.keyboard.press("Escape");
    }

    // Admin review: the header plus a real empty review workspace.
    await open(desktop.page, "/admin/reviews");
    await desktop.page.getByTestId("review-console").waitFor();
    await shot(desktop.page, `desktop-${colorScheme}-admin-review`);
    await desktop.context.close();

    // Mobile: Ask with the full navigation menu open.
    const mobile = await newPage(browser, {
      viewport: "mobile",
      colorScheme,
    });
    await open(mobile.page, "/ask");
    await mobile.page.getByTestId("nav-toggle").click();
    await shot(mobile.page, `mobile-${colorScheme}-menu-open`);
    await mobile.context.close();
  }
} finally {
  await browser.close();
}
