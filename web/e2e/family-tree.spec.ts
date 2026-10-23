import { expect, test } from "@playwright/test";

import {
  layoutTree, relationLabel, revealPerson, READABLE_HEIGHT, READABLE_WIDTH,
} from "../src/lib/family-tree";
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

test("a rooted component with an attached ancestor cycle still renders every person", async ({ page }) => {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  const people = ["A", "B", "C", "D"].map((id) => ({
    record_id: id, person_label: "Person " + id, evidence_status: "unverified", aliases: [],
  }));
  const edge = (from_id: string, to_id: string) => ({
    from_id, to_id, kind: "parent_child",
    evidence_status: "unverified", evidence: [],
  });
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({
        people,
        links: [edge("A", "B"), edge("B", "A"), edge("B", "C"), edge("D", "C")],
        truncated: false,
      }),
    }));
  await page.route("**/api/genealogy/person/**", (route) =>
    route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({
        record_id: route.request().url().split("/").pop(),
        person_label: "Synthetic person", assertions: [],
      }),
    }));
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-person")).toHaveCount(4);
  await expect(page.getByTestId("family-relationship")).toHaveCount(4);
  await expect(page.getByTestId("genealogy-record")).toHaveCount(4);
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
  // An unrelated collapsed branch (U → V) must survive the reveal.
  const withU: TreeGraph = { ...g, people: [...g.people, p("U"), p("V")], links: [...g.links, pc("U", "V")] };
  const next = revealPerson(withU, new Set([...collapsed, "U"]), "C");
  expect(shown(withU, next)).toContain("C");
  expect([...next]).toEqual(["U"]); // Only C's collapsed ancestors were expanded.
});

test("reveal reaches a person whose only route is a spouse of a hidden branch", () => {
  // R → S, S married to A, parent cycle A ⇄ B; collapsing R hides S.
  const g: TreeGraph = {
    people: [p("R"), p("S"), p("A"), p("B")],
    links: [pc("R", "S"), { ...pc("S", "A"), kind: "spouse" }, pc("A", "B"), pc("B", "A")],
    truncated: false,
  };
  const collapsed = new Set(["R"]);
  expect(shown(g, revealPerson(g, collapsed, "A"))).toContain("A");
  expect(shown(g, revealPerson(g, collapsed, "S"))).toContain("S");
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

// --- Explicit camera controls, full screen, and a 263-person catalog --------

const manyUnlinked: TreeGraph = {
  people: Array.from({ length: 263 }, (_, index) =>
    p("L" + String(index).padStart(3, "0"))),
  links: [],
  truncated: false,
};

async function stubManyUnlinked(page: import("@playwright/test").Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(manyUnlinked) }));
  await page.route("**/api/genealogy/person/**", (route) => {
    const id = decodeURIComponent(route.request().url().split("/").pop() || "");
    return route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ record_id: id, person_label: "Person " + id, assertions: [] }) });
  });
}

test("Fit all and Focus selected are separate camera controls", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A1", exact: true }).click();
  const before = await readViewBox(page);
  await page.getByTestId("family-focus-selected").click();
  const focused = await readViewBox(page);
  expect(focused.width).toBeLessThanOrEqual(READABLE_WIDTH);
  expect(focused.height).toBeLessThanOrEqual(READABLE_HEIGHT);
  expect(focused.x).not.toBe(before.x);
  const card = await page.getByRole("button", { name: "Select Person A1", exact: true }).boundingBox();
  const canvas = await page.getByTestId("family-tree-canvas").boundingBox();
  expect(card && canvas).toBeTruthy();
  if (card && canvas) {
    expect(card.x).toBeGreaterThanOrEqual(canvas.x);
    expect(card.x + card.width).toBeLessThanOrEqual(canvas.x + canvas.width + 1);
  }
  await page.getByTestId("family-fit-all").click();
  const fitted = await readViewBox(page);
  expect(fitted.x).toBe(0);
  expect(fitted.y).toBe(0);
  expect(fitted.width).toBeGreaterThanOrEqual(600);
  expect(fitted.height).toBeGreaterThanOrEqual(370);
});

test("the canvas expands to full screen and Escape restores the layout", async ({ page }) => {
  await stubManyUnlinked(page);
  await page.goto("/genealogy");
  const toggle = page.getByTestId("family-canvas-toggle");
  await expect(page.getByTestId("genealogy-tree")).toHaveAttribute("data-expanded", "false");
  await expect(toggle).toHaveAttribute("aria-pressed", "false");
  const before = await readViewBox(page);
  await toggle.click();
  await expect(page.getByTestId("genealogy-tree")).toHaveAttribute("data-expanded", "true");
  await expect(toggle).toHaveAttribute("aria-pressed", "true");
  // Expanding changes the viewport, never the camera.
  expect(await readViewBox(page)).toEqual(before);
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("genealogy-tree")).toHaveAttribute("data-expanded", "false");
});

