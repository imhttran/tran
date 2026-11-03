import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

// BD-002 — recorded branch presentation in the family tree and the family
// profile. These are stubbed presentation checks, so they run without the test
// database. The API decides the state; these tests prove the UI renders that
// state in words, shows every recorded claim, and never invents a branch from
// an unresolved claim or an older payload without a branch block.

const session = {
  state: "signed_in",
  role: "viewer",
  display_name: "Synthetic viewer",
};

type Claim = {
  assertion_kind: string;
  value_text: string | null;
  review_status: string;
  verified: boolean;
  branch_key: string | null;
  branch_label: string | null;
};

type Block = {
  state: "missing" | "unverified" | "verified" | "conflicting";
  verified_branch_key: string | null;
  claims: Claim[];
};

function claim(
  text: string,
  status: string,
  verified: boolean,
  key: string | null,
  label: string | null,
): Claim {
  return {
    assertion_kind: "branch_claim",
    value_text: text,
    review_status: status,
    verified,
    branch_key: key,
    branch_label: label,
  };
}

const missing: Block = {
  state: "missing",
  verified_branch_key: null,
  claims: [],
};

const unverified: Block = {
  state: "unverified",
  verified_branch_key: null,
  claims: [claim("branch-1", "unverified", false, "branch-1", "First Branch")],
};

const verified: Block = {
  state: "verified",
  verified_branch_key: "branch-1",
  claims: [claim("branch-1", "verified", true, "branch-1", "First Branch")],
};

const conflicting: Block = {
  state: "conflicting",
  verified_branch_key: null,
  claims: [
    claim("branch-1", "verified", true, "branch-1", "First Branch"),
    claim("branch-2", "unverified", false, "branch-2", "Second Branch"),
  ],
};

const PERSON_KEY = "synthetic-branch-person";
const PERSON_NAME = "Synthetic Branch Person";

async function stubSession(page: Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(session),
    }),
  );
}

async function openTree(page: Page, branches: Block | undefined) {
  await stubSession(page);
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        people: [
          {
            record_id: PERSON_KEY,
            person_label: PERSON_NAME,
            evidence_status: "verified",
            aliases: [],
          },
        ],
        links: [],
        truncated: false,
      }),
    }),
  );
  await page.route("**/api/genealogy/person/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        record_id: PERSON_KEY,
        person_label: PERSON_NAME,
        assertions: [],
        ...(branches ? { branches } : {}),
      }),
    }),
  );
  await page.goto("/genealogy");
  await page
    .getByRole("button", { name: `Select ${PERSON_NAME}`, exact: true })
    .click();
  await expect(page.getByTestId("family-person-details")).toBeVisible();
}

async function openProfile(page: Page, branches: Block | undefined) {
  await stubSession(page);
  await page.route("**/api/family/persons/**/profile", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        state: "published",
        person: { person_key: PERSON_KEY, display_name: PERSON_NAME },
        fields: [],
        media: [],
        dates: [],
        history: [],
        archive_photographs: [],
        ...(branches ? { branches } : {}),
      }),
    }),
  );
  await page.goto(`/family/${PERSON_KEY}`);
  await expect(page.getByTestId("family-profile")).toBeVisible();
}

