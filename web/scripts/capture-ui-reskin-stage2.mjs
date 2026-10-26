// Capture deterministic stage-two UI evidence for the family tree, person
// details, and family profile routes.
//
// The script drives the real Next.js pages but intercepts the browser-facing
// genealogy and family-profile endpoints it needs. It never reads a database,
// calls Ollama, or writes application data. Every person, record, document, and
// citation below is synthetic.
//
// Usage (with the Next.js app already running):
//   node scripts/capture-ui-reskin-stage2.mjs
//   BASE_URL=http://127.0.0.1:3000 OUT_DIR=../docs/reports/ui-reskin/stage-2/after node scripts/capture-ui-reskin-stage2.mjs

import { chromium } from "playwright";
import fs from "node:fs";
import path from "node:path";

const baseUrl = (process.env.BASE_URL || "http://127.0.0.1:3000").replace(
  /\/+$/,
  "",
);
const outDir = path.resolve(
  process.cwd(),
  process.env.OUT_DIR || "../docs/reports/ui-reskin/stage-2/after",
);
fs.mkdirSync(outDir, { recursive: true });

const SESSION = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};

const EVIDENCE = {
  evidence_id: "synthetic-evidence-1",
  citation: "Synthetic family register, page 4",
  document_ref: "synthetic-family-register.md",
  page: 4,
  excerpt: "The synthetic register records the household at Example Hamlet.",
};

const PEOPLE = [
  {
    record_id: "nguyen-van-an",
    person_label: "Nguyễn Văn An",
    evidence_status: "verified",
    aliases: ["Nguyen Van An", "Ông An"],
    birth: { start: "1920", end: null, precision: "year", qualifier: "exact" },
    death: { start: "1998", end: null, precision: "year", qualifier: "exact" },
    history_passages: 2,
  },
  {
    record_id: "tran-thi-binh",
    person_label: "Trần Thị Bình",
    evidence_status: "verified",
    aliases: ["Tran Thi Binh", "Bà Bình"],
    birth: { start: "1924", end: null, precision: "year", qualifier: "exact" },
    death: { start: "2001", end: null, precision: "year", qualifier: "exact" },
    history_passages: 0,
  },
  {
    record_id: "le-van-cuong",
    person_label: "Lê Văn Cường",
    evidence_status: "needs_review",
    aliases: ["Le Van Cuong"],
    birth: { start: "1950", end: null, precision: "year", qualifier: "circa" },
    death: null,
    history_passages: 1,
  },
  {
    record_id: "pham-thi-dung",
    person_label: "Phạm Thị Dung",
    evidence_status: "unverified",
    aliases: ["Pham Thi Dung"],
    birth: null,
    death: null,
    history_passages: 0,
  },
  {
    record_id: "hoang-van-em",
    person_label: "Hoàng Văn Em",
    evidence_status: "verified",
    aliases: ["Hoang Van Em"],
    birth: { start: "1978", end: null, precision: "year", qualifier: "exact" },
    death: null,
    history_passages: 0,
  },
];

const TREE = {
  people: PEOPLE,
  links: [
    {
      from_id: "nguyen-van-an",
      to_id: "le-van-cuong",
      kind: "parent_child",
      evidence_status: "verified",
      evidence: [EVIDENCE],
    },
    {
      from_id: "tran-thi-binh",
      to_id: "le-van-cuong",
      kind: "parent_child",
      evidence_status: "verified",
      evidence: [EVIDENCE],
    },
    {
      from_id: "nguyen-van-an",
      to_id: "pham-thi-dung",
      kind: "parent_child",
      evidence_status: "needs_review",
      evidence: [EVIDENCE],
    },
    {
      from_id: "tran-thi-binh",
      to_id: "pham-thi-dung",
      kind: "parent_child",
      evidence_status: "needs_review",
      evidence: [EVIDENCE],
    },
    {
      from_id: "le-van-cuong",
      to_id: "hoang-van-em",
      kind: "spouse",
      evidence_status: "verified",
      evidence: [EVIDENCE],
    },
  ],
  truncated: false,
};

const PERSON_DETAIL = {
  record_id: "nguyen-van-an",
  person_label: "Nguyễn Văn An",
  assertions: [
    {
      assertion_id: 101,
      kind: "display_name",
      key: "name",
      value_text: "Nguyễn Văn An",
      review_status: "verified",
      evidence: [EVIDENCE],
    },
    {
      assertion_id: 102,
      kind: "birth_date",
      key: "birth",
      value_text: "1920",
      review_status: "verified",
      date: { start: "1920", end: null, precision: "year", qualifier: "exact" },
      evidence: [EVIDENCE],
    },
    {
      assertion_id: 103,
      kind: "branch",
      key: "branch",
      value_text: "First branch",
      review_status: "needs_review",
      evidence: [EVIDENCE],
    },
  ],
};

