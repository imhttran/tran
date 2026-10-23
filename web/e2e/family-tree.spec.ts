import { expect, test } from "@playwright/test";

import {
  focusedGraph, immediateChildIds, immediateParentIds, layoutTree, popupPosition,
  relationLabel, revealPerson, READABLE_HEIGHT, READABLE_WIDTH,
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
  await page.getByRole("button", { name: "Select Nguyễn Văn A", exact: true }).click();
  await expect(page.getByTestId("family-person-details")).toBeVisible();
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
  // The single click opens the popup; dismiss it so it cannot cover the button.
  await expect(page.getByTestId("family-person-details")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.getByTestId("family-collapse").click();
}

test("the popup lists recorded parents and children from the selection", async ({ page }) => {
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

// --- Person-centred focus, popup placement, and popup behaviour -------------

const family: TreeGraph = {
  people: [p("GP1"), p("GP2"), p("P1"), p("P2"), p("C1"), p("C2"), p("X")],
  links: [
    pc("GP1", "P1"),
    pc("GP2", "P1"),
    { ...pc("P1", "P2"), kind: "spouse" as const },
    pc("P1", "C1"),
    pc("P2", "C1"),
    pc("P1", "C2"),
  ],
  truncated: false,
};

test("recorded children and parents are exact, unique, and ID-keyed", () => {
  expect(immediateChildIds(family, "P1").sort()).toEqual(["C1", "C2"]);
  expect(immediateParentIds(family, "P1").sort()).toEqual(["GP1", "GP2"]);
  expect(immediateChildIds(family, "X")).toEqual([]);
  expect(immediateParentIds(family, "X")).toEqual([]);
});

test("the focused graph keeps only the immediate family", () => {
  const childrenOnly = focusedGraph(family, { rootId: "P1", showParents: false });
  expect(childrenOnly.people.map((person) => person.record_id).sort())
    .toEqual(["C1", "C2", "P1", "P2"]);
  expect(childrenOnly.links.map((link) => link.kind + ":" + link.from_id + ":" + link.to_id).sort())
    .toEqual(["parent_child:P1:C1", "parent_child:P1:C2", "parent_child:P2:C1", "spouse:P1:P2"]);

  const withParents = focusedGraph(family, { rootId: "P1", showParents: true });
  expect(withParents.people.map((person) => person.record_id).sort())
    .toEqual(["C1", "C2", "GP1", "GP2", "P1", "P2"]);
  expect(withParents.people.some((person) => person.record_id === "X")).toBe(false);
});

test("focused traversal terminates on cycles and never duplicates cards", () => {
  const cyclic: TreeGraph = {
    people: [p("A"), p("B"), p("C")],
    links: [pc("A", "B"), pc("B", "A"), pc("B", "C")],
    truncated: false,
  };
  const focused = focusedGraph(cyclic, { rootId: "B", showParents: true });
  expect(focused.people.map((person) => person.record_id).sort()).toEqual(["A", "B", "C"]);
  expect(new Set(focused.people.map((person) => person.record_id)).size).toBe(3);
  expect(focused.links).toHaveLength(3);

  const missing = focusedGraph(cyclic, { rootId: "missing", showParents: true });
  expect(missing.people).toEqual([]);
  expect(missing.links).toEqual([]);
});

test("a focused graph keeps the truncation disclosure", () => {
  const truncated: TreeGraph = { ...family, truncated: true };
  expect(focusedGraph(truncated, { rootId: "P1", showParents: false }).truncated).toBe(true);
});

test("the popup sits below the card, above when needed, and stays inside", () => {
  const container = { left: 0, top: 0, width: 800, height: 500 };
  const svg = { left: 0, top: 0, width: 800, height: 500 };
  const view = { x: 0, y: 0, width: 800, height: 500 };

  const beside = popupPosition({ x: 100, y: 100 }, view, svg, container, { width: 300, height: 200 });
  expect(beside?.placement).toBe("right");
  expect(beside?.left).toBeCloseTo(300, 0);
  expect(beside?.top).toBeCloseTo(100, 0);

  // A narrow canvas has no room beside the card, so it goes above it.
  const narrow = { left: 0, top: 0, width: 400, height: 500 };
  const above = popupPosition({ x: 100, y: 400 }, { x: 0, y: 0, width: 400, height: 500 },
    narrow, narrow, { width: 300, height: 200 });
  expect(above?.placement).toBe("above");
  expect(above?.top).toBeCloseTo(190, 0);

  const clamped = popupPosition({ x: 0, y: 0 }, { x: 0, y: 0, width: 400, height: 500 },
    narrow, narrow, { width: 300, height: 200 });
  expect(clamped?.left).toBe(12);

  // A card near the right edge flips to the left rather than clipping.
  const flipped = popupPosition({ x: 600, y: 100 }, view, svg, container,
    { width: 420, height: 200 });
  expect(flipped?.placement).toBe("left");

  const huge = popupPosition({ x: 100, y: 100 }, view, svg, container, { width: 5000, height: 5000 });
  expect(huge?.maxHeight).toBeLessThanOrEqual(480);
  expect(huge?.left).toBeGreaterThanOrEqual(10);
});

test("the popup placement accounts for letterboxing and hides off-screen anchors", () => {
  const placement = popupPosition(
    { x: 0, y: 0 },
    { x: 0, y: 0, width: 1000, height: 500 },
    { left: 0, top: 0, width: 900, height: 400 },
    { left: 0, top: 0, width: 900, height: 400 },
    { width: 300, height: 200 },
  );
  // scale = min(900/1000, 400/500) = 0.8; the content starts at x = 50, so the
  // card's right edge is 50 + 190*0.8 and the popup starts 10px further right.
  expect(placement?.left).toBe(212);

  const offScreen = popupPosition(
    { x: 900, y: 100 },
    { x: 0, y: 0, width: 800, height: 500 },
    { left: 0, top: 0, width: 800, height: 500 },
    { left: 0, top: 0, width: 800, height: 500 },
    { width: 300, height: 200 },
  );
  expect(offScreen).toBeNull();
});

test("single click opens an anchored popup and Escape restores card focus", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  const card = page.getByRole("button", { name: "Select Person A", exact: true });
  await card.click();
  const popup = page.getByTestId("family-person-details");
  await expect(popup).toBeVisible();
  await expect(popup).toContainText("Person A");
  const canvas = await page.getByTestId("family-canvas").boundingBox();
  const box = await popup.boundingBox();
  expect(canvas && box).toBeTruthy();
  if (canvas && box) {
    expect(box.x).toBeGreaterThanOrEqual(canvas.x - 1);
    expect(box.y).toBeGreaterThanOrEqual(canvas.y - 1);
    expect(box.x + box.width).toBeLessThanOrEqual(canvas.x + canvas.width + 1);
    expect(box.y + box.height).toBeLessThanOrEqual(canvas.y + canvas.height + 1);
  }
  await page.keyboard.press("Escape");
  await expect(popup).toBeHidden();
  await expect(card).toBeFocused();
});

test("double-click focuses immediate children above an unrelated graph", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-person")).toHaveCount(5);
  const before = await readViewBox(page);

  await page.getByRole("button", { name: "Select Person A", exact: true }).dblclick();
  await expect(page.getByTestId("family-focus")).toContainText("Person A");
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await expect(page.getByRole("button", { name: "Select Person A1", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Select Person R", exact: true })).toHaveCount(0);
  await expect(page.getByRole("button", { name: "Select Person B", exact: true })).toHaveCount(0);

  const root = await page.getByRole("button", { name: "Select Person A", exact: true }).boundingBox();
  const child = await page.getByRole("button", { name: "Select Person A1", exact: true }).boundingBox();
  expect(root && child && root.y < child.y).toBe(true);
  expect(child && child.width).toBeGreaterThanOrEqual(100);

  await page.getByTestId("family-back").click();
  await expect(page.getByTestId("family-focus")).toHaveCount(0);
  await expect(page.getByTestId("family-person")).toHaveCount(5);
  expect(await readViewBox(page)).toEqual(before);
});

test("Show parents reveals recorded parents above the person", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A1", exact: true }).click();
  await expect(page.getByTestId("family-show-parents")).toHaveText("Show parents (1)");
  await page.getByTestId("family-show-parents").click();
  await expect(page.getByTestId("family-focus")).toContainText("parents and children");
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await expect(page.getByRole("button", { name: "Select Person R", exact: true })).toHaveCount(0);

  const parent = await page.getByRole("button", { name: "Select Person A", exact: true }).boundingBox();
  const person = await page.getByRole("button", { name: "Select Person A1", exact: true }).boundingBox();
  expect(parent && person && parent.y < person.y).toBe(true);
});

test("Show children moves exactly one generation at a time", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person R", exact: true }).click();
  await expect(page.getByTestId("family-show-children")).toHaveText("Show children (2)");
  await page.getByTestId("family-show-children").click();
  await expect(page.getByTestId("family-focus")).toContainText("Person R");
  await expect(page.getByTestId("family-person")).toHaveCount(3);
  await expect(page.getByRole("button", { name: "Select Person A1", exact: true })).toHaveCount(0);

  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  await expect(
    page.getByTestId("family-person-details").getByRole("heading", { level: 2 }),
  ).toHaveText("Person A");
  await page.getByTestId("family-show-children").click();
  await expect(page.getByTestId("family-focus")).toContainText("Person A");
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await expect(page.getByRole("button", { name: "Select Person A1", exact: true })).toBeVisible();
});

test("a person with no recorded children keeps the view and says so", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-person")).toHaveCount(5);
  const before = await readViewBox(page);
  await page.getByRole("button", { name: "Select Person A1", exact: true }).dblclick();
  await expect(page.getByTestId("family-popup-notice")).toContainText("No children recorded");
  await expect(page.getByTestId("family-focus")).toHaveCount(0);
  expect(await readViewBox(page)).toEqual(before);
});

test("Back restores focus, expansion state, and the camera", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await collapse(page, "B");
  await expect(page.getByTestId("family-person")).toHaveCount(4);
  const before = await readViewBox(page);

  await page.getByRole("button", { name: "Select Person A", exact: true }).dblclick();
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await page.getByTestId("family-back").click();
  await expect(page.getByTestId("family-person")).toHaveCount(4);
  await expect(page.getByRole("button", { name: "Select Person B1", exact: true })).toHaveCount(0);
  expect(await readViewBox(page)).toEqual(before);
});