test.describe("family tree person details", () => {
  test("no claim shows no branch recorded and no invented label", async ({
    page,
  }) => {
    await openTree(page, missing);
    const section = page.getByTestId("family-popup-branch");
    await expect(section).toBeVisible();
    await expect(section).toHaveAttribute("data-branch-state", "missing");
    await expect(page.getByTestId("family-popup-branch-state")).toHaveText(
      "No branch recorded",
    );
    await expect(page.getByTestId("family-popup-branch-claim")).toHaveCount(0);
    await expect(page.getByTestId("family-popup-branch-resolved")).toHaveCount(
      0,
    );
  });

  test("an unverified claim shows the recorded text without a resolved branch", async ({
    page,
  }) => {
    await openTree(page, unverified);
    await expect(page.getByTestId("family-popup-branch-state")).toHaveText(
      "Recorded but not verified",
    );
    const claims = page.getByTestId("family-popup-branch-claim");
    await expect(claims).toHaveCount(1);
    await expect(claims.first()).toContainText("branch-1");
    await expect(claims.first()).toContainText("not verified");
    await expect(page.getByTestId("family-popup-branch-resolved")).toHaveCount(
      0,
    );
  });

  test("a verified claim shows the resolved label and key", async ({
    page,
  }) => {
    await openTree(page, verified);
    await expect(page.getByTestId("family-popup-branch-state")).toHaveText(
      "Verified recorded branch",
    );
    const resolved = page.getByTestId("family-popup-branch-resolved");
    await expect(resolved).toBeVisible();
    await expect(resolved).toContainText("First Branch");
    await expect(resolved).toContainText("branch-1");
  });

  test("conflicting claims show the disagreement and no single branch", async ({
    page,
  }) => {
    await openTree(page, conflicting);
    await expect(page.getByTestId("family-popup-branch-state")).toHaveText(
      "The recorded branches disagree",
    );
    await expect(page.getByTestId("family-popup-branch-claim")).toHaveCount(2);
    await expect(page.getByTestId("family-popup-branch-resolved")).toHaveCount(
      0,
    );
  });

  test("an older payload without a branch block renders no section", async ({
    page,
  }) => {
    await openTree(page, undefined);
    await expect(page.getByTestId("family-person-details")).toBeVisible();
    await expect(page.getByTestId("family-popup-branch")).toHaveCount(0);
  });
});

test.describe("family profile", () => {
  test("no claim shows no branch recorded", async ({ page }) => {
    await openProfile(page, missing);
    await expect(page.getByTestId("family-profile-branch")).toBeVisible();
    await expect(page.getByTestId("family-profile-branch-state")).toHaveText(
      "No branch recorded",
    );
    await expect(
      page.getByTestId("family-profile-branch-resolved"),
    ).toHaveCount(0);
  });

  test("an unverified claim shows the record and no resolved branch", async ({
    page,
  }) => {
    await openProfile(page, unverified);
    await expect(page.getByTestId("family-profile-branch-state")).toHaveText(
      "Recorded but not verified",
    );
    const claims = page.getByTestId("family-profile-branch-claim");
    await expect(claims).toHaveCount(1);
    await expect(claims.first()).toContainText("branch-1");
    await expect(claims.first()).toContainText("not verified");
    await expect(
      page.getByTestId("family-profile-branch-resolved"),
    ).toHaveCount(0);
    // A recorded claim is family information, so the empty-profile panel must
    // not also claim that nothing is shown.
    await expect(page.getByTestId("family-profile-empty")).toHaveCount(0);
  });

  test("a verified claim shows the resolved branch", async ({ page }) => {
    await openProfile(page, verified);
    await expect(page.getByTestId("family-profile-branch-state")).toHaveText(
      "Verified recorded branch",
    );
    const resolved = page.getByTestId("family-profile-branch-resolved");
    await expect(resolved).toContainText("First Branch");
    await expect(resolved).toContainText("branch-1");
    await expect(page.getByTestId("family-profile-empty")).toHaveCount(0);
  });

  test("conflicting claims show the disagreement", async ({ page }) => {
    await openProfile(page, conflicting);
    await expect(page.getByTestId("family-profile-branch-state")).toHaveText(
      "The recorded branches disagree",
    );
    await expect(page.getByTestId("family-profile-branch-claim")).toHaveCount(
      2,
    );
    await expect(
      page.getByTestId("family-profile-branch-resolved"),
    ).toHaveCount(0);
    await expect(page.getByTestId("family-profile-empty")).toHaveCount(0);
  });

  test("an older payload without a branch block omits the section", async ({
    page,
  }) => {
    await openProfile(page, undefined);
    await expect(page.getByTestId("family-profile")).toBeVisible();
    await expect(page.getByTestId("family-profile-branch-wrap")).toHaveCount(0);
  });
});
