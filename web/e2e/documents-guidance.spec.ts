import { expect, test } from "@playwright/test";
import type { Page } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  DOCUMENT_UPLOAD_ACCEPT,
  DOCUMENT_UPLOAD_MAX_BYTES,
  EXAMPLE_FAMILY_STORY,
  documentUploadLimitLabel,
} from "../src/lib/document-upload";

// First-use guidance on the family documents upload page.
//
// The suite uses the harness's synthetic admin token, so it exercises the real
// Next.js page and the real submission form. It never uploads or reviews a real
// document.

const ORIGIN = (process.env.WEB_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");
const ADMIN = process.env.E2E_ADMIN_TOKEN ?? "";

// The guidance repeats the server's accepted formats and size limit, so the
// duplication is checked against the Rust source instead of being trusted. This
// needs no browser and no token; it runs on every suite invocation.
test("the guidance matches the server document contract", () => {
  const source = readFileSync(
    join(process.cwd(), "..", "src", "document_lifecycle.rs"),
    "utf8",
  );

  const limit = /pub const MAX_DOCUMENT_BYTES: usize = ([^;]+);/.exec(source);
  expect(limit, "MAX_DOCUMENT_BYTES not found").not.toBeNull();
  const bytes = limit![1]
    .split("*")
    .map((part) => Number(part.trim()))
    .reduce((total, part) => total * part, 1);
  expect(bytes).toBe(DOCUMENT_UPLOAD_MAX_BYTES);

  const list = /pub const DOCUMENT_EXTENSIONS: &\[&str\] = &\[([^\]]*)\]/.exec(source);
  expect(list, "DOCUMENT_EXTENSIONS not found").not.toBeNull();
  const serverFormats = [...list![1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
  const advertised = DOCUMENT_UPLOAD_ACCEPT.split(",").map((extension) =>
    extension.trim().replace(/^\./, ""),
  );
  expect(advertised).toEqual(serverFormats);
});

async function useAdmin(page: Page) {
  await page.context().addCookies([
    {
      name: "rag_session",
      value: ADMIN,
      url: ORIGIN,
      httpOnly: true,
      sameSite: "Lax",
    },
    {
      name: "rag_csrf",
      value: "e2e-csrf-token",
      url: ORIGIN,
      sameSite: "Lax",
    },
  ]);
}

test.describe("family documents first-use guidance", () => {
  test.skip(!ADMIN, "E2E_ADMIN_TOKEN not provided by the harness");

  test("explains what can be uploaded and the real limits", async ({ page }) => {
    await useAdmin(page);
    await page.goto("/documents");

    const guidance = page.getByTestId("documents-guidance");
    await expect(guidance).toBeVisible();
    await expect(
      guidance.getByRole("heading", { name: "What can I upload?" }),
    ).toBeVisible();

    await expect(guidance).toContainText("Family stories and personal memories");
    await expect(guidance).toContainText("parents, grandparents, and other relatives");
    await expect(guidance).toContainText("Letters, biographies, and historical records");
    await expect(guidance).toContainText("Vietnamese and English");
    await expect(guidance).toContainText("No special writing style is required");

    await expect(guidance).toContainText(".md");
    await expect(guidance).toContainText(".markdown");
    await expect(guidance).toContainText(".txt");
    await expect(guidance).toContainText(".pdf");
    // Markdown is section-based on the server; the guidance must say so.
    await expect(page.getByTestId("markdown-sections-note")).toContainText(
      "single passage",
    );
    await expect(page.getByTestId("markdown-sections-note")).toContainText(
      "heading",
    );
    // Extraction is reviewed separately and never verifies a fact.
    await expect(page.getByTestId("guidance-records")).toContainText(
      "searchable name",
    );
    await expect(page.getByTestId("guidance-records")).toContainText(
      "not a link",
    );
    // The advertised limit comes from the same constant the form uses.
    await expect(guidance).toContainText(documentUploadLimitLabel());
    await expect(guidance).toContainText("OCR");
    await expect(page.getByTestId("document-file")).toHaveAttribute(
      "accept",
      DOCUMENT_UPLOAD_ACCEPT,
    );

    await expect(guidance).toContainText("Use full names when possible");
    await expect(guidance).toContainText("Explain relationships clearly");
    await expect(guidance).toContainText("Identify who supplied the information");
    await expect(guidance).toContainText("Mark approximate dates and uncertain details");
    await expect(guidance).toContainText("Choose visibility carefully before submitting");

    const workflow = page.getByTestId("documents-workflow");
    await expect(workflow).toContainText("Submit");
    await expect(workflow).toContainText("Administrator review");
    await expect(workflow).toContainText("Processing");
    await expect(workflow).toContainText("Available to Ask the archive");
    await expect(guidance).toContainText(
      "does not create or update family-tree records",
    );

    // Guidance never blocks the form.
    await expect(page.getByTestId("document-upload")).toBeVisible();
    await expect(page.getByTestId("document-file")).toBeVisible();
    await expect(page.getByTestId("submit-document")).toBeVisible();
  });

  test("does not advertise unsupported image or word-processing uploads", async ({
    page,
  }) => {
    await useAdmin(page);
    await page.goto("/documents");
    const text = (
      await page.getByTestId("documents-guidance").innerText()
    ).toLowerCase();
    for (const unsupported of [
      "word",
      ".doc",
      ".docx",
      "image",
      ".jpg",
      ".jpeg",
      ".png",
    ]) {
      expect(text, `guidance must not advertise ${unsupported}`).not.toContain(
        unsupported,
      );
    }

    // Stronger than the substring sweep above: every extension the guidance
    // mentions must be one the server accepts, so a newly advertised format
    // cannot slip through.
    const supported = new Set(
      DOCUMENT_UPLOAD_ACCEPT.split(",").map((extension) =>
        extension.trim().toLowerCase(),
      ),
    );
    const mentioned = new Set(text.match(/\.[a-z][a-z0-9]{0,9}\b/g) ?? []);
    expect(mentioned.size, "guidance mentions no file extensions").toBeGreaterThan(0);
    expect([...mentioned].sort()).toEqual(
      [...mentioned].filter((extension) => supported.has(extension)).sort(),
    );
  });

  test("the fictional example expands by keyboard and is copyable", async ({
    page,
  }) => {
    await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
    await useAdmin(page);
    await page.goto("/documents");

    const details = page.getByTestId("example-story");
    await expect(details).toContainText("fictional");
    const summary = details.locator("summary");
    await summary.focus();
    await page.keyboard.press("Enter");
    await expect(page.getByTestId("example-story-text")).toBeVisible();

    const example = await page.getByTestId("example-story-text").innerText();
    expect(example).toContain("Person: Nguyễn Văn An");
    expect(example).toContain("Uncertainty:");
    expect(example.trim()).toBe(EXAMPLE_FAMILY_STORY.trim());

    await page.getByTestId("copy-example").click();
    await expect(page.getByTestId("copy-example-feedback")).toHaveText(
      /Example copied to the clipboard/i,
    );
    const clipboard = await page.evaluate(() => navigator.clipboard.readText());
    expect(clipboard.trim()).toBe(EXAMPLE_FAMILY_STORY.trim());
  });

  test("reads clearly at a 390 px viewport", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await useAdmin(page);
    await page.goto("/documents");

    const guidance = page.getByTestId("documents-guidance");
    await expect(guidance).toBeVisible();
    await page.getByTestId("example-story").locator("summary").click();
    await expect(page.getByTestId("example-story-text")).toBeVisible();

    const overflow = await guidance.evaluate(
      (element) => element.scrollWidth - element.clientWidth,
    );
    expect(overflow, "the guidance must not scroll horizontally").toBeLessThanOrEqual(1);

    await expect(page.getByTestId("document-upload")).toBeVisible();
    await expect(page.getByTestId("submit-document")).toBeVisible();
  });
});
