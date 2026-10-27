// Capture deterministic stage-four UI evidence for the owner profile.
//
// The script drives the real Next.js page but intercepts only the browser
// session, profile, and media endpoints. It never reads a database or writes
// application data. Every name, value, document, and decision is synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin-stage4.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-4/before node scripts/capture-ui-reskin-stage4.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-4/after",
);
fs.mkdirSync(outDir, { recursive: true });

const SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const PNG_BYTES = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAABgAAAAYCAIAAABvFaqvAAAAH0lEQVR42mPQCLChCmIYNWjUoFGDRg0aNWjUoIE3CAAHDpUQ1ztOKgAAAABJRU5ErkJggg==",
  "base64",
);

const profileValue = (field_key, value, visibility = "family") => ({
  field_key,
  value,
  visibility,
  approved_revision_number: 2,
  approved_revision_id: 20,
  updated_at: "2026-10-20T12:00:00Z",
  restricted_from_family: false,
  restriction_generation: null,
});

const revision = (overrides) => ({
  revision_id: 7,
  field_key: "biography",
  revision_number: 3,
  prior_value_text: null,
  prior_approved_revision_number: 2,
  value: "",
  visibility: "family",
  status: "draft",
  changed_at: "2026-10-21T12:00:00Z",
  submitted_at: null,
  reviewed_at: null,
  review_note: null,
  ...overrides,
});

const PROFILE = {
  ok: true,
  linked: true,
  profile: {
    display_name: "Nguyễn Văn An",
    approved: [
      profileValue("display_name", "Nguyễn Văn An"),
      profileValue("preferred_name", "Ông An"),
      profileValue(
        "biography",
        "Nguyễn Văn An grew up near the river and kept the family register for three decades.",
      ),
      profileValue("current_location", "Example Hamlet", "owner_private"),
      profileValue(
        "my_story",
        "The family kept the first house beside the river. Every summer, the household gathered for the ancestral remembrance.",
      ),
      profileValue(
        "family_memories",
        "He remembered the lanterns at the river festival and the names of every cousin who visited.",
      ),
    ],
    pending: [
      revision({
        revision_id: 9,
        field_key: "biography",
        revision_number: 4,
        value: "An updated synthetic biography waiting for independent review.",
        status: "pending_review",
        submitted_at: "2026-10-22T12:00:00Z",
      }),
    ],
    history: [
      revision({
        revision_id: 6,
        field_key: "preferred_name",
        revision_number: 2,
        value: "Ông An",
        status: "needs_changes",
        reviewed_at: "2026-10-20T12:00:00Z",
        review_note: "Synthetic reviewer: add the spelling used in the source.",
      }),
      revision({
        revision_id: 5,
        field_key: "biography",
        revision_number: 2,
        value: "Approved synthetic biography.",
        status: "approved",
        submitted_at: "2026-10-19T12:00:00Z",
        reviewed_at: "2026-10-20T12:00:00Z",
      }),
    ],
  },
  media: [
    {
      media_id: 1,
      revision_id: 21,
      kind: "profile_photo",
      mime_type: "image/png",
      byte_size: 4096,
      file_name: "synthetic-profile.png",
      visibility: "family",
      width: 24,
      height: 24,
      status: "approved",
      created_at: "2026-10-20T12:00:00Z",
      restricted_from_family: false,
    },
    {
      media_id: 2,
      revision_id: 22,
      kind: "personal_document",
      mime_type: "application/pdf",
      byte_size: 182000,
      file_name: "synthetic-family-letter.pdf",
      visibility: "family",
      width: null,
      height: null,
      status: "approved",
      created_at: "2026-10-20T12:00:00Z",
      restricted_from_family: false,
    },
  ],
};

const NO_PHOTO_PROFILE = {
  ...PROFILE,
  media: PROFILE.media.filter((asset) => asset.kind !== "profile_photo"),
};

const VIEWPORTS = {
  desktop: { width: 1440, height: 1024 },
  mobile: { width: 390, height: 844 },
};

async function newPage(
  browser,
  {
    viewport = "desktop",
    colorScheme = "light",
    session = SESSION,
    profile = PROFILE,
    profileStatus = 200,
    delayMs = 0,
  } = {},
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
      body: JSON.stringify(session),
    }),
  );
  await context.route("**/api/profile/media/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "image/png",
      body: PNG_BYTES,
    }),
  );
  await context.route("**/api/profile", async (route) => {
    if (delayMs > 0) {
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    await route.fulfill({
      status: profileStatus,
      contentType: "application/json",
      body: JSON.stringify(profile),
    });
  });
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
  await page.waitForTimeout(100);
  await page.screenshot({
    path: path.join(outDir, `${name}.png`),
    fullPage: true,
  });
  // eslint-disable-next-line no-console
  console.log(`captured ${name}.png`);
}

async function open(page) {
  await page.goto(`${baseUrl}/profile`, { waitUntil: "networkidle" });
}

const browser = await chromium.launch();
try {
  for (const colorScheme of ["light", "dark"]) {
    const { context, page } = await newPage(browser, { colorScheme });
    await open(page);
    await page.getByTestId("profile-header").waitFor();
    await shot(page, `desktop-${colorScheme}-profile`);

    if (colorScheme === "light") {
      await page.getByTestId("profile-add-whatsapp").click();
      await page.getByTestId("profile-input-whatsapp").waitFor();
      await shot(page, "desktop-light-editor");
      await page.getByRole("button", { name: "Cancel", exact: true }).click();
    }
    await context.close();

    const mobile = await newPage(browser, {
      viewport: "mobile",
      colorScheme,
    });
    await open(mobile.page);
    await mobile.page.getByTestId("profile-header").waitFor();
    await shot(mobile.page, `mobile-${colorScheme}-profile`);
    await mobile.context.close();
  }

  const states = [
    {
      name: "desktop-light-no-photo",
      options: { profile: NO_PHOTO_PROFILE },
      wait: (page) => page.getByTestId("profile-header").waitFor(),
    },
    {
      name: "desktop-light-loading",
      options: { delayMs: 5000 },
      wait: (page) => page.getByTestId("profile-loading").waitFor(),
    },
    {
      name: "desktop-light-unavailable",
      options: {
        profileStatus: 503,
        profile: {
          ok: false,
          error: "unavailable",
          message: "The profile service is unavailable.",
        },
      },
      wait: (page) => page.getByTestId("profile-retry").waitFor(),
    },
    {
      name: "desktop-light-unlinked",
      options: { profile: { ok: true, linked: false } },
      wait: (page) => page.getByTestId("profile-unlinked").waitFor(),
    },
    {
      name: "desktop-light-expired",
      options: {
        profileStatus: 401,
        profile: { ok: false, error: "expired" },
      },
      wait: (page) => page.getByTestId("profile-expired").waitFor(),
    },
    {
      name: "desktop-light-signin",
      options: { session: { state: "anonymous" } },
      wait: (page) => page.getByTestId("profile-signin").waitFor(),
    },
  ];

  for (const state of states) {
    const { context, page } = await newPage(browser, state.options);
    await page.goto(`${baseUrl}/profile`, { waitUntil: "domcontentloaded" });
    await state.wait(page);
    await shot(page, state.name);
    await context.close();
  }
} finally {
  await browser.close();
}