test("Show entire tree returns to the full graph", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).dblclick();
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await page.getByTestId("family-show-entire-tree").click();
  await expect(page.getByTestId("family-focus")).toHaveCount(0);
  await expect(page.getByTestId("family-person")).toHaveCount(5);
});

test("search leaves a focused view and reveals the requested person", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).dblclick();
  await expect(page.getByTestId("family-person")).toHaveCount(2);
  await page.getByTestId("family-search").fill("B1");
  await page.getByTestId("family-search-results").getByRole("button").first().click();
  await expect(page.getByTestId("family-focus")).toHaveCount(0);
  await expect(page.getByTestId("family-person")).toHaveCount(5);
  await expect(page.getByTestId("family-person-details")).toContainText("Person B1");
});

test("the popup follows a pan and closes when its anchor leaves the canvas", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  const popup = page.getByTestId("family-person-details");
  await expect(popup).toBeVisible();
  const canvas = await page.getByTestId("family-tree-canvas").boundingBox();
  expect(canvas).toBeTruthy();
  if (!canvas) return;
  const before = await popup.boundingBox();

  await page.mouse.move(canvas.x + 30, canvas.y + canvas.height - 30);
  await page.mouse.down();
  await page.mouse.move(canvas.x + 130, canvas.y + canvas.height - 130, { steps: 6 });
  await page.mouse.up();
  await expect(popup).toBeVisible();
  const after = await popup.boundingBox();
  expect(after && before && after.x !== before.x).toBe(true);

  await page.mouse.move(canvas.x + 30, canvas.y + canvas.height - 30);
  await page.mouse.down();
  await page.mouse.move(canvas.x + 30 + canvas.width * 3, canvas.y + canvas.height - 30, { steps: 12 });
  await page.mouse.up();
  await expect(popup).toBeHidden();
});

