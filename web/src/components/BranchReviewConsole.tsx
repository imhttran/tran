"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { FormStatus } from "@/components/FormStatus";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  decideBranchProposal,
  getBranchGrants,
  getBranchPersonEvidence,
  getBranchProposal,
  getBranchQueue,
  type BranchDecision,
  type BranchGrant,
  type BranchPersonEvidence,
  type BranchProposal,
  type BranchProposalDetailResponse,
  type BranchProposalEvent,
  type BranchQueueFilter,
} from "@/lib/api";

// Branch-scoped shared-genealogy review.
//
// This console shows shared family facts only: proposed values, the currently
// verified value, and the source evidence attached to them. It never reads
// member-private profile fields, contact details, drafts, or owner-private
// media. Authorization is entirely server-side: the list of branches comes from
// the caller's own grants, every queue and detail read is filtered by Rust, and
// a branch without a grant is refused even if the URL is typed by hand.

const FILTERS: { value: BranchQueueFilter; label: string }[] = [
  { value: "pending_review", label: "Pending review" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
  { value: "needs_changes", label: "Needs changes" },
  { value: "all", label: "All decisions" },
];

const PAGE_SIZE = 20;

type Authority =
  | { kind: "loading" }
  | { kind: "denied"; reason: string }
  | { kind: "none"; reason: string }
  | { kind: "granted"; grants: BranchGrant[] };

export function BranchReviewConsole() {
  const [authority, setAuthority] = useState<Authority>({ kind: "loading" });
  const [branch, setBranch] = useState<string>("");
  const [status, setStatus] = useState<BranchQueueFilter>("pending_review");
  const [entries, setEntries] = useState<BranchProposal[]>([]);
  const [nextCursor, setNextCursor] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<BranchProposalDetailResponse | null>(
    null,
  );
  const [detailLoading, setDetailLoading] = useState(false);
  const [evidence, setEvidence] = useState<BranchPersonEvidence[]>([]);
  const [evidenceError, setEvidenceError] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [confirm, setConfirm] = useState<BranchDecision | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);
  const loadSequence = useRef(0);

  useEffect(() => {
    let cancelled = false;
    getBranchGrants()
      .then((body) => {
        if (cancelled) return;
        if (body.grants.length === 0) {
          setAuthority({
            kind: "none",
            reason:
              "You do not hold review authority for any family branch. An administrator grants branch review authority explicitly; a family title, relationship, or Head of Family designation does not confer it.",
          });
          return;
        }
        setAuthority({ kind: "granted", grants: body.grants });
      })
      .catch((err) => {
        if (cancelled) return;
        setAuthority({ kind: "denied", reason: messageOf(err) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const granted = authority.kind === "granted";

  const load = useCallback(
    async (cursor: number | null, append: boolean) => {
      if (!granted) return;
      const sequence = ++loadSequence.current;
      setLoading(true);
      setError(null);
      try {
        const body = await getBranchQueue({
          branch: branch || null,
          status,
          cursor,
          limit: PAGE_SIZE,
        });
        if (sequence !== loadSequence.current) return;
        setEntries((current) =>
          append ? [...current, ...body.submissions] : body.submissions,
        );
        setNextCursor(body.next_cursor);
        setLoaded(true);
      } catch (err) {
        if (sequence !== loadSequence.current) return;
        setError(messageOf(err));
      } finally {
        if (sequence === loadSequence.current) setLoading(false);
      }
    },
    [branch, granted, status],
  );

  useEffect(() => {
    if (!granted) return;
    setSelectedId(null);
    setDetail(null);
    setEvidence([]);
    setNote("");
    setConfirm(null);
    setMessage(null);
    setLoaded(false);
    void load(null, false);
  }, [granted, load]);

  useEffect(() => {
    if (!selectedId) return;
    let cancelled = false;
    setDetailLoading(true);
    setDetail(null);
    setEvidence([]);
    setEvidenceError(null);
    setNote("");
    setConfirm(null);
    getBranchProposal(selectedId)
      .then((body) => {
        if (cancelled) return;
        setDetail(body);
        return getBranchPersonEvidence(body.proposal.person_key)
          .then((refs) => {
            if (!cancelled) setEvidence(refs.evidence);
          })
          .catch((err) => {
            // The decision can still be made from the proposal's own evidence
            // references; the wider list is supporting context only.
            if (!cancelled) setEvidenceError(messageOf(err));
          });
      })
      .catch((err) => {
        if (!cancelled) setError(messageOf(err));
      })
      .finally(() => {
        if (!cancelled) setDetailLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [selectedId]);

  async function decide(decision: BranchDecision) {
    if (!detail) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await decideBranchProposal(
        detail.proposal.proposal_id,
        decision,
        note.trim(),
      );
      setMessage({
        kind: "ok",
        text:
          decision === "approve"
            ? "Change approved. The verified family fact now carries the proposed value."
            : decision === "reject"
              ? "Proposal rejected. The verified family fact is unchanged."
              : "Changes requested. The verified family fact is unchanged.",
      });
      setConfirm(null);
      setNote("");
      await load(null, false);
      const refreshed = await getBranchProposal(detail.proposal.proposal_id);
      setDetail(refreshed);
    } catch (err) {
      if (err instanceof ApiRequestError && err.status === 409) {
        setMessage({
          kind: "error",
          text: "This proposal was decided by someone else, or the family fact changed. The current state has been reloaded.",
        });
        try {
          setDetail(await getBranchProposal(detail.proposal.proposal_id));
          await load(null, false);
        } catch {
          // A refresh failure leaves the original error visible.
        }
      } else {
        setMessage({ kind: "error", text: messageOf(err) });
      }
    } finally {
      setBusy(false);
    }
  }

  if (authority.kind === "loading") {
    return (
      <>
        <PageTitle title="Branch review" />
        <PageHeader title="Branch review" />
        <p className="hint" role="status" data-testid="branch-review-loading">
          Checking your branch review authority…
        </p>
      </>
    );
  }

  if (authority.kind === "denied") {
    return (
      <>
        <PageTitle title="Branch review" />
        <PageHeader title="Branch review" />
        <DenialView kind="denied" reason={authority.reason} />
      </>
    );
  }

  if (authority.kind === "none") {
    return (
      <>
        <PageTitle title="Branch review" />
        <PageHeader
          title="Branch review"
          subtitle="Review shared family information within an authorized branch."
        />
        <section className="card" data-testid="branch-review-no-authority">
          <h2>No branch review authority</h2>
          <p>{authority.reason}</p>
          <p className="hint">
            Ask an administrator to grant review authority for a specific
            branch. Nothing in this interface can create that authority itself.
          </p>
        </section>
      </>
    );
  }

  const selected = detail?.proposal ?? null;

  return (
    <>
      <PageTitle title="Branch review" />
      <PageHeader
        title="Branch review"
        subtitle="Review proposed corrections to shared family facts inside the branches you were explicitly granted. Private profiles, drafts, and owner-private media are never shown here."
      />

      <section className="review-toolbar" data-testid="branch-review-toolbar">
        <label htmlFor="branch-review-branch">Family branch</label>
        <select
          id="branch-review-branch"
          value={branch}
          onChange={(event) => setBranch(event.target.value)}
          data-testid="branch-review-branch-filter"
        >
          <option value="">All granted branches</option>
          {authority.grants.map((grant) => (
            <option key={grant.branch_key} value={grant.branch_key}>
              {grant.branch_label ?? grant.branch_key}
            </option>
          ))}
        </select>

        <label htmlFor="branch-review-status">Show submissions</label>
        <select
          id="branch-review-status"
          value={status}
          onChange={(event) =>
            setStatus(event.target.value as BranchQueueFilter)
          }
          data-testid="branch-review-status-filter"
        >
          {FILTERS.map((filter) => (
            <option key={filter.value} value={filter.value}>
              {filter.label}
            </option>
          ))}
        </select>
        <span className="hint">
          {entries.length} loaded
          {nextCursor ? " · more available" : ""}
        </span>
      </section>

      {message ? (
        <div data-testid="branch-review-message">
          <FormStatus status={{ kind: message.kind, text: message.text }} />
        </div>
      ) : null}
      {error ? (
        <div data-testid="branch-review-error">
          <FormStatus status={{ kind: "error", text: error }} />
          <button type="button" onClick={() => void load(null, false)}>
            Try again
          </button>
        </div>
      ) : null}

      <div className="review-workspace" data-testid="branch-review-console">
        <section className="review-queue" aria-label="Shared-fact submissions">
          {loading && !loaded ? (
            <p className="hint" role="status" data-testid="branch-review-loading">
              Loading branch submissions…
            </p>
          ) : entries.length === 0 ? (
            <p className="hint" data-testid="branch-review-empty">
              No shared-fact submissions match this filter in your branches.
            </p>
          ) : (
            <ul className="review-list" data-testid="branch-review-list">
              {entries.map((proposal) => (
                <li key={proposal.proposal_id}>
                  <button
                    type="button"
                    className={
                      selectedId === proposal.proposal_id
                        ? "review-row selected"
                        : "review-row"
                    }
                    aria-pressed={selectedId === proposal.proposal_id}
                    onClick={() => setSelectedId(proposal.proposal_id)}
                    data-testid={`branch-review-row-${proposal.proposal_id}`}
                  >
                    <span className="review-row-heading">
                      <strong>{proposal.display_name}</strong>
                      <StatusChip status={proposal.status} />
                    </span>
                    <span>
                      {branchLabel(proposal.branch_key, authority.grants)} ·{" "}
                      {factLabel(proposal.assertion_kind, proposal.assertion_key)}
                    </span>
                    <span className="hint">
                      Submitted {formatDate(proposal.submitted_at)} · proposal{" "}
                      {proposal.proposal_id} · {proposal.evidence.length} source
                      {proposal.evidence.length === 1 ? "" : "s"}
                    </span>
                    <span className="review-row-value">
                      {truncate(proposal.proposed_value_text, 140)}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          )}
          {nextCursor ? (
            <button
              type="button"
              className="button-secondary"
              disabled={loading}
              onClick={() => void load(nextCursor, true)}
              data-testid="branch-review-load-more"
            >
              {loading ? "Loading…" : "Load more"}
            </button>
          ) : null}
        </section>

        <section
          className="review-detail"
          aria-label="Branch review detail"
          data-testid="branch-review-detail"
        >
          {!selectedId ? (
            <p className="hint">
              Select a submission to compare the proposed fact with the verified
              value and its sources.
            </p>
          ) : detailLoading || !selected ? (
            <p
              className="hint"
              role="status"
              data-testid="branch-review-detail-loading"
            >
              Loading review detail…
            </p>
          ) : (
            <BranchDetailPanel
              proposal={selected}
              current={detail?.current_assertion ?? null}
              events={detail?.events ?? []}
              evidence={evidence}
              evidenceError={evidenceError}
              grants={authority.grants}
              note={note}
              onNote={setNote}
              confirm={confirm}
              busy={busy}
              onAsk={setConfirm}
              onCancel={() => setConfirm(null)}
              onConfirm={() => {
                if (confirm) void decide(confirm);
              }}
            />
          )}
        </section>
      </div>
    </>
  );
}

function BranchDetailPanel({
  proposal,
  current,
  events,
  evidence,
  evidenceError,
  grants,
  note,
  onNote,
  confirm,
  busy,
  onAsk,
  onCancel,
  onConfirm,
}: {
  proposal: BranchProposal;
  current: BranchProposalDetailResponse["current_assertion"];
  events: BranchProposalEvent[];
  evidence: BranchPersonEvidence[];
  evidenceError: string | null;
  grants: BranchGrant[];
  note: string;
  onNote: (value: string) => void;
  confirm: BranchDecision | null;
  busy: boolean;
  onAsk: (decision: BranchDecision) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const requiresNote = confirm === "reject" || confirm === "needs_changes";
  const noteReady = !requiresNote || note.trim().length > 0;

  return (
    <div data-testid={`branch-review-detail-${proposal.proposal_id}`}>
      <header className="review-detail-heading">
        <div>
          <p className="eyebrow">Proposal {proposal.proposal_id}</p>
          <h2>{proposal.display_name}</h2>
          <p className="hint">
            {branchLabel(proposal.branch_key, grants)} ·{" "}
            {factLabel(proposal.assertion_kind, proposal.assertion_key)} ·
            submitted {formatDate(proposal.submitted_at)}
          </p>
        </div>
        <StatusChip status={proposal.status} />
      </header>

      <div className="review-meta">
        <span className="badge">Shared genealogy fact</span>
        <span className="badge">{proposal.person_key}</span>
      </div>

      <div className="review-compare">
        <section aria-labelledby="branch-current-heading">
          <h3 id="branch-current-heading">Currently verified fact</h3>
          {current?.value_text ? (
            <div data-testid="branch-review-current">
              <p className="review-long-value">{current.value_text}</p>
              <p className="hint">
                Status {current.review_status}
                {current.reviewed_at
                  ? ` · reviewed ${formatDate(current.reviewed_at)}`
                  : ""}
              </p>
            </div>
          ) : (
            <p className="hint" data-testid="branch-review-current">
              No verified value yet.
              {proposal.prior_value_text
                ? ` The snapshot captured at submission was “${proposal.prior_value_text}”.`
                : ""}
            </p>
          )}
        </section>

        <section aria-labelledby="branch-proposed-heading">
          <h3 id="branch-proposed-heading">Proposed correction</h3>
          <div data-testid="branch-review-proposed">
            <p className="review-long-value">{proposal.proposed_value_text}</p>
            {proposal.prior_value_text ? (
              <p className="hint">
                Submitted against “{proposal.prior_value_text}”
                {proposal.prior_review_status
                  ? ` (${proposal.prior_review_status})`
                  : ""}
                .
              </p>
            ) : (
              <p className="hint">Submitted when no value was recorded.</p>
            )}
          </div>
        </section>
      </div>

      <section
        className="review-history"
        aria-labelledby="branch-evidence-heading"
        data-testid="branch-review-evidence-section"
      >
        <h3 id="branch-evidence-heading">Cited sources</h3>
        {proposal.evidence.length === 0 ? (
          <p className="hint">This proposal cites no evidence.</p>
        ) : (
          <ol data-testid="branch-review-proposal-evidence">
            {proposal.evidence.map((ref) => (
              <li key={`${ref.source_evidence_id}-${ref.page}`}>
                <strong>{ref.document_ref}</strong>
                <span className="hint"> · page {ref.page}</span>
              </li>
            ))}
          </ol>
        )}
        {evidenceError ? (
          <p className="hint" data-testid="branch-review-evidence-error">
            {evidenceError}
          </p>
        ) : evidence.length > 0 ? (
          <>
            <h4>Other sources recorded for this person</h4>
            <ol data-testid="branch-review-person-evidence">
              {evidence.map((ref) => (
                <li
                  key={`${ref.source_evidence_id}-${ref.assertion_kind}-${ref.assertion_key}`}
                >
                  <strong>{ref.document_ref}</strong>
                  <span className="hint">
                    {" "}
                    · page {ref.page}
                    {ref.locator ? ` · ${ref.locator}` : ""} ·{" "}
                    {factLabel(ref.assertion_kind, ref.assertion_key)}
                  </span>
                </li>
              ))}
            </ol>
          </>
        ) : null}
      </section>

      {proposal.can_decide ? (
        <section
          className="review-decision"
          aria-labelledby="branch-decision-heading"
        >
          <h3 id="branch-decision-heading">Decision</h3>
          <label htmlFor="branch-review-note">Reviewer note</label>
          <textarea
            id="branch-review-note"
            value={note}
            rows={4}
            maxLength={2000}
            onChange={(event) => onNote(event.target.value)}
            placeholder="Optional for approval; required for rejection or changes."
            data-testid="branch-review-note"
          />
          <p className="hint">
            {note.length}/2000 · Rejection and requests for changes require a
            note. Approval replaces the verified value only if it still matches
            the snapshot this proposal was submitted against.
          </p>

          {confirm ? (
            <div
              className="review-confirm"
              data-testid="branch-review-confirm"
              role="alert"
            >
              <p>
                {confirm === "approve"
                  ? "Approve this correction? The shared family fact will be updated and recorded in the review history."
                  : confirm === "reject"
                    ? "Reject this proposal? The verified fact stays unchanged."
                    : "Request changes? The verified fact stays unchanged."}
              </p>
              {!noteReady ? (
                <p className="hint">Add a reviewer note before confirming.</p>
              ) : null}
              <div className="review-actions">
                <button
                  type="button"
                  onClick={onConfirm}
                  disabled={busy || !noteReady}
                  data-testid="branch-review-confirm-button"
                >
                  {busy ? "Saving…" : "Confirm decision"}
                </button>
                <button
                  type="button"
                  className="button-secondary"
                  onClick={onCancel}
                  disabled={busy}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : (
            <div className="review-actions">
              <button
                type="button"
                onClick={() => onAsk("approve")}
                data-testid="branch-review-approve"
              >
                Approve
              </button>
              <button
                type="button"
                className="button-secondary"
                onClick={() => onAsk("needs_changes")}
                data-testid="branch-review-needs-changes"
              >
                Request changes
              </button>
              <button
                type="button"
                className="button-danger"
                onClick={() => onAsk("reject")}
                data-testid="branch-review-reject"
              >
                Reject
              </button>
            </div>
          )}
        </section>
      ) : (
        <p
          className="review-blocked"
          data-testid="branch-review-decision-blocked"
        >
          {blockedMessage(proposal.decision_block)}
        </p>
      )}

      <section
        className="review-history"
        aria-labelledby="branch-history-heading"
      >
        <h3 id="branch-history-heading">Review history</h3>
        {events.length === 0 ? (
          <p className="hint">No review events recorded.</p>
        ) : (
          <ol data-testid="branch-review-events">
            {events.map((event, index) => (
              <li key={`${event.created_at}-${index}`}>
                <div className="review-history-heading">
                  <StatusChip status={event.event} />
                  <span className="hint">{formatDate(event.created_at)}</span>
                </div>
                {event.note ? <p>{event.note}</p> : null}
                <p className="hint">Actor {shortActor(event.actor)}</p>
              </li>
            ))}
          </ol>
        )}
      </section>
    </div>
  );
}

function StatusChip({ status }: { status: string }) {
  return (
    <span className={`status-chip status-${status}`}>
      {statusLabel(status)}
    </span>
  );
}

function statusLabel(status: string): string {
  switch (status) {
    case "pending_review":
      return "Pending review";
    case "approved":
      return "Approved";
    case "rejected":
      return "Rejected";
    case "needs_changes":
      return "Needs changes";
    case "submitted":
      return "Submitted";
    default:
      return status;
  }
}

// Fact labels keep the shared-fact language plain. Unknown kinds fall back to
// the stored key so a new catalog kind is never hidden or mislabelled.
function factLabel(kind: string, key: string): string {
  const known: Record<string, string> = {
    branch_claim: "Branch claim",
    name: "Name",
    gender: "Gender",
    vital_status: "Vital status",
    parent_child: "Parent–child",
    spouse: "Spouse",
    birth_order: "Birth order",
  };
  const label = known[kind] ?? kind;
  return key && key !== kind ? `${label} · ${key}` : label;
}

function branchLabel(branchKey: string, grants: BranchGrant[]): string {
  const grant = grants.find((item) => item.branch_key === branchKey);
  return grant?.branch_label ?? branchKey;
}

function blockedMessage(block: string | null): string {
  switch (block) {
    case "self_submitted":
      return "You submitted this proposal. An independent reviewer must decide it.";
    case "reviewer_owns_person":
      return "You are linked to this genealogy person. Another branch reviewer must decide this proposal.";
    case "not_pending":
      return "This proposal is no longer pending review.";
    default:
      return "This proposal cannot be decided by your account.";
  }
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max)}…` : value;
}

function shortActor(actor: string): string {
  return actor.length > 16 ? `${actor.slice(0, 12)}…` : actor;
}

function formatDate(value: string | null): string {
  if (!value) return "not recorded";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError) {
    if (error.status === 401) return "Your session expired. Sign in again.";
    if (error.status === 403) {
      return "Your account has no review authority for this branch or submission.";
    }
    if (error.status === 404) {
      return "That submission is not available in your granted branches.";
    }
    if (error.status === 409) {
      return "This submission conflicts with the current review state. Reload to see it.";
    }
    if (error.body?.message) return error.body.message;
  }
  return error instanceof Error ? error.message : "The request failed.";
}
