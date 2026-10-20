import { expect, test } from "@playwright/test";

import { layoutTree, relationLabel, revealPerson } from "../src/lib/family-tree";
import type { TreeGraph } from "../src/lib/family-tree";

const session = { state: "signed_in", role: "viewer", display_name: "Synthetic viewer" };
const graph = {
  people: [
    { record_id: "root", person_label: "Nguyễn Văn A", evidence_status: "verified", aliases: ["Nguyen A"] },
    { record_id: "child", person_label: "Trần Văn B", evidence_status: "unverified", aliases: [] },
    { record_id: "grandchild", person_label: "Child C", evidence_status: "needs_review", aliases: [] },
    { record_id: "disconnected", person_label: "Unlinked Person", evidence_status: "unverified", aliases: [] },
  ],
  links: [
    { from_id: "root", to_id: "child", kind: "parent_child",
      evidence_status: "verified", evidence: [{ evidence_id: "e-1", citation: "synthetic p.2" }] },
    { from_id: "child", to_id: "grandchild", kind: "parent_child",
      evidence_status: "needs_review", evidence: [] },
  ],
  truncated: false,
};
const person = {
  record_id: "root",
  person_label: "Nguyễn Văn A",
  assertions: [{ assertion_id: 101, kind: "birth", key: "date",
    value_text: "1901", review_status: "verified",
    evidence: [{ evidence_id: "e-2", citation: "synthetic p.3" }] }],
};

async function stubAuthorized(page: import("@playwright/test").Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(graph) }));
  await page.route("**/api/genealogy/person/**", (route) =>
    route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ ...person,
        record_id: decodeURIComponent(route.request().url().split("/").pop() || ""),
      }) }));
}

test("graph, citations, disconnected records, and uncertain edges stay distinct", async ({ page }) => {
  await stubAuthorized(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("genealogy-tree")).toBeVisible();
  await expect(page.getByTestId("family-person")).toHaveCount(4);
  await expect(page.getByTestId("family-relationship")).toHaveCount(2);
  await expect(page.locator("[data-testid=family-relationship][data-status=verified]")).toHaveCount(1);
  await expect(page.locator("[data-testid=family-relationship][data-status=needs_review]")).toHaveCount(1);
  await expect(page.getByTestId("genealogy-record")).toHaveCount(4);
  await expect(page.getByTestId("family-assertions")).toContainText("synthetic p.3");
});

test("Vietnamese name search reveals the requested person", async ({ page }) => {
  await stubAuthorized(page);
  await page.goto("/genealogy");
  await page.getByTestId("family-search").fill("Nguyen A");
  await expect(page.getByTestId("family-search-results")).toContainText("Nguyễn Văn A");
  await page.getByTestId("family-search-results").getByRole("button").first().click();
  await expect(page.getByTestId("family-person-details")).toContainText("Nguyễn Văn A");
});

test("collapsing a lineage does not delete catalog records", async ({ page }) => {
  await stubAuthorized(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-person")).toHaveCount(4);
  await page.getByTestId("family-collapse").click();
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await expect(page.getByTestId("genealogy-record")).toHaveCount(4);
  await page.getByRole("button", { name: "Expand all" }).click();
  await expect(page.getByTestId("family-person")).toHaveCount(4);
});

test("a 403 denies the tree before any person details render", async ({ page }) => {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 403, contentType: "application/json", body: "{}" }));
  await page.goto("/genealogy");
  await expect(page.getByTestId("denied-view")).toBeVisible();
  await expect(page.getByTestId("family-person")).toHaveCount(0);
  await expect(page.getByTestId("family-person-details")).toHaveCount(0);
});

test("unreferenced graph edges are rejected rather than fabricated", async ({ page }) => {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ ...graph, links: [
        ...graph.links, { from_id: "root", to_id: "unknown-id",
          kind: "parent_child", evidence_status: "verified", evidence: [] },
      ] }) }));
  await page.goto("/genealogy");
  await expect(page.getByTestId("denied-view")).toBeVisible();
  await expect(page.getByTestId("family-relationship")).toHaveCount(0);
});

test("anonymous graph request is denied by Rust API", async ({ request }) => {
  const response = await request.get("/api/genealogy/tree");
  expect([401, 403]).toContain(response.status());
});

// --- Relationship direction, path reveal, and source drill-down -------------

const p = (record_id: string, aliases: string[] = []) => ({
  record_id, person_label: record_id, evidence_status: "unverified" as const, aliases });
const pc = (from_id: string, to_id: string) => ({
  from_id, to_id, kind: "parent_child" as const, evidence_status: "unverified" as const, evidence: [] });
const shown = (g: TreeGraph, collapsed: ReadonlySet<string>) =>
  layoutTree(g, collapsed).people.map((x) => x.record_id).sort();

