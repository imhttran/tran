// Capture deterministic stage-seven UI evidence for the four admin consoles.
//
// The script drives the real Next.js pages but stubs only the browser-facing
// session and API endpoints. It never reads a database or writes application
// data. Every record below is synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin-stage7.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-7/before node scripts/capture-ui-reskin-stage7.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-7/after",
);
fs.mkdirSync(outDir, { recursive: true });

const ADMIN = {
  state: "signed_in",
  role: "admin",
  display_name: "Synthetic admin",
};

const SUBMISSION = {
  version_id: 201,
  document_key: "synthetic-letter",
  version_number: 1,
  corpus: "family",
  source: "synthetic-letter.md",
  title: "Synthetic family letter",
  description: "A synthetic letter.",
  category: "letter",
  origin: "upload",
  access_scope: "family",
  review_status: "approved",
  processing_status: "ready",
  content_sha256: "a".repeat(64),
  byte_size: 1234,
  mime_type: "text/markdown",
  original_filename: "synthetic-letter.md",
  reviewed_by: "Synthetic admin",
  reviewed_at: "2026-10-20T12:00:00Z",
  review_note: null,
  removed_at: null,
  created_by: "Synthetic admin",
  created_at: "2026-10-19T12:00:00Z",
  updated_at: "2026-10-20T12:00:00Z",
};

const CANDIDATE = {
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  evidence_status: "verified",
  match_kind: "exact_name",
  matched_name: "Nguyễn Văn An",
};

const EXTRACTION = {
  ok: true,
  extraction: {
    extraction_id: 31,
    document_version_id: 201,
    document_source: "synthetic-letter.md",
    content_sha256: "a".repeat(64),
    status: "draft",
    created_by: "Synthetic admin",
    created_at: "2026-10-21T12:00:00Z",
    applied_by: null,
    applied_at: null,
  },
  proposals: [
    {
      proposal_id: 1,
      ordinal: 1,
      kind: "person",
      subject_name: "Nguyễn Văn An",
      target_name: null,
      resolution: "matched",
      resolved_person_key: "nguyen-van-an",
      candidates: [CANDIDATE],
      value_text: null,
      support_text: "Nguyễn Văn An giữ sổ hộ và ghi chép các thành viên.",
      status: "proposed",
      decided_by: null,
      applied_result: null,
    },
    {
      proposal_id: 2,
      ordinal: 2,
      kind: "alias",
      subject_name: "Nguyễn Văn An",
      target_name: null,
      resolution: "resolved",
      resolved_person_key: "nguyen-van-an",
      candidates: [],
      value_text: "Ông An",
      support_text: "Người trong làng gọi ông là Ông An.",
      status: "proposed",
      decided_by: null,
      applied_result: null,
    },
  ],
};

const BULK_PREVIEW = {
  ok: true,
  rows: [
    {
      person_key: "nguyen-van-an",
      classification: "eligible",
      reasons: [],
      proposed_username: "nguyen.van.an",
      preparable: true,
      blocked_missing_recipient: false,
      sendable: true,
    },
  ],
  summary: {
    eligible_for_invite: 1,
    needs_review: 0,
    already_linked: 0,
    already_prepared: 0,
    invited: 0,
    not_eligible: 0,
    blocked_missing_recipient: 0,
  },
  review_needed: null,
};

const BULK_PREPARE = {
  ok: true,
  batch_id: 41,
  batch_key: "batch-41",
  created: true,
  prepared: 1,
  skipped: 0,
  needs_review: 0,
  failed: 0,
  items: [
    {
      person_key: "nguyen-van-an",
      status: "prepared",
      reason: null,
      reservation_id: 1,
    },
  ],
};

const BULK_BATCH = {
  ok: true,
  batch_id: 41,
  status: "prepared",
  confirmed_count: 1,
  created_by: "Synthetic admin",
  created_at: "2026-10-21T12:00:00Z",
  completed_at: null,
  items: [
    {
      person_key: "nguyen-van-an",
      status: "prepared",
      reason: null,
      delivery_status: "pending",
      provider: null,
      attempts: 0,
      username: "nguyen.van.an",
      invitation_status: null,
      has_verified_contact: true,
    },
  ],
};

