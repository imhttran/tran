import { expect, test } from "@playwright/test";
import type { APIRequestContext, Page } from "@playwright/test";

// TRAN-002I: "History from the family archive" on a family profile.
//
// Runs against the real Next.js app, the proxy, the Axum API, and the disposable
// genealogy database seeded by scripts/web-e2e.sh. The fixture gives one person
// an approved, family-visible passage plus a pending and an admin-only passage,
// all synthetic, so the privacy assertions are about real rows.

const ORIGIN = new URL(process.env.E2E_BASE_URL || "http://127.0.0.1:3000")
  .origin;
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
    document_ref: string;
    section_title: string | null;
    page_start: number;
    page_end: number;
    body: string;
  }[];
  fields: { field_key: string }[];
};

async function useToken(page: Page, token: string) {
  await page.context().addCookies([
    {
      name: "rag_session",
      value: token,
      url: ORIGIN,
      httpOnly: true,
      sameSite: "Lax",
    },
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
  expect(passage.document_ref).toBe("synthetic/e2e-branch-review-source.md");
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
  await expect(
    section.getByRole("heading", { name: "History from the family archive" }),
  ).toBeVisible();
  await expect(section).toContainText(APPROVED);
  await expect(page.getByTestId("family-history-source")).toContainText(
    "Vietnamese",
  );
  await expect(page.getByTestId("family-history-source")).toContainText(
    "PDF page 8",
  );
  await expect(page.getByTestId("family-history-source")).toContainText(
    "synthetic/e2e-branch-review-source.md",
  );

  // The member's own approved biography is a separate section and is unchanged.
  await expect(
    page.getByTestId("family-profile-field-biography"),
  ).toBeVisible();
});

test("the tree shows Personal history only for a person with published history", async ({
  page,
}) => {
  await useToken(page, REVIEWER);
  await page.goto("/genealogy");

  const eligible = page.getByRole("group", {
    name: "Synthetic Branch Subject",
    exact: true,
  });
  const link = eligible.getByTestId("family-person-history");
  await expect(link).toBeVisible();

  const ineligible = page.getByRole("group", {
    name: "Synthetic Second Branch Subject",
    exact: true,
  });
  await expect(ineligible.getByTestId("family-person-history")).toHaveCount(0);

  // The full tree can be wider than the viewport, so use the existing search
  // control to bring the card into view before clicking its link. Escape closes
  // the person popup that opening a search result also shows.
  await page.getByTestId("family-search").fill("Synthetic Branch Subject");
  await page
    .getByTestId("family-search-results")
    .getByRole("button")
    .first()
    .click();
  await page.keyboard.press("Escape");
  await expect(link).toBeVisible();

  await link.click();
  const dialog = page.getByTestId("personal-history-dialog");
  await expect(dialog).toBeVisible();
  await expect(dialog.getByTestId("personal-history-passage")).toHaveCount(1);
  await expect(dialog).toContainText(APPROVED);
  await expect(dialog).toContainText("Vietnamese");
  await expect(dialog).toContainText("synthetic/e2e-branch-review-source.md");
  await expect(dialog).toContainText("PDF pages");
  await expect(dialog).toContainText("page 8");

  // Pending and admin-only text is absent from the dialog payload and view.
  const rendered = (await dialog.textContent()) ?? "";
  expect(rendered).not.toContain(PENDING);
  expect(rendered).not.toContain(ADMIN_ONLY);

  // Opening history never opens the person-details popup.
  await expect(page.getByTestId("family-person-details")).toHaveCount(0);

  // Escape closes the modal and restores focus to the invoking link.
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(link).toBeFocused();
});

test("an unauthorized caller receives no history", async ({
  page,
  request,
}) => {
  // No session cookie in the browser and no token on the API.
  const anonymous = await api(
    request,
    "",
    `/api/family/persons/${PERSON}/profile`,
  );
  expect([401, 403]).toContain(anonymous.status);
  expect(JSON.stringify(anonymous.body)).not.toContain(APPROVED);
});