test("dragging a card is not a click and does not open the popup", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  const card = page.getByRole("button", { name: "Select Person A", exact: true });
  const box = await card.boundingBox();
  expect(box).toBeTruthy();
  if (!box) return;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width / 2 + 40, box.y + box.height / 2 + 40, { steps: 5 });
  await page.mouse.up();
  await page.waitForTimeout(SINGLE_CLICK_DELAY_WAIT_MS);
  await expect(page.getByTestId("family-person-details")).toHaveCount(0);
});

test("keyboard opens, navigates, and dismisses the popup", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  const card = page.getByRole("button", { name: "Select Person A", exact: true });
  await card.focus();
  await page.keyboard.press("Enter");
  const popup = page.getByTestId("family-person-details");
  await expect(popup).toBeVisible();
  await expect(popup).toBeFocused();

  await page.getByTestId("family-show-children").focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("family-focus")).toContainText("Person A");

  await page.keyboard.press("Escape");
  await expect(popup).toBeHidden();
  await expect(page.getByRole("button", { name: "Select Person A", exact: true })).toBeFocused();
});

test("a truncated graph discloses that relatives and counts may be incomplete", async ({ page }) => {
  await page.route("**/api/auth/session", (route) =>
    route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify(session) }));
  await page.route("**/api/genealogy/tree", (route) =>
    route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ ...branchGraph, truncated: true }) }));
  await page.route("**/api/genealogy/person/**", (route) => {
    const id = decodeURIComponent(route.request().url().split("/").pop() || "");
    return route.fulfill({ status: 200, contentType: "application/json",
      body: JSON.stringify({ record_id: id, person_label: "Person " + id, assertions: [] }) });
  });
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  await expect(page.getByTestId("family-popup-truncated")).toContainText("partial");
  await expect(page.getByTestId("family-show-children")).toHaveText("Show children (1)");
  await expect(page.getByRole("status").filter({ hasText: "counts and relatives may be incomplete" }))
    .toBeVisible();
});

