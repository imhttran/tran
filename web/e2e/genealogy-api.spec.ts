import { test, expect } from "@playwright/test";

// Real genealogy API integration against the disposable genealogy database. The
// synthetic person is seeded by scripts/seed-genealogy-e2e.sql. No UI mocking.
const API_URL = (process.env.API_URL || "http://127.0.0.1:8080").replace(/\/+$/, "");
const ADMIN_TOKEN = process.env.E2E_ADMIN_TOKEN;
const VIEWER_TOKEN = process.env.E2E_VIEWER_TOKEN;

const auth = (token: string) => ({ authorization: `Bearer ${token}` });

test.describe.serial("genealogy API integration", () => {
  test("admin reads synthetic records, nominates, approves, and revokes", async ({
    request,
  }) => {
    test.skip(!ADMIN_TOKEN, "E2E_ADMIN_TOKEN not provided by the harness");

    const records = await request.get(`${API_URL}/api/genealogy/records`, {
      headers: auth(ADMIN_TOKEN!),
    });
    expect(records.status()).toBe(200);
    const recordBody = (await records.json()) as {
      records: Array<{ record_id: string; person_label: string; evidence_status: string }>;
    };
    const synthetic = recordBody.records.find(
      (record) => record.record_id === "synthetic-e2e-person",
    );
    expect(synthetic).toBeTruthy();
    expect(synthetic?.evidence_status).toBe("verified");

    const tree = await request.get(API_URL + "/api/genealogy/tree", {
      headers: auth(ADMIN_TOKEN!),
    });
    expect(tree.status()).toBe(200);
    const treeBody = (await tree.json()) as {
      people: Array<{ record_id: string; person_label: string;
        evidence_status: string; aliases: string[] }>;
      links: unknown[];
      truncated: boolean;
    };
    expect(treeBody.people.some((p) => p.record_id === "synthetic-e2e-person")).toBeTruthy();
    expect(treeBody.people.every((p) => Array.isArray(p.aliases))).toBeTruthy();
    expect(Array.isArray(treeBody.links)).toBeTruthy();
    expect(typeof treeBody.truncated).toBe("boolean");

    const nomination = await request.post(`${API_URL}/api/genealogy/nominations`, {
      headers: auth(ADMIN_TOKEN!),
      data: { record_id: "synthetic-e2e-person" },
    });
    expect(nomination.status()).toBe(200);
    const nominationBody = (await nomination.json()) as { nomination_id: number };

    const approval = await request.post(
      `${API_URL}/api/genealogy/nominations/${nominationBody.nomination_id}`,
      { headers: auth(ADMIN_TOKEN!), data: { approve: true } },
    );
    expect(approval.status()).toBe(200);
    const approvalBody = (await approval.json()) as { appointment_id: number };

    const head = await request.get(`${API_URL}/api/genealogy/head-of-family`, {
      headers: auth(ADMIN_TOKEN!),
    });
    expect(head.status()).toBe(200);
    const headBody = (await head.json()) as {
      conclusions: Array<{ head_label: string; evidence_status: string }>;
    };
    expect(
      headBody.conclusions.some(
        (conclusion) =>
          conclusion.head_label === "Synthetic E2E Person" &&
          conclusion.evidence_status === "verified",
      ),
    ).toBeTruthy();

    const revoked = await request.post(
      `${API_URL}/api/genealogy/appointments/${approvalBody.appointment_id}`,
      { headers: auth(ADMIN_TOKEN!), data: { reason: "synthetic test revocation" } },
    );
    expect(revoked.status()).toBe(200);

    const after = await request.get(`${API_URL}/api/genealogy/head-of-family`, {
      headers: auth(ADMIN_TOKEN!),
    });
    const afterBody = (await after.json()) as { conclusions: unknown[] };
    expect(afterBody.conclusions.length).toBe(0);
  });

  test("person citations carry document and page, directly and through the proxy", async ({
    request,
  }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    const base = process.env.E2E_BASE_URL || "http://127.0.0.1:3000";
    type Person = { assertions: Array<{ evidence: Array<{ document_ref?: string; page?: number }> }> };
    const reads = [
      request.get(`${API_URL}/api/genealogy/person/synthetic-e2e-person`, { headers: auth(VIEWER_TOKEN!) }),
      request.get(`${base}/api/genealogy/person/synthetic-e2e-person`,
        { headers: { cookie: `rag_session=${VIEWER_TOKEN}` } }),
    ];
    for (const response of await Promise.all(reads)) {
      expect(response.status()).toBe(200);
      const evidence = ((await response.json()) as Person).assertions.flatMap((a) => a.evidence);
      expect(evidence).toContainEqual(expect.objectContaining({ document_ref: "synthetic/e2e.pdf", page: 1 }));
    }
    // The same source metadata stays denied without a session on either path.
    for (const url of [API_URL, base]) {
      const anonymous = await request.get(`${url}/api/genealogy/person/synthetic-e2e-person`);
      expect(anonymous.status()).toBe(401);
    }
  });

  test("viewer cannot nominate and anonymous reads are denied", async ({ request }) => {
    test.skip(!VIEWER_TOKEN, "E2E_VIEWER_TOKEN not provided by the harness");
    const nomination = await request.post(`${API_URL}/api/genealogy/nominations`, {
      headers: auth(VIEWER_TOKEN!),
      data: { record_id: "synthetic-e2e-person" },
    });
    expect(nomination.status()).toBe(403);

    const anonymous = await request.get(`${API_URL}/api/genealogy/records`);
    expect(anonymous.status()).toBe(401);
  });
});
