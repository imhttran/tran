// Capture deterministic stage-eight UI evidence for sign-in and denial states.
//
// Usage (with the Next.js app already running with a configured OIDC provider):
//   node scripts/capture-ui-reskin-stage8.mjs
//   BASE_URL=http://127.0.0.1:3001 OUT_DIR=../docs/reports/ui-reskin/stage-8/before node scripts/capture-ui-reskin-stage8.mjs
//
// Set UNCONFIGURED_BASE_URL to a second server started with empty OIDC_ISSUER
// and OIDC_CLIENT_ID to capture the operator "not configured" state.

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const unconfiguredUrl = (process.env.UNCONFIGURED_BASE_URL || "").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-8/after",
);
fs.mkdirSync(outDir, { recursive: true });

const ANONYMOUS = { state: "anonymous" };
const VIEWER = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};
const VIEWPORTS = {
  desktop: { width: 1440, height: 1024 },
  mobile: { width: 390, height: 844 },
};

async function newPage(
  browser,
  { viewport = "desktop", colorScheme = "light", session = ANONYMOUS } = {},
) {
  const context = await browser.newContext({
    viewport: VIEWPORTS[viewport],
    colorScheme,
    deviceScaleFactor: 1,
  });
  await context.route("/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
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
  await page.evaluate(() => window.scrollTo(0, 0));
  await page.waitForTimeout(150);
  await page.screenshot({
    path: path.join(outDir, `${name}.png`),
    fullPage: true,
  });
  // eslint-disable-next-line no-console
  console.log(`captured ${name}.png`);
}

const browser = await chromium.launch();
try {
  for (const colorScheme of ["light", "dark"]) {
    const desktop = await newPage(browser, { colorScheme });
    await desktop.page.goto(`${baseUrl}/signin`, { waitUntil: "networkidle" });
    await desktop.page.getByTestId("signin-view").waitFor();
    await shot(desktop.page, `desktop-${colorScheme}-signin`);

    await desktop.page.goto(`${baseUrl}/signin?error=state_mismatch`, {
      waitUntil: "networkidle",
    });
    await desktop.page.getByTestId("signin-view").waitFor();
    await shot(desktop.page, `desktop-${colorScheme}-signin-error`);
    await desktop.context.close();

    const denied = await newPage(browser, { colorScheme, session: VIEWER });
    await denied.page.goto(`${baseUrl}/admin/document-extraction`, {
      waitUntil: "networkidle",
    });
    await denied.page.getByTestId("denied-view").waitFor();
    await shot(denied.page, `desktop-${colorScheme}-denied`);
    await denied.context.close();
  }

  const mobile = await newPage(browser, {
    viewport: "mobile",
    colorScheme: "light",
  });
  await mobile.page.goto(`${baseUrl}/signin`, { waitUntil: "networkidle" });
  await mobile.page.getByTestId("signin-view").waitFor();
  await shot(mobile.page, "mobile-light-signin");
  await mobile.context.close();

  if (unconfiguredUrl) {
    const unconfigured = await newPage(browser, { colorScheme: "light" });
    await unconfigured.page.goto(`${unconfiguredUrl}/signin`, {
      waitUntil: "networkidle",
    });
    await unconfigured.page.getByTestId("signin-view").waitFor();
    await shot(unconfigured.page, "desktop-light-signin-unconfigured");
    await unconfigured.context.close();
  }
} finally {
  await browser.close();
}
