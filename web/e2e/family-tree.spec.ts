import { expect, test } from "@playwright/test";

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