test("the popup closes when the click is outside the tree workspace", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  await expect(page.getByTestId("family-person-details")).toBeVisible();
  await page.getByRole("heading", { name: "Family tree", level: 1 }).click();
  await expect(page.getByTestId("family-person-details")).toBeHidden();
});

test.describe("touch", () => {
  test.use({ hasTouch: true, viewport: { width: 390, height: 780 } });

  test("a tap opens the popup and the visible buttons navigate", async ({ page }) => {
    await stubBranches(page);
    await page.goto("/genealogy");
    await page.getByRole("button", { name: "Select Person A", exact: true }).tap();
    const popup = page.getByTestId("family-person-details");
    await expect(popup).toBeVisible();
    await page.getByTestId("family-show-children").tap();
    await expect(page.getByTestId("family-focus")).toContainText("Person A");
    await expect(page.getByTestId("family-person")).toHaveCount(2);
  });
});

const SINGLE_CLICK_DELAY_WAIT_MS = 500;

test("the popup keeps the family profile link", async ({ page }) => {
  await stubAuthorized(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Nguyễn Văn A", exact: true }).click();
  await expect(page.getByTestId("open-family-profile")).toHaveAttribute("href", "/family/root");
});

test("the popup stays in bounds when the canvas expands", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  const popup = page.getByTestId("family-person-details");
  await expect(popup).toBeVisible();

  await page.getByTestId("family-canvas-toggle").click();
  await expect(page.getByTestId("genealogy-tree")).toHaveAttribute("data-expanded", "true");
  await expect(popup).toBeVisible();
  const canvas = await page.getByTestId("family-canvas").boundingBox();
  const box = await popup.boundingBox();
  expect(canvas && box).toBeTruthy();
  if (canvas && box) {
    expect(box.x).toBeGreaterThanOrEqual(canvas.x - 1);
    expect(box.x + box.width).toBeLessThanOrEqual(canvas.x + canvas.width + 1);
    expect(box.y + box.height).toBeLessThanOrEqual(canvas.y + canvas.height + 1);
  }

  await page.keyboard.press("Escape");
  await expect(popup).toBeHidden();
  await expect(page.getByTestId("genealogy-tree")).toHaveAttribute("data-expanded", "true");
  await page.keyboard.press("Escape");
  await expect(page.getByTestId("genealogy-tree")).toHaveAttribute("data-expanded", "false");
});

// --- Contextual help ---------------------------------------------------------

test("the canvas shows the compact desktop navigation hint", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await expect(page.getByTestId("family-navigation-hint")).toHaveText(
    "Click for details. Double-click to explore children. Drag empty space to pan.",
  );
  await expect(page.getByTestId("family-help")).toBeVisible();
});

test("Help opens a dismissible guide with the implemented controls", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  const helpButton = page.getByTestId("family-help");
  await helpButton.focus();
  await page.keyboard.press("Enter");
  const panel = page.getByTestId("family-help-panel");
  await expect(panel).toBeVisible();
  await expect(panel).toBeFocused();
  for (const text of [
    "Click a person to open their details",
    "Show children",
    "Show parents",
    "Back",
    "Nguyen",
    "Fit all",
    "Focus selected",
    "Expand canvas",
    "Verified",
    "Needs review",
    "Unverified",
    "Tab moves between controls",
  ]) {
    await expect(panel).toContainText(text);
  }
  await expect(page.getByTestId("family-help-no-children")).toContainText("does not establish");
  await page.keyboard.press("Escape");
  await expect(panel).toBeHidden();
  await expect(helpButton).toBeFocused();

  // Dismissing does not remove the way back in.
  await helpButton.click();
  await expect(panel).toBeVisible();
  await page.getByTestId("family-help-close").click();
  await expect(panel).toBeHidden();
});

