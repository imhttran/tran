"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { branchStateText } from "@/components/BranchSection";
import { FormStatus } from "@/components/FormStatus";
import type { FormStatusState } from "@/components/FormStatus";
import {
  ApiRequestError,
  grantRecordedBranch,
  listReviewerGrants,
  revokeBranchReviewer,
  searchBranchGrantPeople,
} from "@/lib/api";
import type {
  AdminContact,
  BranchBlock,
  BranchGrantEligibility,
  BranchGrantPerson,
  ReviewerGrant,
} from "@/lib/api";

// Admin > Family Members > recorded-branch grants.
//
// The administrator searches the authorized person index, sees the exact
// person, the reviewed account, and the recorded branch, and grants review
// authority for the branch that person's own verified claim records. The server
// resolves and rechecks everything; nothing here decides authorization and the
// console never offers a branch picker.

const ERROR_MESSAGES: Record<string, string> = {
  account_not_linked:
    "This account has no reviewed person link. Nothing was granted.",
  branch_grant_exists: "This account already reviews that branch.",
  branch_grant_not_active: "That grant was already revoked.",
  person_link_mismatch:
    "That person is not the account's reviewed link. Nothing was granted.",
  branch_membership_missing: "This person has no recorded branch.",
  branch_membership_unverified: "The recorded branch is not verified.",
  branch_membership_conflicting: "The recorded branches disagree.",
  invalid_branch_reason: "Give a reason of at most 1000 characters.",
  csrf_failed:
    "Your session's security token is stale. Reload the page and try again.",
};

function messageFor(error: unknown, fallback: string): string {
  if (error instanceof ApiRequestError) {
    const code = error.body?.error;
    return (code && ERROR_MESSAGES[code]) || error.message || fallback;
  }
  return error instanceof Error ? error.message : fallback;
}

function contactText(value: AdminContact["email"]): string {
  const source =
    value.source === "identity_account" ? "identity account" : "not recorded";
  const recorded = value.value ?? "not recorded";
  return `${recorded} (source: ${source}, verification: not recorded)`;
}

/// The resolved branch of a verified projection, or a statement of the state.
function branchText(block: BranchBlock): string {
  if (block.state !== "verified" || !block.verified_branch_key) {
    return branchStateText(block);
  }
  const claim = block.claims.find(
    (entry) => entry.verified && entry.branch_key === block.verified_branch_key,
  );
  const label =
    claim?.branch_label ?? claim?.value_text ?? block.verified_branch_key;
  return `${label} (branch key: ${block.verified_branch_key})`;
}

function eligibilityReason(eligibility: BranchGrantEligibility): string {
  switch (eligibility) {
    case "eligible":
      return "Eligible: one verified recorded branch and a reviewed link.";
    case "missing_link":
      return "Not eligible: the account has no reviewed person link.";
    case "missing_branch":
      return "Not eligible: no branch recorded.";
    case "unverified_branch":
      return "Not eligible: the recorded branch is not verified.";
    case "conflicting_branch":
      return "Not eligible: the recorded branches disagree.";
    case "already_granted":
      return "Already granted for this branch.";
    default:
      return "Eligibility is unavailable.";
  }
}

function accountName(account: BranchGrantPerson["account"]): string {
  if (!account) return "No reviewed account link";
  const who = account.username || account.display_name || account.subject;
  return `${who} · user_id ${account.user_id}`;
}

type SearchState = "idle" | "loading" | "ready" | "error";
type LoadState = "loading" | "ready" | "error";