const PROFILE = {
  ok: true,
  state: "published",
  person: { person_key: "nguyen-van-an", display_name: "Nguyễn Văn An" },
  fields: [
    {
      field_key: "display_name",
      value: "Nguyễn Văn An",
      visibility: "family",
      approved_revision_number: 2,
      updated_at: "2026-10-20T12:00:00Z",
    },
    {
      field_key: "preferred_name",
      value: "Ông An",
      visibility: "family",
      approved_revision_number: 2,
      updated_at: "2026-10-20T12:00:00Z",
    },
    {
      field_key: "biography",
      value:
        "Nguyễn Văn An was a schoolteacher in Example Hamlet. He kept the synthetic family register and recorded the household's ordinary days alongside its milestones.",
      visibility: "family",
      approved_revision_number: 2,
      updated_at: "2026-10-20T12:00:00Z",
    },
    {
      field_key: "my_story",
      value:
        "I grew up beside the river and learned to read from the household register. Later I taught the children of Example Hamlet and kept a small garden behind the schoolhouse.\n\nI hope the family remembers the ordinary kindnesses more than the dates.",
      visibility: "family",
      approved_revision_number: 1,
      updated_at: "2026-10-18T12:00:00Z",
    },
    {
      field_key: "family_memories",
      value:
        "During the harvest, the family gathered in the front room and told stories until the lamps burned low. The children were allowed to stay up for the last story only.",
      visibility: "family",
      approved_revision_number: 1,
      updated_at: "2026-10-18T12:00:00Z",
    },
  ],
  dates: [
    {
      kind: "birth_date",
      value: "1920",
      start: "1920",
      end: null,
      precision: "year",
      qualifier: "exact",
    },
    {
      kind: "death_date",
      value: "1998",
      start: "1998",
      end: null,
      precision: "year",
      qualifier: "exact",
    },
  ],
  history: [
    {
      passage_id: 301,
      language: "vi",
      document_ref: "synthetic-family-register.md",
      section_title: "Example Hamlet",
      page_start: 4,
      page_end: 5,
      body: "Nguyễn Văn An sống ở Example Hamlet bên bờ sông. Ông giữ sổ gia đình và ghi lại những ngày bình thường của gia đình.\n\nBản ghi này là dữ liệu tổng hợp để kiểm tra cách hiển thị tiếng Việt.",
    },
  ],
  media: [
    {
      kind: "profile_photo",
      media_id: 501,
      mime_type: "image/svg+xml",
      byte_size: 4200,
      file_name: "synthetic-portrait.svg",
      width: 600,
      height: 600,
      approved_revision_number: 1,
      created_at: "2026-10-19T12:00:00Z",
    },
    {
      kind: "personal_document",
      media_id: 502,
      mime_type: "application/pdf",
      byte_size: 120000,
      file_name: "synthetic-family-letter.pdf",
      width: null,
      height: null,
      approved_revision_number: 1,
      created_at: "2026-10-19T12:00:00Z",
    },
  ],
};

const HISTORY = {
  ok: true,
  person: { person_key: "nguyen-van-an", display_name: "Nguyễn Văn An" },
  history: PROFILE.history,
};

const PORTRAIT_SVG = `<svg xmlns="http://www.w3.org/2000/svg" width="600" height="600" viewBox="0 0 600 600">
  <rect width="600" height="600" fill="#e3f1ea"/>
  <circle cx="300" cy="235" r="105" fill="#0f6b4a" opacity="0.18"/>
  <circle cx="300" cy="215" r="72" fill="#0f6b4a" opacity="0.4"/>
  <path d="M160 520c12-105 72-160 140-160s128 55 140 160" fill="#0f6b4a" opacity="0.28"/>
  <text x="300" y="560" text-anchor="middle" font-family="system-ui, sans-serif" font-size="34" fill="#0f6b4a">Synthetic portrait</text>
</svg>`;

