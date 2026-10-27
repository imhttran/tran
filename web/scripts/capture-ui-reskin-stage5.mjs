// Capture deterministic stage-five UI evidence for documents and review consoles.
//
// The script drives the real Next.js pages but stubs only the browser-facing
// session, document, review, and branch-review endpoints. It never reads a
// database or writes application data. Every record below is synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin-stage5.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-5/before node scripts/capture-ui-reskin-stage5.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-5/after",
);
fs.mkdirSync(outDir, { recursive: true });

const SESSION = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const DOCUMENTS = {
  ok: true,
  submissions: [
    {
      version_id: 101,
      document_key: "synthetic-family-letter",
      version_number: 2,
      corpus: "family",
      source: "synthetic-family-letter.txt",
      title: "Synthetic family letter",
      description: "A synthetic letter for the workspace visual suite.",
      category: "letter",
      origin: "upload",
      access_scope: "family",
      review_status: "approved",
      processing_status: "ready",
      content_sha256: "a".repeat(64),
      byte_size: 1234,
      mime_type: "text/plain",
      original_filename: "synthetic-family-letter.txt",
      reviewed_by: "Synthetic admin",
      reviewed_at: "2026-10-20T12:00:00Z",
      review_note: null,
      removed_at: null,
      created_by: "Synthetic admin",
      created_at: "2026-10-19T12:00:00Z",
      updated_at: "2026-10-20T12:00:00Z",
    },
    {
      version_id: 102,
      document_key: "synthetic-register",
      version_number: 1,
      corpus: "family",
      source: "synthetic-register.pdf",
      title: "Synthetic family register",
      description: "A synthetic register awaiting review.",
      category: "genealogy",
      origin: "upload",
      access_scope: "admin_only",
      review_status: "pending_review",
      processing_status: "queued",
      content_sha256: "b".repeat(64),
      byte_size: 9876,
      mime_type: "application/pdf",
      original_filename: "synthetic-register.pdf",
      reviewed_by: null,
      reviewed_at: null,
      review_note: null,
      removed_at: null,
      created_by: "Synthetic admin",
      created_at: "2026-10-21T12:00:00Z",
      updated_at: "2026-10-21T12:00:00Z",
    },
  ],
};

const JOBS = {
  ok: true,
  jobs: [
    {
      job_id: 11,
      version_id: 101,
      status: "failed",
      attempts: 3,
      max_attempts: 3,
      last_error: "Synthetic processing failure.",
      created_at: "2026-10-20T12:00:00Z",
      updated_at: "2026-10-20T12:05:00Z",
    },
  ],
};

const REVIEW_ENTRY = {
  display_name: "Nguyễn Văn An",
  revision: {
    revision_id: 501,
    field_key: "biography",
    revision_number: 3,
    prior_value_text: "Approved synthetic biography.",
    prior_approved_revision_number: 2,
    value:
      "Nguyễn Văn An kept the family register and recorded the household at Example Hamlet.",
    visibility: "family",
    status: "pending_review",
    changed_at: "2026-10-21T12:00:00Z",
    submitted_at: "2026-10-21T12:05:00Z",
    reviewed_at: null,
    review_note: null,
  },
  current_approved: {
    field_key: "biography",
    value: "Approved synthetic biography.",
    visibility: "family",
    approved_revision_number: 2,
    approved_revision_id: 20,
    updated_at: "2026-10-20T12:00:00Z",
    restricted_from_family: false,
    restriction_generation: null,
  },
  current_media: null,
  media: null,
  can_decide: true,
  decision_block: null,
};

const REVIEW_QUEUE = {
  ok: true,
  status: "pending_review",
  limit: 20,
  next_cursor: null,
  entries: [REVIEW_ENTRY],
};

const REVIEW_DETAIL = {
  ok: true,
  entry: REVIEW_ENTRY,
  events: [
    {
      event: "submitted",
      actor: "Synthetic owner",
      note: null,
      created_at: "2026-10-21T12:05:00Z",
    },
  ],
  history: [
    {
      revision_id: 20,
      field_key: "biography",
      revision_number: 2,
      prior_value_text: null,
      prior_approved_revision_number: 1,
      value: "Approved synthetic biography.",
      visibility: "family",
      status: "approved",
      changed_at: "2026-10-20T12:00:00Z",
      submitted_at: "2026-10-20T12:00:00Z",
      reviewed_at: "2026-10-20T12:05:00Z",
      review_note: "Synthetic approval.",
    },
  ],
};

