import { test, expect } from "@playwright/test";

// Proxy RBAC verification. These tests drive the Next.js server-side proxy and
// the Axum API directly to prove that neither path bypasses RBAC, that the
// proxy rejects a POST without a valid CSRF token, and that client identity
// headers never elevate permissions.
//
// The runner starts both servers (see playwright.config.ts and the web README).
// A verified bearer token can be supplied via E2E_VIEWER_TOKEN /
// E2E_ADMIN_TOKEN when the harness seeds the identity store. Without a token all
// protected operations must answer 401.

const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
const WEB_URL = (process.env.WEB_URL || "http://127.0.0.1:3000").replace(/\/+$/, "");

const VIEWER_TOKEN = process.env.E2E_VIEWER_TOKEN;
const ADMIN_TOKEN = process.env.E2E_ADMIN_TOKEN;

const PROTECTED = [
  { method: "POST", path: "/api/query", role: "viewer" },
  { method: "GET", path: "/api/documents/loan-policy.md/status", role: "viewer" },
  { method: "GET", path: "/api/account", role: "admin" },
] as const;

test.describe("health is public", () => {
  test("GET /api/health through the proxy needs no session", async ({ request }) => {
    const response = await request.get(`${WEB_URL}/api/health`);
    expect(response.status()).toBe(200);
    const body = await response.json();
    expect(body.service).toBe("rust-api");
    // No identity or document content is disclosed.
    expect(JSON.stringify(body)).not.toContain("subject");
    expect(JSON.stringify(body)).not.toContain("sources");
  });
});

test.describe("proxy rejects unauthenticated requests", () => {
  for (const route of PROTECTED) {
    test(`${route.method} ${route.path} without a token returns 401`, async ({
      request,
    }) => {
      const response = await request.fetch(`${WEB_URL}${route.path}`, {
        method: route.method,
        headers: { "x-csrf-token": "csrf", "content-type": "application/json" },
        data: route.method === "POST" ? JSON.stringify({ question: "hi" }) : undefined,
      });
      expect(response.status()).toBe(401);
    });
  }
});

test.describe("CSRF protection", () => {
  test("POST without a CSRF token is rejected", async ({ request }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    const response = await request.post(`${WEB_URL}/api/query`, {
      headers: {
        authorization: `Bearer ${VIEWER_TOKEN}`,
        "content-type": "application/json",
      },
      data: JSON.stringify({ question: "hi" }),
    });
    expect(response.status()).toBe(403);
    const body = await response.json();
    expect(body.error).toBe("csrf_failed");
  });
});

test.describe("client identity headers cannot elevate", () => {
  test("X-User/X-Role are stripped and do not grant access", async ({ request }) => {
    const response = await request.get(`${WEB_URL}/api/account`, {
      headers: { "x-user": "admin", "x-role": "admin" },
    });
    expect(response.status()).toBe(401);
  });

  test("direct Axum call with X-Role is still denied", async ({ request }) => {
    const response = await request.get(`${API_URL}/api/account`, {
      headers: { "x-user": "admin", "x-role": "admin" },
    });
    expect(response.status()).toBe(401);
  });
});

test.describe("role enforcement through both paths", () => {
  test("viewer token can query and is forbidden from admin", async ({ request }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    const query = await request.post(`${WEB_URL}/api/query`, {
      headers: {
        authorization: `Bearer ${VIEWER_TOKEN}`,
        "content-type": "application/json",
        cookie: "rag_csrf=e2e-csrf-token",
        "x-csrf-token": "e2e-csrf-token",
      },
      data: JSON.stringify({ question: "What happens if someone misses a payment?" }),
    });
    // 200 or a backend 5xx is fine; RBAC must not reject a viewer on query.
    expect([200, 503]).toContain(query.status());

    const admin = await request.get(`${WEB_URL}/api/account`, {
      headers: { authorization: `Bearer ${VIEWER_TOKEN}` },
    });
    expect(admin.status()).toBe(403);

    const direct = await request.get(`${API_URL}/api/account`, {
      headers: { authorization: `Bearer ${VIEWER_TOKEN}` },
    });
    expect(direct.status()).toBe(403);
  });

  test("admin token reaches the admin account endpoint", async ({ request }) => {
    test.skip(!ADMIN_TOKEN, "E2E_ADMIN_TOKEN not provided by the harness");
    const response = await request.get(`${WEB_URL}/api/account`, {
      headers: { authorization: `Bearer ${ADMIN_TOKEN}` },
    });
    expect(response.status()).toBe(200);
    const body = await response.json();
    expect(body.roles).toContain("admin");
  });
});
