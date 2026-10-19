import { defineConfig } from "@playwright/test";

// Real-browser verification for RUST-008A. The runner script
// (scripts/web-e2e.sh) starts the Axum API and the Next.js server before
// invoking this config; Playwright drives the real browser against them.
export default defineConfig({
  testDir: "./e2e",
  timeout: 180_000,
  expect: { timeout: 120_000 },
  fullyParallel: false,
  workers: 1,
  reporter: [["list"]],
  use: {
    baseURL: process.env.E2E_BASE_URL || "http://127.0.0.1:3000",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
});