const BRANCH_PROPOSAL = {
  proposal_id: 701,
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  branch_key: "branch-1",
  assertion_kind: "name",
  assertion_key: "display_name",
  prior_value_text: "Nguyễn Văn An",
  prior_review_status: "verified",
  proposed_value_text: "Nguyễn Văn An (synthetic correction)",
  evidence: [
    {
      source_evidence_id: 1,
      document_ref: "synthetic/register.pdf",
      page: 4,
    },
  ],
  submitted_at: "2026-10-21T12:00:00Z",
  status: "pending_review",
  can_decide: true,
  decision_block: null,
};

const BRANCH_QUEUE = {
  ok: true,
  status: "pending_review",
  limit: 20,
  next_cursor: null,
  submissions: [BRANCH_PROPOSAL],
};

const BRANCH_DETAIL = {
  ok: true,
  proposal: BRANCH_PROPOSAL,
  current_assertion: {
    assertion_id: 11,
    value_text: "Nguyễn Văn An",
    review_status: "verified",
    reviewed_at: "2026-10-20T12:00:00Z",
  },
  events: [
    {
      event: "submitted",
      actor: "Synthetic reviewer",
      note: null,
      created_at: "2026-10-21T12:00:00Z",
    },
  ],
};

const BRANCH_EVIDENCE = {
  ok: true,
  evidence: [
    {
      source_evidence_id: 2,
      document_ref: "synthetic/letters.pdf",
      page: 2,
      locator: "entry 4",
      assertion_kind: "name",
      assertion_key: "alias",
    },
  ],
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
  await context.route("**/api/documents/submissions**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(DOCUMENTS),
    }),
  );
  await context.route("**/api/documents/jobs", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(JOBS),
    }),
  );
  await context.route("**/api/admin/reviews?*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(REVIEW_QUEUE),
    }),
  );
  await context.route("**/api/admin/reviews/501", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(REVIEW_DETAIL),
    }),
  );
  await context.route("**/api/branch-review/grants/me", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        grants: [{ branch_key: "branch-1", branch_label: "First Branch" }],
      }),
    }),
  );
  await context.route("**/api/branch-review/submissions?*", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BRANCH_QUEUE),
    }),
  );
  await context.route("**/api/branch-review/submissions/701", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BRANCH_DETAIL),
    }),
  );
  await context.route("**/api/branch-review/persons/**/evidence", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BRANCH_EVIDENCE),
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
  await page.waitForTimeout(100);
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
    await page.goto(`${baseUrl}/documents`, { waitUntil: "networkidle" });
    await page.getByTestId("document-row-101").waitFor();
    await shot(page, `desktop-${colorScheme}-documents`);

    await page.goto(`${baseUrl}/admin/reviews`, { waitUntil: "networkidle" });
    await page.getByTestId("review-row-501").click();
    await page.getByTestId("review-detail-501").waitFor();
    await shot(page, `desktop-${colorScheme}-review-detail`);

    await page.goto(`${baseUrl}/branch-review`, { waitUntil: "networkidle" });
    await page.getByTestId("branch-review-row-701").click();
    await page.getByTestId("branch-review-detail-701").waitFor();
    await shot(page, `desktop-${colorScheme}-branch-detail`);
    await context.close();

    const mobile = await newPage(browser, {
      viewport: "mobile",
      colorScheme,
    });
    await mobile.page.goto(`${baseUrl}/documents`, {
      waitUntil: "networkidle",
    });
    await mobile.page.getByTestId("document-row-101").waitFor();
    await shot(mobile.page, `mobile-${colorScheme}-documents`);

    await mobile.page.goto(`${baseUrl}/admin/reviews`, {
      waitUntil: "networkidle",
    });
    await mobile.page.getByTestId("review-row-501").click();
    await mobile.page.getByTestId("review-detail-501").waitFor();
    await shot(mobile.page, `mobile-${colorScheme}-review-detail`);

    await mobile.page.goto(`${baseUrl}/branch-review`, {
      waitUntil: "networkidle",
    });
    await mobile.page.getByTestId("branch-review-row-701").click();
    await mobile.page.getByTestId("branch-review-detail-701").waitFor();
    await shot(mobile.page, `mobile-${colorScheme}-branch-detail`);
    await mobile.context.close();
  }
} finally {
  await browser.close();
}