async function newPage(
  browser,
  { viewport = "desktop", colorScheme = "light" } = {},
) {
  const context = await browser.newContext({
    viewport:
      viewport === "mobile"
        ? { width: 390, height: 844 }
        : { width: 1440, height: 1024 },
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
  await context.route("**/api/genealogy/tree", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(TREE),
    }),
  );
  await context.route("**/api/genealogy/person/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(PERSON_DETAIL),
    }),
  );
  await context.route("**/api/family/persons/**/history", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(HISTORY),
    }),
  );
  await context.route("**/api/family/persons/**/media/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "image/svg+xml",
      body: PORTRAIT_SVG,
    }),
  );
  await context.route("**/api/family/persons/**/profile", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(PROFILE),
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

async function openTree(page) {
  await page.goto(`${baseUrl}/genealogy`, { waitUntil: "networkidle" });
  await page.getByTestId("family-person").first().waitFor();
}

async function openPopup(page) {
  await page
    .getByRole("button", { name: "Select Nguyễn Văn An", exact: true })
    .click();
  await page.getByTestId("family-person-details").waitFor();
  await page.waitForTimeout(260);
}

async function openProfile(page) {
  await page.goto(`${baseUrl}/family/nguyen-van-an`, {
    waitUntil: "networkidle",
  });
  await page.getByTestId("family-profile").waitFor();
}

async function captureTree(browser, name, options) {
  const { context, page } = await newPage(browser, options);
  await openTree(page);
  await shot(page, `${name}-tree`);
  await openPopup(page);
  await shot(page, `${name}-popup`);
  await context.close();
}

async function captureProfile(browser, name, options) {
  const { context, page } = await newPage(browser, options);
  await openProfile(page);
  await shot(page, `${name}-profile`);
  await context.close();
}

async function captureStates(browser) {
  // Unavailable tree. Before the reskin, a 503 is rendered as a denial; after
  // the reskin it gets its own unavailable state. Accept either while capturing
  // the comparison.
  {
    const { context, page } = await newPage(browser);
    await page.route("**/api/genealogy/tree", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ ok: false, error: "unavailable" }),
      }),
    );
    await page.goto(`${baseUrl}/genealogy`, { waitUntil: "networkidle" });
    await page
      .getByTestId("genealogy-unavailable")
      .or(page.getByTestId("denied-view"))
      .first()
      .waitFor();
    await shot(page, "desktop-light-tree-unavailable");
    await context.close();
  }

  // Empty tree.
  {
    const { context, page } = await newPage(browser);
    await page.route("**/api/genealogy/tree", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ people: [], links: [], truncated: false }),
      }),
    );
    await page.goto(`${baseUrl}/genealogy`, { waitUntil: "networkidle" });
    await page.getByTestId("genealogy-empty").waitFor();
    await shot(page, "desktop-light-tree-empty");
    await context.close();
  }

  // Unavailable profile.
  {
    const { context, page } = await newPage(browser);
    await page.route("**/api/family/persons/**/profile", (route) =>
      route.fulfill({
        status: 503,
        contentType: "application/json",
        body: JSON.stringify({ ok: false, error: "unavailable" }),
      }),
    );
    await page.goto(`${baseUrl}/family/nguyen-van-an`, {
      waitUntil: "networkidle",
    });
    await page.getByTestId("family-profile-unavailable").waitFor();
    await shot(page, "desktop-light-profile-unavailable");
    await context.close();
  }

  // Empty profile.
  {
    const { context, page } = await newPage(browser);
    await page.route("**/api/family/persons/**/profile", (route) =>
      route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({
          ...PROFILE,
          state: "empty",
          fields: [],
          dates: [],
          history: [],
          media: [],
        }),
      }),
    );
    await page.goto(`${baseUrl}/family/nguyen-van-an`, {
      waitUntil: "networkidle",
    });
    await page.getByTestId("family-profile-empty").waitFor();
    await shot(page, "desktop-light-profile-empty");
    await context.close();
  }
}

const browser = await chromium.launch();
try {
  await captureTree(browser, "desktop-light", {
    viewport: "desktop",
    colorScheme: "light",
  });
  await captureTree(browser, "desktop-dark", {
    viewport: "desktop",
    colorScheme: "dark",
  });
  await captureTree(browser, "mobile-light", {
    viewport: "mobile",
    colorScheme: "light",
  });
  await captureProfile(browser, "desktop-light", {
    viewport: "desktop",
    colorScheme: "light",
  });
  await captureProfile(browser, "desktop-dark", {
    viewport: "desktop",
    colorScheme: "dark",
  });
  await captureProfile(browser, "mobile-light", {
    viewport: "mobile",
    colorScheme: "light",
  });
  await captureProfile(browser, "mobile-dark", {
    viewport: "mobile",
    colorScheme: "dark",
  });
  await captureStates(browser);
} finally {
  await browser.close();
}
