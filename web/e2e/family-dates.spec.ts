import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";

import {
  cardLifespanLabel,
  formatStructuredDate,
  lifespanLabel,
  parseStructuredDate,
  precisionLabel,
} from "../src/lib/dates";
import type { DatePrecision, DateQualifier, StructuredDate } from "../src/lib/dates";

// TRAN-002H: date presentation.
//
// The formatter tests need no browser and no database. The two page tests stub
// the API so they prove only what this change owns: the card label, the popup
// line, and the family-profile "Dates" section. They never read a real family
// record.

const DATE = (
  start: string,
  end: string | null,
  precision: DatePrecision,
  qualifier: DateQualifier,
): StructuredDate => ({ start, end, precision, qualifier });

test.describe("date formatting", () => {
  test("preserves precision and uncertainty", () => {
    expect(formatStructuredDate(DATE("1877", null, "year", "exact")))
      .toBe("1877");
    expect(formatStructuredDate(DATE("1877-03", null, "month", "exact")))
      .toBe("Mar 1877");
    expect(formatStructuredDate(DATE("1877-03-15", null, "day", "exact")))
      .toBe("15 Mar 1877");
    expect(formatStructuredDate(DATE("1877", null, "year", "circa")))
      .toBe("c. 1877");
    expect(formatStructuredDate(DATE("1877", null, "year", "after")))
      .toBe("after 1877");
    expect(
      formatStructuredDate(DATE("1877", "1922", "range", "exact")),
    ).toBe("1877–1922");
    expect(precisionLabel("year")).toBe("Year only");
    expect(precisionLabel("day")).toBe("Exact day");
  });

  test("a missing death date is never presented as living status", () => {
    expect(lifespanLabel(DATE("1877", null, "year", "exact"), null))
      .toBe("b. 1877");
    expect(lifespanLabel(null, DATE("1922", null, "year", "exact")))
      .toBe("d. 1922");
    expect(lifespanLabel(null, null)).toBe("");
    expect(lifespanLabel(DATE("1877", null, "year", "exact"),
      DATE("1922", null, "year", "exact"))).toBe("1877–1922");
  });

  test("a card label is omitted when it does not fit", () => {
    const birth = DATE("1877-03-15", null, "day", "exact");
    const death = DATE("1922-11-02", null, "day", "exact");
    expect(cardLifespanLabel(birth, death, 20)).toBe("");
    expect(cardLifespanLabel(birth, death, 60)).toContain("15 Mar 1877");
    // The default card budget is deliberately tight, so a full-day range never
    // collides with the status text.
    expect(cardLifespanLabel(birth, death)).toBe("");
    expect(cardLifespanLabel(birth, null)).toBe("b. 15 Mar 1877");
  });

  test("an unreadable date is dropped rather than guessed", () => {
    expect(parseStructuredDate({ start: "15/03/1877", end: null,
      precision: "day", qualifier: "exact" })).toBeNull();
    expect(parseStructuredDate({ start: "1877", end: null,
      precision: "century", qualifier: "exact" })).toBeNull();
    expect(parseStructuredDate(null)).toBeNull();
    // A range without a second bound, and a second bound without a range, are
    // both inconsistent and dropped.
    expect(parseStructuredDate({ start: "1877", end: null,
      precision: "range", qualifier: "exact" })).toBeNull();
    expect(parseStructuredDate({ start: "1877", end: "1922",
      precision: "year", qualifier: "exact" })).toBeNull();
    expect(parseStructuredDate({ start: "1877", end: null,
      precision: "year", qualifier: "exact" })).toEqual({
      start: "1877", end: null, precision: "year", qualifier: "exact",
    });
  });
});

const session = { state: "signed_in", role: "viewer", display_name: "Synthetic viewer" };

async function stubSession(page: Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
}

test("a verified lifespan appears on the tree card and in the popup", async ({ page }) => {
  await stubSession(page);
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        people: [
          { record_id: "dated", person_label: "Synthetic Dated Person",
            evidence_status: "verified", aliases: [],
            birth: DATE("1877", null, "year", "exact"),
            death: DATE("1922", null, "year", "circa") },
          { record_id: "undated", person_label: "Synthetic Undated Person",
            evidence_status: "unverified", aliases: [],
            birth: null, death: null },
        ],
        links: [],
        truncated: false,
      }),
    }));
  await page.route("**/api/genealogy/person/**", (route) =>
    route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ record_id: "dated", person_label: "Synthetic Dated Person",
        assertions: [] }) }));

  await page.goto("/genealogy");
  const cards = page.getByTestId("family-person");
  await expect(cards).toHaveCount(2);
  await expect(page.getByTestId("family-person-life")).toHaveCount(1);
  await expect(page.getByTestId("family-person-life")).toHaveText("1877–c. 1922");

  // The canvas is pannable and zoomable, so pointer clicks are not a reliable
  // way to select a card here. The keyboard path is the accessible one and
  // exercises the same open handler.
  const dated = page.getByRole("button", { name: "Select Synthetic Dated Person", exact: true });
  await dated.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("family-popup-lifespan")).toHaveText("1877–c. 1922");

  const undated = page.getByRole("button", { name: "Select Synthetic Undated Person", exact: true });
  await undated.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("family-popup-lifespan")).toHaveCount(0);
});

test("a family profile shows reviewed dates with their precision", async ({ page }) => {
  await stubSession(page);
  await page.route("**/api/family/persons/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        state: "published",
        person: { person_key: "dated", display_name: "Synthetic Dated Person" },
        fields: [],
        media: [],
        dates: [
          { kind: "birth_date", value: "1901", start: "1901", end: null,
            precision: "year", qualifier: "exact" },
          { kind: "death_date", value: "khoảng 1980", start: "1980", end: null,
            precision: "year", qualifier: "circa" },
        ],
      }),
    }));

  await page.goto("/family/dated");
  const section = page.getByTestId("family-profile-dates");
  await expect(section).toBeVisible();
  await expect(page.getByTestId("family-profile-date-birth_date"))
    .toContainText("1901");
  await expect(page.getByTestId("family-profile-date-birth_date"))
    .toContainText("Year only");
  const death = page.getByTestId("family-profile-date-death_date");
  await expect(death).toContainText("c. 1980");
  await expect(death).toContainText("as written: “khoảng 1980”");
  await expect(page.getByTestId("family-profile-empty")).toHaveCount(0);
});

test("a profile without verified dates shows no dates section", async ({ page }) => {
  await stubSession(page);
  await page.route("**/api/family/persons/**", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({
        ok: true,
        state: "empty",
        person: { person_key: "undated", display_name: "Synthetic Undated Person" },
        fields: [],
        media: [],
        dates: [],
      }),
    }));

  await page.goto("/family/undated");
  await expect(page.getByTestId("family-profile-dates")).toHaveCount(0);
  await expect(page.getByTestId("family-profile-empty")).toBeVisible();
});