test.describe("help on a narrow screen", () => {
  test.use({ viewport: { width: 390, height: 780 } });

  test("the guide stays inside the viewport and is dismissible", async ({ page }) => {
    await stubBranches(page);
    await page.goto("/genealogy");
    await page.getByTestId("family-help").click();
    const panel = page.getByTestId("family-help-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText("No children recorded");
    const box = await panel.boundingBox();
    const viewport = page.viewportSize();
    expect(box && viewport).toBeTruthy();
    if (box && viewport) {
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(viewport.width + 1);
    }
    await page.getByTestId("family-help-close").click();
    await expect(panel).toBeHidden();
  });
});

test.describe("touch hint", () => {
  test.use({ hasTouch: true });

  test("uses tap wording instead of click and double-click", async ({ page }) => {
    await stubBranches(page);
    await page.goto("/genealogy");
    const hint = page.getByTestId("family-navigation-hint");
    await expect(hint).toHaveText(
      "Tap a person for details, then choose Show children or Show parents.",
    );
    await expect(hint).not.toContainText("Double-click");
  });
});

// --- Popup sizing, scrolling, and motion -------------------------------------

test("the popup is about 420px wide on desktop and animates opacity and transform", async ({ page }) => {
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  const popup = page.getByTestId("family-person-details");
  await expect(popup).toBeVisible();
  const box = await popup.boundingBox();
  expect(box).toBeTruthy();
  if (box) {
    expect(Math.round(box.width)).toBeGreaterThanOrEqual(419);
    expect(Math.round(box.width)).toBeLessThanOrEqual(421);
  }
  const motion = await popup.evaluate((element) => {
    const style = getComputedStyle(element as HTMLElement);
    return { duration: style.transitionDuration, properties: style.transitionProperty };
  });
  expect(motion.duration).toContain("0.18s");
  expect(motion.properties).toContain("opacity");
  expect(motion.properties).toContain("transform");
});

test.describe("popup on a narrow screen", () => {
  test.use({ viewport: { width: 390, height: 780 } });

  test("fits with 12px clearance and scrolls its content", async ({ page }) => {
    await stubBranches(page);
    const assertions = Array.from({ length: 40 }, (_, index) => ({
      assertion_id: 1000 + index,
      kind: "note",
      key: "note-" + index,
      value_text: "Synthetic note line " + index,
      review_status: "verified",
      evidence: [],
    }));
    await page.route("**/api/genealogy/person/**", (route) => {
      const id = decodeURIComponent(route.request().url().split("/").pop() || "");
      return route.fulfill({
        status: 200,
        contentType: "application/json",
        body: JSON.stringify({ record_id: id, person_label: "Person " + id, assertions }),
      });
    });
    await page.goto("/genealogy");
    await page.getByRole("button", { name: "Select Person A", exact: true }).click();
    const popup = page.getByTestId("family-person-details");
    await expect(popup).toBeVisible();

    const box = await popup.boundingBox();
    expect(box).toBeTruthy();
    if (box) {
      expect(box.x).toBeGreaterThanOrEqual(12);
      expect(box.x + box.width).toBeLessThanOrEqual(390 - 12 + 0.5);
    }
    const overflow = await popup.evaluate(
      (element) => getComputedStyle(element as HTMLElement).overflowY,
    );
    expect(["auto", "scroll"]).toContain(overflow);
    expect(await popup.evaluate((element) => element.scrollHeight > element.clientHeight))
      .toBe(true);
    await popup.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    expect(await popup.evaluate((element) => element.scrollTop)).toBeGreaterThan(0);
  });
});

test("reduced motion removes the fade and slide and still dismisses instantly", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await stubBranches(page);
  await page.goto("/genealogy");
  await page.getByRole("button", { name: "Select Person A", exact: true }).click();
  const popup = page.getByTestId("family-person-details");
  await expect(popup).toBeVisible();
  const motion = await popup.evaluate((element) => {
    const style = getComputedStyle(element as HTMLElement);
    return { duration: style.transitionDuration, transform: style.transform };
  });
  expect(motion.duration).toBe("0s");
  expect(motion.transform).toBe("none");
  await page.keyboard.press("Escape");
  await expect(popup).toHaveCount(0);
});