const MEMBERS = {
  ok: true,
  members: [
    {
      user_id: 1,
      issuer: "synthetic-issuer",
      subject: "subject-1",
      email: "an@example.test",
      display_name: "Nguyễn Văn An",
      roles: ["admin"],
      status: "active",
      created_at: "2026-10-20T12:00:00Z",
      linked_person: {
        link_id: 9,
        person_key: "nguyen-van-an",
        username: "an",
      },
    },
    {
      user_id: 2,
      issuer: "synthetic-issuer",
      subject: "subject-2",
      email: null,
      display_name: "Trần Thị Bình",
      roles: ["viewer"],
      status: "deactivated",
      created_at: "2026-10-20T12:00:00Z",
      linked_person: null,
    },
  ],
  next_after: null,
  linkage: "available",
};

const PASSAGE = {
  passage_id: 901,
  language: "vi",
  section_title: "Hồi ký gia đình",
  document_ref: "synthetic-memoir.md",
  page_start: 2,
  page_end: 3,
  review_status: "pending",
  visibility: "admin_only",
  body_excerpt: "Đoạn trích hồi ký gia đình.",
  body: "Đoạn một của hồi ký.\n\nĐoạn hai của hồi ký.",
  updated_at: "2026-10-21T12:00:00Z",
};

const NARRATIVE_LINK = {
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  link_status: "resolved",
  match_kind: "exact_name",
  evidence_note: "Synthetic evidence note.",
  reviewed_by: null,
  reviewed_at: null,
};

const NARRATIVE_PERSON = {
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  aliases: ["Ông An"],
  birth: null,
  death: null,
  parents: [],
  spouses: [],
  children: [],
};

const NARRATIVE_QUEUE = {
  ok: true,
  status: "pending",
  language: null,
  person: null,
  unmatched: false,
  link: "all",
  visibility: null,
  document: null,
  page_from: null,
  page_to: null,
  search: null,
  offset: 0,
  limit: 20,
  total: 1,
  truncated: false,
  documents: ["synthetic-memoir.md"],
  entries: [{ passage: PASSAGE, links: [NARRATIVE_LINK] }],
};

const NARRATIVE_DETAIL = {
  ok: true,
  passage: PASSAGE,
  links: [NARRATIVE_LINK],
  people: [NARRATIVE_PERSON],
  events: [
    {
      event_id: 1,
      person_key: "nguyen-van-an",
      actor: "Synthetic admin",
      action: "submitted",
      from_status: null,
      to_status: "pending",
      visibility: null,
      note: null,
      created_at: "2026-10-21T12:00:00Z",
    },
  ],
};

const ASSERTION = {
  assertion_id: 11,
  person_key: "nguyen-van-an",
  display_name: "Nguyễn Văn An",
  kind: "name",
  key: "display_name",
  value_text: "Nguyễn Văn An",
  review_status: "verified",
  citations: [
    {
      document_ref: "synthetic-letter.md",
      page: 1,
      locator: null,
      excerpt: "Nguyễn Văn An.",
      label: "Synthetic letter p.1",
    },
  ],
};

const AUTHORITY_PREVIEW = {
  ok: true,
  preview: {
    document_version_id: 201,
    document_ref: "synthetic-letter.md",
    content_sha256: "a".repeat(64),
    scope: "document",
    person_key: null,
    items: [
      {
        action: "replaced",
        incoming: ASSERTION,
        replaces: [
          { ...ASSERTION, assertion_id: 10, value_text: "Nguyễn Văn A" },
        ],
      },
    ],
    omissions: [{ ...ASSERTION, assertion_id: 12 }],
    conflicts: [
      {
        code: "loses_authority",
        assertion: { ...ASSERTION, assertion_id: 13 },
      },
    ],
    preview_hash: "preview-hash-abc",
  },
};