export function BranchGrantAdmin() {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<BranchGrantPerson[]>([]);
  const [searchState, setSearchState] = useState<SearchState>("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const sequence = useRef(0);

  const [grants, setGrants] = useState<ReviewerGrant[]>([]);
  const [grantsState, setGrantsState] = useState<LoadState>("loading");
  const [grantsError, setGrantsError] = useState<string | null>(null);

  const [grantTarget, setGrantTarget] = useState<BranchGrantPerson | null>(
    null,
  );
  const [revokeTarget, setRevokeTarget] = useState<ReviewerGrant | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<FormStatusState>(null);

  const runSearch = useCallback(async (term: string) => {
    const current = ++sequence.current;
    const trimmed = term.trim();
    if (!trimmed) {
      setResults([]);
      setSearchState("idle");
      setSearchError(null);
      return;
    }
    setSearchState("loading");
    setSearchError(null);
    try {
      const response = await searchBranchGrantPeople(trimmed);
      if (sequence.current !== current) return;
      setResults(response.people);
      setSearchState("ready");
    } catch (error) {
      if (sequence.current !== current) return;
      setSearchError(messageFor(error, "The person search failed."));
      setSearchState("error");
    }
  }, []);

  // A short debounce and a stale-response guard: only the newest term may
  // replace the list.
  useEffect(() => {
    const handle = window.setTimeout(() => {
      void runSearch(query);
    }, 300);
    return () => window.clearTimeout(handle);
  }, [query, runSearch]);

  const loadGrants = useCallback(async () => {
    setGrantsState("loading");
    try {
      const all = await listReviewerGrants();
      setGrants(all);
      setGrantsState("ready");
      setGrantsError(null);
    } catch (error) {
      setGrantsError(
        messageFor(error, "The branch reviewer grants could not be loaded."),
      );
      setGrantsState("error");
    }
  }, []);

  useEffect(() => {
    void loadGrants();
  }, [loadGrants]);

  async function confirmGrant(reason: string) {
    if (!grantTarget?.account || busy) return;
    setBusy(true);
    const account = grantTarget.account;
    const person = grantTarget.person;
    const branch = branchText(grantTarget.branches);
    try {
      await grantRecordedBranch(
        { issuer: account.issuer, subject: account.subject },
        person.person_key,
        reason,
      );
      // Success is reported only after the grant exists and the views reload.
      await Promise.all([loadGrants(), runSearch(query)]);
      setStatus({
        kind: "ok",
        text: `Granted branch review for ${branch} to ${person.display_name}.`,
      });
      setGrantTarget(null);
    } catch (error) {
      setStatus({
        kind: "error",
        text: messageFor(error, "The grant failed. Nothing was changed."),
      });
      setGrantTarget(null);
    } finally {
      setBusy(false);
    }
  }

  async function confirmRevoke(reason: string) {
    if (!revokeTarget || busy) return;
    setBusy(true);
    const grant = revokeTarget;
    try {
      await revokeBranchReviewer(grant.grant_id, reason);
      await Promise.all([loadGrants(), runSearch(query)]);
      setStatus({
        kind: "ok",
        text: `Revoked ${grant.branch_label || grant.branch_key}. The audit history is kept.`,
      });
      setRevokeTarget(null);
    } catch (error) {
      setStatus({
        kind: "error",
        text: messageFor(error, "The revocation failed. Nothing was changed."),
      });
      setRevokeTarget(null);
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="status-panel fm-accounts branch-grant-admin"
      data-testid="branch-grant-admin"
      aria-labelledby="branch-grant-admin-heading"
    >
      <div className="fm-section-head">
        <h2 id="branch-grant-admin-heading">Branch review grants</h2>
        <p className="hint">
          Search a person to grant review authority for the branch their own
          verified claim records. Membership is never inferred from a name,
          relationship, or appointment, and the API rechecks it.
        </p>
      </div>

      <label className="fm-filter-field branch-grant-search">
        <span>Search person</span>
        <input
          type="search"
          placeholder="Name or person key"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          data-testid="branch-grant-search"
        />
      </label>

      <FormStatus status={status} />

      {searchState === "loading" ? (
        <p
          className="state-panel state-panel-loading fm-state"
          role="status"
          data-testid="branch-grant-search-loading"
        >
          Searching people…
        </p>
      ) : null}
      {searchState === "error" ? (
        <p
          className="state-panel state-panel-unavailable fm-state"
          role="alert"
          data-testid="branch-grant-search-error"
        >
          {searchError}
        </p>
      ) : null}
      {searchState === "ready" && results.length === 0 ? (
        <p
          className="state-panel state-panel-empty fm-state"
          data-testid="branch-grant-search-empty"
        >
          No people match this search.
        </p>
      ) : null}

      {results.length > 0 ? (
        <ul className="branch-grant-results" data-testid="branch-grant-results">
          {results.map((candidate) => {
            const eligible = candidate.eligibility === "eligible";
            const account = candidate.account;
            const contact = candidate.contact;
            return (
              <li
                key={candidate.person.person_key}
                className="branch-grant-result"
                data-testid="branch-grant-result"
                data-eligibility={candidate.eligibility}
                data-person={candidate.person.person_key}
              >
                <div className="branch-grant-result-head">
                  <strong>{candidate.person.display_name}</strong>
                  <span className="hint">
                    {candidate.person.person_key} · person_id{" "}
                    {candidate.person.person_id ?? "not recorded"}
                  </span>
                </div>
                <dl className="branch-grant-facts">
                  <div>
                    <dt>Account</dt>
                    <dd>{accountName(account)}</dd>
                  </div>
                  <div>
                    <dt>Email</dt>
                    <dd>
                      {contact ? contactText(contact.email) : "not recorded"}
                    </dd>
                  </div>
                  <div>
                    <dt>Phone</dt>
                    <dd>not recorded (source: not recorded)</dd>
                  </div>
                  <div>
                    <dt>Recorded branch</dt>
                    <dd>{branchText(candidate.branches)}</dd>
                  </div>
                </dl>
                <p
                  className={`branch-grant-eligibility branch-grant-eligibility-${candidate.eligibility}`}
                  data-testid="branch-grant-eligibility"
                >
                  {eligibilityReason(candidate.eligibility)}
                </p>
                <button
                  type="button"
                  disabled={!eligible}
                  onClick={() => {
                    setStatus(null);
                    setGrantTarget(candidate);
                  }}
                  data-testid="branch-grant-start"
                >
                  Grant branch review
                </button>
              </li>
            );
          })}
        </ul>
      ) : null}

      <div className="branch-grant-list">
        <h3>Current and historical grants</h3>
        {grantsState === "loading" ? (
          <p
            className="state-panel state-panel-loading fm-state"
            role="status"
            data-testid="branch-grant-list-loading"
          >
            Loading grants…
          </p>
        ) : null}
        {grantsState === "error" ? (
          <p
            className="state-panel state-panel-unavailable fm-state"
            role="alert"
            data-testid="branch-grant-list-error"
          >
            {grantsError}
          </p>
        ) : null}
        {grantsState === "ready" && grants.length === 0 ? (
          <p
            className="state-panel state-panel-empty fm-state"
            data-testid="branch-grant-list-empty"
          >
            No branch grants yet.
          </p>
        ) : null}
        {grants.length > 0 ? (
          <div className="fm-table-wrap">
            <table className="bulk-table branch-grant-table">
              <thead>
                <tr>
                  <th>Person</th>
                  <th>Account</th>
                  <th>Branch</th>
                  <th>Actor</th>
                  <th>Reason</th>
                  <th>Status</th>
                  <th>Created</th>
                  <th>Revoked</th>
                  <th>Action</th>
                </tr>
              </thead>
              <tbody>
                {grants.map((grant) => (
                  <tr
                    key={grant.grant_id}
                    data-testid="branch-grant-row"
                    data-grant-id={grant.grant_id}
                    data-person={grant.person?.person_key ?? ""}
                    data-branch={grant.branch_key}
                    data-active={grant.active ? "true" : "false"}
                  >
                    <td data-label="Person">
                      {grant.person
                        ? `${grant.person.display_name} (${grant.person.person_key}, person_id ${grant.person.person_id})`
                        : "—"}
                    </td>
                    <td data-label="Account">
                      {grant.account
                        ? `${grant.account.username || grant.account.display_name || grant.account.subject} (user_id ${grant.account.user_id})`
                        : "—"}
                    </td>
                    <td data-label="Branch">
                      {grant.branch_label || grant.branch_key}
                    </td>
                    <td data-label="Actor">{grant.granted_by}</td>
                    <td data-label="Reason">{grant.granted_reason ?? "—"}</td>
                    <td data-label="Status">
                      {grant.active ? "active" : "revoked"}
                    </td>
                    <td data-label="Created">{grant.created_at}</td>
                    <td data-label="Revoked">
                      {grant.revoked_at
                        ? `${grant.revoked_at}${grant.revoke_reason ? ` — ${grant.revoke_reason}` : ""}`
                        : "—"}
                    </td>
                    <td data-label="Action">
                      {grant.active ? (
                        <button
                          type="button"
                          className="button-secondary"
                          onClick={() => {
                            setStatus(null);
                            setRevokeTarget(grant);
                          }}
                          data-testid="branch-grant-revoke"
                        >
                          Revoke
                        </button>
                      ) : (
                        "—"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        ) : null}
      </div>

      {grantTarget ? (
        <ReasonDialog
          testId="branch-grant-confirm"
          title="Confirm branch review grant"
          confirmLabel="Grant branch review"
          busy={busy}
          onCancel={() => setGrantTarget(null)}
          onConfirm={(reason) => void confirmGrant(reason)}
        >
          <p>
            Grant <strong>{branchText(grantTarget.branches)}</strong> to{" "}
            <strong>{grantTarget.person.display_name}</strong> (
            {grantTarget.person.person_key}, person_id{" "}
            {grantTarget.person.person_id ?? "not recorded"}).
          </p>
          <p>
            Account: <strong>{accountName(grantTarget.account)}</strong>
            {grantTarget.account
              ? ` · ${grantTarget.account.subject} @ ${grantTarget.account.issuer}`
              : ""}
          </p>
        </ReasonDialog>
      ) : null}

      {revokeTarget ? (
        <ReasonDialog
          testId="branch-grant-revoke-confirm"
          title="Confirm branch review revocation"
          confirmLabel="Revoke branch review"
          busy={busy}
          onCancel={() => setRevokeTarget(null)}
          onConfirm={(reason) => void confirmRevoke(reason)}
        >
          <p>
            Revoke{" "}
            <strong>
              {revokeTarget.branch_label || revokeTarget.branch_key}
            </strong>
            {revokeTarget.person ? (
              <>
                {" "}
                from <strong>{revokeTarget.person.display_name}</strong>
              </>
            ) : null}
            ? The grant history is kept.
          </p>
        </ReasonDialog>
      ) : null}
    </section>
  );
}

/// A confirmation with a required reason. Cancel takes focus on open, Escape
/// cancels, and a click outside the dialog cancels; the confirm action stays
/// explicit and is disabled until a reason is entered.
function ReasonDialog({
  testId,
  title,
  confirmLabel,
  busy,
  children,
  onCancel,
  onConfirm,
}: {
  testId: string;
  title: string;
  confirmLabel: string;
  busy: boolean;
  children: React.ReactNode;
  onCancel: () => void;
  onConfirm: (reason: string) => void;
}) {
  const [reason, setReason] = useState("");
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cancelRef.current?.focus();
  }, []);

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
    }
  }

  return (
    <div
      className="confirm fm-confirm branch-grant-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby={`${testId}-title`}
      data-testid={testId}
      onKeyDown={handleKeyDown}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
    >
      <h3 id={`${testId}-title`}>{title}</h3>
      {children}
      <label className="branch-grant-reason">
        <span>Reason (required)</span>
        <textarea
          maxLength={1000}
          value={reason}
          onChange={(event) => setReason(event.target.value)}
          data-testid={`${testId}-reason`}
        />
      </label>
      <div className="fm-confirm-actions">
        <button
          ref={cancelRef}
          type="button"
          className="button-secondary"
          disabled={busy}
          onClick={onCancel}
          data-testid={`${testId}-cancel`}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={busy || !reason.trim()}
          onClick={() => onConfirm(reason.trim())}
          data-testid={`${testId}-yes`}
        >
          {busy ? "Working…" : confirmLabel}
        </button>
      </div>
    </div>
  );
}