test("manual pan and zoom survive unrelated UI updates", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  const canvas = page.getByTestId("family-tree-canvas");
  const box = await canvas.boundingBox();
  expect(box).toBeTruthy();
  if (!box) return;

  // Pan by dragging blank canvas space.
  await page.mouse.move(box.x + 40, box.y + box.height - 30);
  await page.mouse.down();
  await page.mouse.move(box.x + 120, box.y + box.height - 90, { steps: 5 });
  await page.mouse.up();
  const panned = await readViewBox(page);

  // Zoom with the toolbar control.
  await page.getByRole("button", { name: "Zoom out" }).click();
  const zoomed = await readViewBox(page);
  expect(zoomed.width).toBeGreaterThan(panned.width);

  // Selecting a card, searching, and toggling the canvas must not reset it.
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  await expect(page.getByTestId("family-person-details")).toContainText("Person A");
  await page.getByTestId("family-search").fill("B1");
  await page.getByTestId("family-canvas-toggle").click();
  await page.getByTestId("family-canvas-toggle").click();
  expect(await readViewBox(page)).toEqual(zoomed);
  expect(zoomed.x).not.toBe(0);
});

test("263 unlinked people open readable and a search selection stays readable", async ({ page }) => {
  await stubManyUnlinked(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-person")).toHaveCount(263);

  const initial = await readViewBox(page);
  expect(initial.width).toBeLessThanOrEqual(READABLE_WIDTH);
  expect(initial.height).toBeLessThanOrEqual(READABLE_HEIGHT);

  const canvas = await page.getByTestId("family-tree-canvas").boundingBox();
  expect(canvas).toBeTruthy();
  if (!canvas) return;
  // A 76-unit card drawn into this viewport must stay a readable size.
  const cardHeight = (76 / initial.height) * canvas.height;
  expect(cardHeight).toBeGreaterThanOrEqual(30);

  await page.getByTestId("family-search").fill("L262");
  await page.getByTestId("family-search-results").getByRole("button").first().click();
  const selected = await readViewBox(page);
  expect(selected.width).toBeLessThanOrEqual(READABLE_WIDTH);
  expect(selected.height).toBeLessThanOrEqual(READABLE_HEIGHT);

  const card = await page.getByRole("button", { name: "Select L262", exact: true }).boundingBox();
  expect(card).toBeTruthy();
  if (!card) return;
  expect(card.width).toBeGreaterThanOrEqual(100);
  expect(card.y).toBeGreaterThanOrEqual(canvas.y - 1);
  expect(card.y + card.height).toBeLessThanOrEqual(canvas.y + canvas.height + 1);
});

// --- A catalog without relationships still renders readable cards ----------

const unlinked: TreeGraph = {
  people: Array.from({ length: 60 }, (_, index) => p("L" + String(index).padStart(2, "0"))),
  links: [],
  truncated: false,
};

async function stubUnlinked(page: import("@playwright/test").Page) {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(unlinked) }));
  await page.route("**/api/genealogy/person/**", (route) => {
    const id = decodeURIComponent(route.request().url().split("/").pop() || "");
    return route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ record_id: id, person_label: "Person " + id, assertions: [] }) });
  });
}

async function readViewBox(page: import("@playwright/test").Page) {
  const value = await page.getByTestId("family-tree-canvas").getAttribute("viewBox");
  const [x, y, width, height] = (value ?? "").split(" ").map(Number);
  return { x, y, width, height };
}

test("an unlinked catalog wraps components into bands instead of one long row", () => {
  const layout = layoutTree(unlinked, new Set());
  expect(layout.people).toHaveLength(60);
  // A single row of 60 cards would be ~15,000px wide and scale every card to
  // less than a pixel; the bands keep the canvas bounded (max band + one card).
  expect(layout.width).toBeLessThanOrEqual(1790);
  expect(layout.height).toBeGreaterThan(720);
  expect(new Set(layout.people.map((person) => person.y)).size).toBeGreaterThan(1);
  expect(layout.people.every((person) => person.x >= 0 && person.y >= 0)).toBe(true);
});

test("every authorized person renders without a relationship edge", async ({ page }) => {
  await stubUnlinked(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-person")).toHaveCount(60);
  await expect(page.getByTestId("genealogy-no-links")).toBeVisible();
  const view = await readViewBox(page);
  expect(view.width).toBeLessThanOrEqual(READABLE_WIDTH);
  expect(view.height).toBeLessThanOrEqual(720);
});

test("search selection reveals a readable card in an unlinked catalog", async ({ page }) => {
  await stubUnlinked(page);
  await page.goto("/genealogy");
  await page.getByTestId("family-search").fill("L59");
  await page.getByTestId("family-search-results").getByRole("button").first().click();
  await expect(page.getByTestId("family-person-details")).toContainText("L59");
  const view = await readViewBox(page);
  expect(view.width).toBeLessThanOrEqual(READABLE_WIDTH);
  expect(view.height).toBeLessThanOrEqual(720);
});
