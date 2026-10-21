import { expect, test } from "@playwright/test";
import type { APIRequestContext } from "@playwright/test";

// USR-005B branch-scoped shared-genealogy review.
//
// These are API-level checks against the real Axum API and disposable
// PostgreSQL data seeded by scripts/seed-branch-review-e2e.sql. They prove the
// server-side authorization boundary: an explicit branch grant, branch scoping,
// independent decisions, self/owner denial, and revoked-link behavior. The
// /branch-review interface arrives in USR-005C and the full 16-scenario
// security matrix in USR-005D.

const API_URL = process.env.E2E_API_URL || "http://127.0.0.1:8080";
const ISSUER = process.env.E2E_ISSUER || "";
const ADMIN = process.env.E2E_ADMIN_TOKEN || "";
const APPROVER = process.env.E2E_APPROVER_TOKEN || "";
const REVIEWER = process.env.E2E_REVIEWER_TOKEN || "";
const REVIEWER_ACCOUNT_ID = Number(process.env.E2E_REVIEWER_ACCOUNT_ID || 0);
const APPROVER_ACCOUNT_ID = Number(process.env.E2E_APPROVER_ACCOUNT_ID || 0);
const READY = Boolean(
  ISSUER &&
  ADMIN &&
  APPROVER &&
  REVIEWER &&
  REVIEWER_ACCOUNT_ID &&
  APPROVER_ACCOUNT_ID,
);

type Grant = {
  grant_id: number;
  account_id: number;
  branch_key: string;
  active: boolean;
  usable: boolean;
};

type Evidence = {
  source_evidence_id: number;
  document_ref: string;
  page: number;
};

type Proposal = {
  proposal_id: number;
  person_key: string;
  branch_key: string;
  assertion_kind: string;
  assertion_key: string;
  prior_value_text: string | null;
  proposed_value_text: string;
  evidence: Evidence[];
  status: string;
  can_decide: boolean;
  decision_block: string | null;
};

type ProposalEvent = {
  event: string;
  actor: string;
  note: string | null;
  created_at: string;
};

// Unique per run so a re-run never collides with the partial unique index that
// allows only one open proposal per fact.
const RUN = `${Date.now().toString(36)}-${Math.floor(Math.random() * 1e6).toString(36)}`;

function auth(token: string) {
  return { authorization: `Bearer ${token}` };
}

async function api(
  request: APIRequestContext,
  token: string,
  method: "GET" | "POST",
  path: string,
  data?: unknown,
) {
  const response = await request.fetch(`${API_URL}${path}`, {
    method,
    headers: auth(token),
    data,
  });
  const body = await response.json().catch(() => null);
  return { status: response.status(), body };
}

async function grantsFor(
  request: APIRequestContext,
  accountId: number,
): Promise<Grant[]> {
  const listed = await api(
    request,
    ADMIN,
    "GET",
    `/api/admin/branch-reviewers?account_id=${accountId}`,
  );
  expect(listed.status, JSON.stringify(listed.body)).toBe(200);
  return listed.body.grants as Grant[];
}

