"use client";

import type { BranchBlock, BranchClaim } from "@/lib/api";

// Shared rendering for the additive shared-genealogy branch block.
//
// The Rust projection decides the state and the resolved branch. This component
// only presents what the payload states, in words as well as text classes, and
// never invents a branch from an unresolved claim.

export function branchStateText(block: BranchBlock): string {
  switch (block.state) {
    case "missing":
      return "No branch recorded";
    case "unverified":
      return "Recorded but not verified";
    case "conflicting":
      return "The recorded branches disagree";
    case "verified":
      return "Verified recorded branch";
    default:
      return "Branch state unavailable";
  }
}

function claimStatusText(claim: BranchClaim): string {
  if (claim.verified) return "verified";
  if (claim.review_status === "draft") return "draft";
  if (claim.review_status === "needs_review") return "needs review";
  return claim.review_status === "unverified"
    ? "not verified"
    : claim.review_status.replace("_", " ");
}

/// The resolved label for the single verified branch, or `null`.
function resolvedLabel(block: BranchBlock): string | null {
  if (block.state !== "verified" || !block.verified_branch_key) return null;
  const match = block.claims.find(
    (claim) =>
      claim.verified && claim.branch_key === block.verified_branch_key,
  );
  return match?.branch_label ?? null;
}

export function BranchSection({
  branches,
  testIdPrefix,
}: {
  branches: BranchBlock | undefined;
  testIdPrefix: string;
}) {
  // A payload without a branch block is ignored entirely, so an older server
  // renders no section and no placeholder.
  if (!branches) return null;

  const label = resolvedLabel(branches);
  const showResolved = label !== null && branches.verified_branch_key !== null;

  return (
    <section
      className="branch-section"
      aria-labelledby={`${testIdPrefix}-branch-heading`}
      data-testid={`${testIdPrefix}-branch`}
      data-branch-state={branches.state}
    >
      <h2
        id={`${testIdPrefix}-branch-heading`}
        className="branch-section-heading"
      >
        Recorded branch
      </h2>
      <p
        className={`branch-state branch-state-${branches.state}`}
        data-testid={`${testIdPrefix}-branch-state`}
      >
        {branchStateText(branches)}
      </p>
      {branches.claims.length > 0 ? (
        <ul className="branch-claims" data-testid={`${testIdPrefix}-branch-claims`}>
          {branches.claims.map((claim, index) => (
            <li
              key={`${claim.assertion_kind}:${claim.value_text ?? ""}:${index}`}
              className="branch-claim"
              data-testid={`${testIdPrefix}-branch-claim`}
              data-verified={claim.verified ? "true" : "false"}
            >
              <span className="branch-claim-value">
                {claim.value_text ?? "(no value recorded)"}
              </span>
              <span className="branch-claim-status">
                {" — "}
                {claimStatusText(claim)}
              </span>
            </li>
          ))}
        </ul>
      ) : null}
      {showResolved ? (
        <p className="branch-resolved" data-testid={`${testIdPrefix}-branch-resolved`}>
          <span className="branch-resolved-label">{label}</span>
          <span className="branch-resolved-key">
            {" "}
            (branch key: <code>{branches.verified_branch_key}</code>)
          </span>
        </p>
      ) : null}
    </section>
  );
}
