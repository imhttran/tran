import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// TRAN-002I: "History from the family archive" on a family profile.
//
// Runs against the real Next.js app, the proxy, the Axum API, and the disposable
// genealogy database seeded by scripts/web-e2e.sh. The fixture gives one person
// an approved, family-visible passage plus a pending and an admin-only passage,
// all synthetic, so the privacy assertions are about real rows.

const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000").origin;
const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const PERSON = "e2e-branch-subject";

const APPROVED = "SYNTHETIC-ARCHIVE-HISTORY-APPROVED";
const PENDING = "SYNTHETIC-ARCHIVE-HISTORY-PENDING";
const ADMIN_ONLY = "SYNTHETIC-ARCHIVE-HISTORY-ADMIN";

const READY = Boolean(REVIEWER);

type HistoryBody = {
  history: {
    passage_id: number;
    language: string;
    section_title: string | null;
    page_start: number;
    page_end: number;
    body: string;
  }[];
  fields: { field_key: string }[];
};

async function useToken(page: Page, token: string) {
  await page.context().addCookies([
    { name: "rag_session", value: token, url: ORIGIN, httpOnly: true, sameSite: "Lax" },
    { name: "rag_csrf", value: "e2e-csrf-token", url: ORIGIN, sameSite: "Lax" },
  ]);
}

async function api(
  request: APIRequestContext,
  token: string,
  path: string,
): Promise<{ status: number; body: unknown }> {
  const response = await request.get(`${API_URL}${path}`, {
    headers: { authorization: `Bearer ${token}`, accept: "application/json" },
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test("an approved passage reaches the family profile with its source pages", async ({
  page,
}) => {
  await useToken(page, REVIEWER);
  const response = await page.request.get(
    `${ORIGIN}/api/family/persons/${PERSON}/profile`,
    { headers: { accept: "application/json" } },
  );
  expect(response.status()).toBe(200);
  const body = (await response.json()) as HistoryBody;

  expect(body.history).toHaveLength(1);
  const passage = body.history[0];
  expect(passage.language).toBe("vi");
  expect(passage.page_start).toBe(8);
  expect(passage.page_end).toBe(8);
  expect(passage.section_title).toBe("Synthetic archive section");
  expect(passage.body).toContain(APPROVED);
  expect(passage.body).toContain("\n\n");
});

test("pending and admin-only passages never reach a family response", async ({
  page,
}) => {
  await useToken(page, REVIEWER);
  const response = await page.request.get(
    `${ORIGIN}/api/family/persons/${PERSON}/profile`,
    { headers: { accept: "application/json" } },
  );
  const raw = await response.text();
  expect(response.status()).toBe(200);
  expect(raw).toContain(APPROVED);
  expect(raw).not.toContain(PENDING);
  expect(raw).not.toContain(ADMIN_ONLY);
});

test("the profile page shows the archive history separately from member fields", async ({
  page,
}) => {
  await useToken(page, REVIEWER);
  await page.goto(`/family/${PERSON}`);

  const section = page.getByTestId("family-profile-history");
  await expect(section).toBeVisible();
  await expect(section.getByRole("heading", { name: "History from the family archive" }))
    .toBeVisible();
  await expect(section).toContainText(APPROVED);
  await expect(page.getByTestId("family-history-source")).toContainText("Vietnamese");
  await expect(page.getByTestId("family-history-source")).toContainText("PDF page 8");

  // The member's own approved biography is a separate section and is unchanged.
  await expect(page.getByTestId("family-profile-field-biography")).toBeVisible();
});

test("an unauthorized caller receives no history", async ({ page, request }) => {
  // No session cookie in the browser and no token on the API.
  const anonymous = await api(request, "", `/api/family/persons/${PERSON}/profile`);
  expect([401, 403]).toContain(anonymous.status);
  expect(JSON.stringify(anonymous.body)).not.toContain(APPROVED);
});