// R → A → A1, R → B → B1
const branches: TreeGraph = {
  people: [p("R"), p("A"), p("A1", ["Ánh Một"]), p("B"), p("B1")],
  links: [pc("R", "A"), pc("A", "A1"), pc("R", "B"), pc("B", "B1")],
  truncated: false,
};

test("relationship labels follow the selected person's perspective", () => {
  const link = pc("parent", "child");
  expect(relationLabel(link, "parent")).toBe("Child");
  expect(relationLabel(link, "child")).toBe("Parent");
  expect(relationLabel({ ...link, kind: "spouse" }, "child")).toBe("Spouse");
});

test("reveal expands only the hidden ancestor path", () => {
  const next = revealPerson(branches, new Set(["R", "B"]), "A1");
  expect([...next]).toEqual(["B"]);
  expect(shown(branches, next)).toEqual(["A", "A1", "B", "R"]);
  // Already visible: nothing changes.
  expect([...revealPerson(branches, new Set(["B"]), "A")]).toEqual(["B"]);
});

test("reveal and layout terminate on cycles, shared ancestors, and multiple parents", () => {
  // X ⇄ Y is a parent cycle with no parent-free entry; Y and Z share child C;
  // M and F are both parents of C.
  const g: TreeGraph = {
    people: [p("M"), p("F"), p("X"), p("Y"), p("C")],
    links: [pc("X", "Y"), pc("Y", "X"), pc("Y", "C"), pc("M", "C"), pc("F", "C")],
    truncated: false,
  };
  expect(shown(g, new Set())).toEqual(["C", "F", "M", "X", "Y"]);
  const collapsed = new Set(["M", "F", "X", "Y"]);
  expect(shown(g, collapsed)).not.toContain("C");
  const next = revealPerson(g, collapsed, "C");
  expect(shown(g, next)).toContain("C");
  expect(next.size).toBe(3); // Exactly one parent path was expanded.
});

const branchGraph = {
  people: branches.people.map((x) => ({ ...x, person_label: "Person " + x.record_id })),
  links: [
    { ...pc("R", "A"), evidence_status: "verified",
      evidence: [{ evidence_id: "synthetic-register#4", citation: "synthetic-register p.4",
        document_ref: "synthetic-register", page: 4 }] },
    pc("A", "A1"), pc("R", "B"), pc("B", "B1"),
  ],
  truncated: false,
};

async function stubBranches(page: import("@playwright/test").Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(branchGraph) }));
  await page.route("**/api/genealogy/person/**", (route) => {
    const id = decodeURIComponent(route.request().url().split("/").pop() || "");
    return route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ record_id: id, person_label: "Person " + id, assertions: [] }) });
  });
}

async function collapse(page: import("@playwright/test").Page, id: string) {
  await page.getByRole("button", { name: "Select Person " + id, exact: true }).click();
  await page.getByTestId("family-collapse").click();
}

test("the details panel labels parents and children from the selection", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  const relatives = page.getByTestId("family-relative");
  await expect(relatives.filter({ hasText: "Person R" }).getByTestId("family-relation")).toHaveText("Parent");
  await expect(relatives.filter({ hasText: "Person A1" }).getByTestId("family-relation")).toHaveText("Child");
});

test("search reveals a person without expanding unrelated branches", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await collapse(page, "B");
  await collapse(page, "R");
  await expect(page.getByTestId("family-person")).toHaveCount(1);
  await page.getByTestId("family-search").fill("anh mot"); // Alias, without diacritics.
  await page.getByTestId("family-search-results").getByRole("button").first().click();
  await expect(page.getByTestId("family-person-details")).toContainText("Person A1");
  await expect(page.getByTestId("family-person")).toHaveCount(4); // R, A, A1, B — B1 stays collapsed.
  await expect(page.getByRole("button", { name: "Select Person B1" })).toHaveCount(0);
});

test("catalog selection reveals the selected person", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await collapse(page, "B");
  await page.getByTestId("genealogy-records").getByRole("button", { name: "Person B1" }).click();
  await expect(page.getByRole("button", { name: "Select Person B1" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Select Person B1" })).toHaveAttribute("aria-pressed", "true");
});

test("a tree citation opens metadata-only source details", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  await page.getByTestId("citation-button").filter({ hasText: "synthetic-register p.4" }).click();
  const dialog = page.getByTestId("source-details");
  await expect(dialog).toBeVisible();
  await expect(dialog).toContainText("synthetic-register");
  await expect(dialog).toContainText("Page");
  await expect(dialog).toContainText("Person R — parent of Person A");
  await expect(dialog.getByTestId("source-metadata-only")).toBeVisible();
  await expect(dialog.locator("a")).toHaveCount(0); // No link to a nonexistent viewer.
  await dialog.getByRole("button", { name: "Close" }).click();
  await expect(dialog).toBeHidden();
});