const AUTHORITY_DECISIONS = {
  ok: true,
  decisions: [
    {
      decision_id: 5,
      scope: "document",
      kind: "designate",
      status: "active",
      document_version_id: 201,
      document_ref: "synthetic-letter.md",
      content_sha256: "a".repeat(64),
      person_key: null,
      reason: "Synthetic decision reason.",
      actor: "Synthetic admin",
      created_at: "2026-10-21T12:00:00Z",
      retired_at: null,
      reverses_decision_id: null,
      supersedes_decision_id: null,
      items: [{ action: "replaced", incoming: ASSERTION, replaces: [] }],
      needs_attention: true,
      unsupported_assertions: [ASSERTION],
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
      body: JSON.stringify(ADMIN),
    }),
  );
  await context.route("**/api/documents/submissions**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, submissions: [SUBMISSION] }),
    }),
  );
  await context.route("**/api/genealogy/documents/201/extraction", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(EXTRACTION),
    }),
  );
  await context.route("**/api/genealogy/document-proposals/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(EXTRACTION),
    }),
  );
  await context.route("**/api/genealogy/documents/201/apply", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        extraction_id: 31,
        already_applied: false,
        applied: [],
      }),
    }),
  );
  await context.route("**/api/admin/family-members/bulk/preview", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BULK_PREVIEW),
    }),
  );
  await context.route("**/api/admin/family-members/bulk/prepare", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BULK_PREPARE),
    }),
  );
  await context.route("**/api/admin/family-members/bulk/batches/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(BULK_BATCH),
    }),
  );
  await context.route("**/api/admin/family-members", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(MEMBERS),
    }),
  );
  await context.route("**/api/admin/family-members?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(MEMBERS),
    }),
  );
  await context.route("**/api/admin/narrative/passages?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(NARRATIVE_QUEUE),
    }),
  );
  await context.route("**/api/admin/narrative/passages/901", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(NARRATIVE_DETAIL),
    }),
  );
  await context.route("**/api/admin/narrative/persons?**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ ok: true, people: [NARRATIVE_PERSON] }),
    }),
  );
  await context.route(
    "**/api/genealogy/documents/201/authority/preview",
    (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify(AUTHORITY_PREVIEW),
      }),
  );
  await context.route(
    "**/api/genealogy/documents/201/authority/apply",
    (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ok: true,
          decision: {
            decision_id: 6,
            status: "active",
            applied: 1,
            superseded: 1,
            removed: 0,
          },
        }),
      }),
  );
  await context.route("**/api/genealogy/authority**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(AUTHORITY_DECISIONS),
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

async function captureExtraction(page, prefix) {
  await page.goto(`${baseUrl}/admin/document-extraction`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("extraction-panel").waitFor();
  await shot(page, `${prefix}-extraction`);
}

async function captureFamily(page, prefix, { confirm = false } = {}) {
  await page.goto(`${baseUrl}/admin/family-members`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("bulk-preview").waitFor();
  if (confirm) {
    await page.getByRole("button", { name: "Select eligible" }).click();
    await page.getByTestId("bulk-prepare").click();
    await page.getByTestId("bulk-prepare-confirm").waitFor();
    await shot(page, `${prefix}-family-confirm`);
    return;
  }
  await shot(page, `${prefix}-family`);
}

async function captureNarrative(page, prefix) {
  await page.goto(`${baseUrl}/admin/narrative-review`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("narrative-review-row-901").click();
  await page.getByTestId("narrative-review-detail-901").waitFor();
  await shot(page, `${prefix}-narrative`);
}

async function captureAuthority(page, prefix) {
  await page.goto(`${baseUrl}/admin/source-of-truth`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("authority-preview").click();
  await page.getByTestId("authority-preview-panel").waitFor();
  await shot(page, `${prefix}-source-of-truth`);
}

const browser = await chromium.launch();
try {
  for (const colorScheme of ["light", "dark"]) {
    const desktop = await newPage(browser, { colorScheme });
    await captureExtraction(desktop.page, `desktop-${colorScheme}`);
    await captureFamily(desktop.page, `desktop-${colorScheme}`);
    await captureFamily(desktop.page, `desktop-${colorScheme}`, {
      confirm: true,
    });
    await captureNarrative(desktop.page, `desktop-${colorScheme}`);
    await captureAuthority(desktop.page, `desktop-${colorScheme}`);
    await desktop.context.close();
  }

  const mobile = await newPage(browser, {
    viewport: "mobile",
    colorScheme: "light",
  });
  await captureExtraction(mobile.page, "mobile-light");
  await captureFamily(mobile.page, "mobile-light");
  await captureNarrative(mobile.page, "mobile-light");
  await captureAuthority(mobile.page, "mobile-light");
  await mobile.context.close();
} finally {
  await browser.close();
}