// Revoking first keeps the fixture deterministic across re-runs, because the
// grant table allows only one active grant per account and branch.
async function replaceGrants(
  request: APIRequestContext,
  accountId: number,
  subject: string,
  branchKeys: string[],
) {
  for (const grant of await grantsFor(request, accountId)) {
    if (!grant.active) continue;
    const revoked = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/branch-reviewers/${grant.grant_id}/revoke`,
      { reason: "E2E reset before granting" },
    );
    expect(revoked.status, JSON.stringify(revoked.body)).toBe(200);
  }
  if (branchKeys.length === 0) return [];
  const granted = await api(
    request,
    ADMIN,
    "POST",
    "/api/admin/branch-reviewers",
    {
      issuer: ISSUER,
      subject,
      branch_keys: branchKeys,
      reason: "Synthetic E2E branch review appointment",
    },
  );
  expect(granted.status, JSON.stringify(granted.body)).toBe(200);
  return granted.body.grants as Grant[];
}

async function branchEvidence(
  request: APIRequestContext,
  token: string,
  personKey: string,
) {
  const response = await api(
    request,
    token,
    "GET",
    `/api/branch-review/persons/${personKey}/evidence`,
  );
  expect(response.status, JSON.stringify(response.body)).toBe(200);
  return response.body.evidence as Evidence[];
}

async function propose(
  request: APIRequestContext,
  token: string,
  options: {
    personKey: string;
    branchKey: string;
    assertionKey: string;
    value: string;
    // A caller that is expected to be denied sends a placeholder reference so
    // the assertion exercises authorization rather than input validation.
    evidence?: Evidence[];
  },
) {
  const evidence =
    options.evidence ??
    (await branchEvidence(request, token, options.personKey));
  expect(evidence.length).toBeGreaterThan(0);
  return api(request, token, "POST", "/api/branch-review/submissions", {
    person_key: options.personKey,
    branch_key: options.branchKey,
    assertion_kind: "name",
    assertion_key: options.assertionKey,
    proposed_value: options.value,
    evidence: [evidence[0]],
    note: "Synthetic E2E shared-fact proposal",
  });
}

test.beforeEach(() => {
  test.skip(!READY, "run this suite through scripts/web-e2e.sh");
});

test.describe.serial("branch-scoped shared-fact review", () => {
  test("grants and queue reads stay inside the reviewer's branches", async ({
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", []);

    const mine = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(mine.status).toBe(200);
    expect(
      (mine.body.grants as { branch_key: string }[]).map(
        (grant) => grant.branch_key,
      ),
    ).toEqual(["branch-1"]);

    // The approver is a verified, linked account with no branch grant at all.
    const none = await api(
      request,
      APPROVER,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(none.status).toBe(200);
    expect(none.body.grants).toEqual([]);

    const inScope = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/submissions?branch=branch-1",
    );
    expect(inScope.status).toBe(200);
    expect(
      (inScope.body.submissions as Proposal[]).every(
        (row) => row.branch_key === "branch-1",
      ),
    ).toBeTruthy();

    const outOfScope = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/submissions?branch=branch-2",
    );
    expect(outOfScope.status).toBe(403);
    expect(outOfScope.body.error).toBe("branch_review_forbidden");

    const noGrantQueue = await api(
      request,
      APPROVER,
      "GET",
      "/api/branch-review/submissions",
    );
    expect(noGrantQueue.status).toBe(200);
    expect(noGrantQueue.body.submissions).toEqual([]);
  });

  test("a linked owner proposes and an independent reviewer approves", async ({
    request,
  }) => {
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);

    const assertionKey = `name.preferred.${RUN}`;
    const created = await propose(request, APPROVER, {
      personKey: "e2e-branch-subject",
      branchKey: "branch-1",
      assertionKey,
      value: "Synthetic Approved Branch Name",
    });
    expect(created.status, JSON.stringify(created.body)).toBe(200);
    const proposalId = created.body.proposal_id as number;

    const queue = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/submissions?branch=branch-1",
    );
    expect(queue.status).toBe(200);
    const queued = (queue.body.submissions as Proposal[]).find(
      (row) => row.proposal_id === proposalId,
    );
    expect(
      queued,
      "the pending proposal is queued for the reviewer",
    ).toBeTruthy();
    expect(queued!.status).toBe("pending_review");
    expect(queued!.can_decide).toBe(true);
    expect(queued!.decision_block).toBeNull();
    expect(queued!.evidence.length).toBeGreaterThan(0);

    const detail = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${proposalId}`,
    );
    expect(detail.status).toBe(200);
    expect(detail.body.current_assertion).toBeNull();
    expect(
      (detail.body.events as ProposalEvent[]).map((event) => event.event),
    ).toEqual(["submitted"]);

    // The submitter cannot decide its own proposal, even with a grant.
    const selfDecision = await api(
      request,
      APPROVER,
      "POST",
      `/api/branch-review/submissions/${proposalId}/decision`,
      { decision: "approve" },
    );
    expect(selfDecision.status).toBe(403);
    expect(selfDecision.body.error).toBe("branch_review_forbidden");

    const approved = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${proposalId}/decision`,
      { decision: "approve", note: "Matches the synthetic source page." },
    );
    expect(approved.status, JSON.stringify(approved.body)).toBe(200);
    expect(approved.body.status).toBe("approved");
    expect(typeof approved.body.applied_assertion_id).toBe("number");

    const after = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${proposalId}`,
    );
    expect(after.status).toBe(200);
    expect(after.body.proposal.status).toBe("approved");
    expect(after.body.current_assertion.value_text).toBe(
      "Synthetic Approved Branch Name",
    );
    expect(after.body.current_assertion.review_status).toBe("verified");
    expect(
      (after.body.events as ProposalEvent[]).map((event) => event.event),
    ).toEqual(["submitted", "approved"]);

    // A decided proposal cannot be decided again.
    const repeat = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${proposalId}/decision`,
      { decision: "approve" },
    );
    expect(repeat.status).toBe(409);
    expect(repeat.body.error).toBe("branch_review_conflict");
  });

  test("rejection and change requests need a note and leave the fact unchanged", async ({
    request,
  }) => {
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);

    const rejectedId = (
      await propose(request, APPROVER, {
        personKey: "e2e-branch-subject",
        branchKey: "branch-1",
        assertionKey: `name.rejected.${RUN}`,
        value: "Unsupported Synthetic Name",
      })
    ).body.proposal_id as number;

    const silent = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${rejectedId}/decision`,
      { decision: "reject" },
    );
    expect(silent.status).toBe(422);
    expect(silent.body.error).toBe("branch_review_invalid");

    const rejected = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${rejectedId}/decision`,
      { decision: "reject", note: "The cited page does not name this person." },
    );
    expect(rejected.status, JSON.stringify(rejected.body)).toBe(200);
    expect(rejected.body.status).toBe("rejected");
    expect(rejected.body.applied_assertion_id).toBeNull();

    const rejectedDetail = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${rejectedId}`,
    );
    expect(rejectedDetail.body.current_assertion).toBeNull();
    expect((rejectedDetail.body.events as ProposalEvent[]).at(-1)?.note).toBe(
      "The cited page does not name this person.",
    );

    const changesId = (
      await propose(request, APPROVER, {
        personKey: "e2e-branch-subject",
        branchKey: "branch-1",
        assertionKey: `name.needs-changes.${RUN}`,
        value: "Incomplete Synthetic Name",
      })
    ).body.proposal_id as number;

    const changes = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${changesId}/decision`,
      { decision: "needs_changes", note: "Cite the second page as well." },
    );
    expect(changes.status, JSON.stringify(changes.body)).toBe(200);
    expect(changes.body.status).toBe("needs_changes");

    const rejectedQueue = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/submissions?status=rejected",
    );
    expect(rejectedQueue.status).toBe(200);
    expect(
      (rejectedQueue.body.submissions as Proposal[]).some(
        (row) => row.proposal_id === rejectedId,
      ),
    ).toBeTruthy();

    // Decided proposals are no longer pending, so the default queue omits them.
    const pendingQueue = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/submissions?status=pending",
    );
    expect(
      (pendingQueue.body.submissions as Proposal[]).some(
        (row) => row.proposal_id === rejectedId,
      ),
    ).toBeFalsy();
  });

  test("cross-branch and linked-owner decisions are denied", async ({
    request,
  }) => {
    await replaceGrants(request, APPROVER_ACCOUNT_ID, "e2e-approver", [
      "branch-1",
    ]);
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);

    // A branch-2 person is invisible to a branch-1 reviewer.
    const otherEvidence = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/persons/e2e-branch-other/evidence",
    );
    expect(otherEvidence.status).toBe(403);

    const crossBranch = await propose(request, APPROVER, {
      personKey: "e2e-branch-other",
      branchKey: "branch-2",
      assertionKey: `name.cross.${RUN}`,
      value: "Synthetic Cross Branch Name",
      evidence: [{ source_evidence_id: 1, document_ref: "x", page: 1 }],
    });
    expect(crossBranch.status).toBe(403);

    // The reviewer is linked to e2e-branch-owner, so they cannot decide a
    // proposal about their own person even though they hold the branch grant.
    const ownedId = (
      await propose(request, APPROVER, {
        personKey: "e2e-branch-owner",
        branchKey: "branch-1",
        assertionKey: `name.owned.${RUN}`,
        value: "Synthetic Owned Person Name",
      })
    ).body.proposal_id as number;

    const ownerDetail = await api(
      request,
      REVIEWER,
      "GET",
      `/api/branch-review/submissions/${ownedId}`,
    );
    expect(ownerDetail.status).toBe(200);
    expect(ownerDetail.body.proposal.can_decide).toBe(false);
    expect(ownerDetail.body.proposal.decision_block).toBe(
      "reviewer_owns_person",
    );

    const ownerDecision = await api(
      request,
      REVIEWER,
      "POST",
      `/api/branch-review/submissions/${ownedId}/decision`,
      { decision: "approve" },
    );
    expect(ownerDecision.status).toBe(403);
    expect(ownerDecision.body.error).toBe("branch_review_forbidden");

    // An unlinked account sees no review data at all.
    const anonymous = await api(
      request,
      "",
      "GET",
      "/api/branch-review/submissions",
    );
    expect([401, 403]).toContain(anonymous.status);
  });

  test("revoking the reviewed account link immediately blocks review access", async ({
    request,
  }) => {
    await replaceGrants(request, REVIEWER_ACCOUNT_ID, "e2e-reviewer", [
      "branch-1",
    ]);

    const before = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(before.status).toBe(200);
    expect(before.body.grants).toHaveLength(1);

    const directory = await api(
      request,
      ADMIN,
      "GET",
      "/api/admin/family-members?q=e2e-reviewer",
    );
    expect(directory.status).toBe(200);
    const linked = (
      directory.body.members as {
        linked_person: { link_id: number; person_key: string } | null;
      }[]
    ).find((member) => member.linked_person);
    expect(linked?.linked_person?.person_key).toBe("e2e-branch-owner");

    const revoked = await api(
      request,
      ADMIN,
      "POST",
      `/api/admin/family-members/links/${linked!.linked_person!.link_id}/revoke`,
    );
    expect(revoked.status, JSON.stringify(revoked.body)).toBe(200);

    // The grant row survives, but it is unusable without the reviewed link.
    const after = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(after.status).toBe(200);
    expect(after.body.grants).toEqual([]);

    const queue = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/submissions",
    );
    expect(queue.status).toBe(200);
    expect(queue.body.submissions).toEqual([]);

    const denied = await api(
      request,
      REVIEWER,
      "POST",
      "/api/branch-review/submissions",
      {
        person_key: "e2e-branch-subject",
        branch_key: "branch-1",
        assertion_kind: "name",
        assertion_key: `name.unlinked.${RUN}`,
        proposed_value: "Synthetic Unlinked Name",
        evidence: [{ source_evidence_id: 1, document_ref: "x", page: 1 }],
      },
    );
    expect(denied.status).toBe(403);

    // Restore the reviewed link so the fixture is reusable.
    const relinked = await api(
      request,
      ADMIN,
      "POST",
      "/api/admin/family-members/links",
      {
        person_key: "e2e-branch-owner",
        issuer: ISSUER,
        subject: "e2e-reviewer",
        decision: "approve",
      },
    );
    expect(relinked.status, JSON.stringify(relinked.body)).toBe(200);

    const restored = await api(
      request,
      REVIEWER,
      "GET",
      "/api/branch-review/grants/me",
    );
    expect(restored.status).toBe(200);
    expect(restored.body.grants).toHaveLength(1);
  });
});
