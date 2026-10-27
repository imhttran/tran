// Capture deterministic stage-six UI evidence for the Head of Family page.
//
// The script drives the real Next.js page but stubs only the browser-facing
// session, Head of Family, record, and nomination endpoints. It never reads a
// database or writes application data. Every record below is synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin-stage6.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-6/before node scripts/capture-ui-reskin-stage6.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-6/after",
);
fs.mkdirSync(outDir, { recursive: true });

const SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const HOF = {
  catalog: "available",
  conclusions: [
    {
      conclusion: "Head of Family",
      record_id: "nguyen-van-an",
      appointment_id: "appointment-7",
      head_label: "Nguyễn Văn An",
      evidence: [
        {
          evidence_id: "ev-1",
          citation: "Synthetic family register p.4",
          document_ref: "synthetic/register.pdf",
          page: 4,
          locator: "entry 4",
          excerpt: "Nguyễn Văn An giữ sổ hộ và ghi chép các thành viên.",
        },
        {
          evidence_id: "ev-2",
          citation: "Synthetic family letter",
          document_ref: "synthetic/letters.pdf",
          page: 2,
          locator: null,
          excerpt: null,
        },
      ],
      evidence_status: "verified",
    },
  ],
};

const HOF_EMPTY = { catalog: "available", conclusions: [] };

const HOF_UNAVAILABLE = {
  catalog: "unavailable",
  reason: "Synthetic catalog outage.",
};

const RECORDS = {
  records: [
    { record_id: "nguyen-van-an", person_label: "Nguyễn Văn An" },
    { record_id: "tran-thi-binh", person_label: "Trần Thị Bình" },
  ],
};

const NOMINATIONS = {
  nominations: [
    { nomination_id: 11, record_id: "nguyen-van-an", status: "pending" },
  ],
};

const VIEWPORTS = {
  desktop: { width: 1440, height: 1024 },
  mobile: { width: 390, height: 844 },
};

async function newPage(
  browser,
  { viewport = "desktop", colorScheme = "light", hof = HOF } = {},
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
  await context.route("**/api/genealogy/head-of-family", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(hof),
    }),
  );
  await context.route("**/api/genealogy/records", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(RECORDS),
    }),
  );
  await context.route("**/api/genealogy/nominations", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(NOMINATIONS),
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
    const { context, page } = await newPage(browser, { colorScheme });
    await page.goto(`${baseUrl}/head-of-family`, { waitUntil: "networkidle" });
    await page.getByTestId("hof-view").waitFor();
    await shot(page, `desktop-${colorScheme}-hof`);

    await page.getByTestId("approve-button").first().click();
    await page.getByTestId("confirm-dialog").waitFor();
    await shot(page, `desktop-${colorScheme}-confirm`);
    await context.close();

    const empty = await newPage(browser, { colorScheme, hof: HOF_EMPTY });
    await empty.page.goto(`${baseUrl}/head-of-family`, {
      waitUntil: "networkidle",
    });
    await empty.page.getByTestId("hof-view").waitFor();
    await shot(empty.page, `desktop-${colorScheme}-empty`);
    await empty.context.close();

    const unavailable = await newPage(browser, {
      colorScheme,
      hof: HOF_UNAVAILABLE,
    });
    await unavailable.page.goto(`${baseUrl}/head-of-family`, {
      waitUntil: "networkidle",
    });
    await unavailable.page.getByTestId("hof-catalog-unavailable").waitFor();
    await shot(unavailable.page, `desktop-${colorScheme}-unavailable`);
    await unavailable.context.close();

    const mobile = await newPage(browser, {
      viewport: "mobile",
      colorScheme,
    });
    await mobile.page.goto(`${baseUrl}/head-of-family`, {
      waitUntil: "networkidle",
    });
    await mobile.page.getByTestId("hof-view").waitFor();
    await shot(mobile.page, `mobile-${colorScheme}-hof`);

    await mobile.page.getByTestId("approve-button").first().click();
    await mobile.page.getByTestId("confirm-dialog").waitFor();
    await shot(mobile.page, `mobile-${colorScheme}-confirm`);
    await mobile.context.close();
  }
} finally {
  await browser.close();
}
