"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { FormStatus } from "@/components/FormStatus";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  decideProfileRevision,
  getReviewDetail,
  getReviewQueue,
  reviewMediaUrl,
  type ReviewDecision,
  type ReviewDetailResponse,
  type ReviewEntry,
  type ReviewEvent,
  type ReviewMedia,
  type ReviewQueueStatus,
} from "@/lib/api";
import type { Role } from "@/lib/experience-contract";
import { profileField } from "@/lib/profile-fields";

const FILTERS: { value: ReviewQueueStatus; label: string }[] = [
  { value: "pending_review", label: "Pending review" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
  { value: "needs_changes", label: "Needs changes" },
  { value: "all", label: "All decisions" },
];

const PAGE_SIZE = 20;

export function ReviewConsole({ role }: { role: Role | null }) {
  const [status, setStatus] = useState<ReviewQueueStatus>("pending_review");
  const [entries, setEntries] = useState<ReviewEntry[]>([]);
  const [nextCursor, setNextCursor] = useState<number | null>(null);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<ReviewDetailResponse | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);
  const [note, setNote] = useState("");
  const [confirm, setConfirm] = useState<ReviewDecision | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);
  const loadSequence = useRef(0);

  const load = useCallback(
    async (cursor: number | null, append: boolean) => {
      const sequence = ++loadSequence.current;
      setLoading(true);
      setError(null);
      try {
        const body = await getReviewQueue({
          status,
          cursor,
          limit: PAGE_SIZE,
        });
        if (sequence !== loadSequence.current) return;
        setEntries((current) =>
          append ? [...current, ...body.entries] : body.entries,
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
    [status],
  );

  useEffect(() => {
    if (role !== "admin") return;
    setSelectedId(null);
    setDetail(null);
    setNote("");
    setConfirm(null);
    setMessage(null);
    void load(null, false);
  }, [load, role]);

  useEffect(() => {
    if (!selectedId) return;
    let cancelled = false;
    setDetailLoading(true);
    setDetail(null);
    setNote("");
    setConfirm(null);
    getReviewDetail(selectedId)
      .then((body) => {
        if (!cancelled) setDetail(body);
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

  if (role !== "admin") {
    return (
      <>
        <PageTitle title="Review queue" />
        <PageHeader title="Review queue" />
        <DenialView
          kind="denied"
          reason="Only an administrator with the review permission may inspect submitted profile changes."
        />
      </>
    );
  }

  async function decide(decision: ReviewDecision) {
    if (!detail) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await decideProfileRevision(
        detail.entry.revision.revision_id,
        decision,
        note.trim(),
      );
      setMessage({
        kind: "ok",
        text:
          decision === "approve"
            ? "Submission approved. The approved value was updated."
            : decision === "reject"
              ? "Submission rejected. The last approved value is unchanged."
              : "Changes requested. The last approved value is unchanged.",
      });
      setConfirm(null);
      setNote("");
      await load(null, false);
      const refreshed = await getReviewDetail(
        detail.entry.revision.revision_id,
      );
      setDetail(refreshed);
    } catch (err) {
      setMessage({ kind: "error", text: messageOf(err) });
    } finally {
      setBusy(false);
    }
  }

  const selected = detail?.entry ?? null;

  return (
    <>
      <PageTitle title="Review queue" />
      <PageHeader
        title="Review queue"
        subtitle="Inspect submitted profile changes and decide independently. Drafts never appear here, and a submitter or linked owner cannot approve their own contribution."
      />

      <section className="review-toolbar" data-testid="review-toolbar">
        <label htmlFor="review-status-filter">Show submissions</label>
        <select
          id="review-status-filter"
          value={status}
          onChange={(event) =>
            setStatus(event.target.value as ReviewQueueStatus)
          }
          data-testid="review-filter"
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
        <div data-testid="review-message">
          <FormStatus status={{ kind: message.kind, text: message.text }} />
        </div>
      ) : null}
      {error ? (
        <div data-testid="review-error">
          <FormStatus status={{ kind: "error", text: error }} />
          <button type="button" onClick={() => void load(null, false)}>
            Try again
          </button>
        </div>
      ) : null}

      <div className="review-workspace" data-testid="review-console">
        <section className="review-queue" aria-label="Submitted revisions">
          {loading && !loaded ? (
            <p className="hint" role="status" data-testid="review-loading">
              Loading submitted revisions…
            </p>
          ) : entries.length === 0 ? (
            <p className="hint" data-testid="review-empty">
              No submissions match this filter.
            </p>
          ) : (
            <ul className="review-list" data-testid="review-list">
              {entries.map((entry) => {
                const revision = entry.revision;
                return (
                  <li key={revision.revision_id}>
                    <button
                      type="button"
                      className={
                        selectedId === revision.revision_id
                          ? "review-row selected"
                          : "review-row"
                      }
                      aria-pressed={selectedId === revision.revision_id}
                      onClick={() => setSelectedId(revision.revision_id)}
                      data-testid={`review-row-${revision.revision_id}`}
                    >
                      <span className="review-row-heading">
                        <strong>{entry.display_name}</strong>
                        <StatusChip status={revision.status} />
                      </span>
                      <span>
                        {fieldLabel(revision.field_key)} · revision{" "}
                        {revision.revision_number}
                      </span>
                      <span className="hint">
                        Submitted {formatDate(revision.submitted_at)} ·{" "}
                        {visibilityLabel(revision.visibility)}
                      </span>
                      <span className="review-row-value">
                        {mediaSummary(entry) ?? truncate(revision.value, 140)}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
          {nextCursor ? (
            <button
              type="button"
              className="button-secondary"
              disabled={loading}
              onClick={() => void load(nextCursor, true)}
              data-testid="review-load-more"
            >
              {loading ? "Loading…" : "Load more"}
            </button>
          ) : null}
        </section>

        <section
          className="review-detail"
          aria-label="Review detail"
          data-testid="review-detail"
        >
          {!selectedId ? (
            <p className="hint">
              Select a submission to compare the proposal with the last approved
              value.
            </p>
          ) : detailLoading || !selected ? (
            <p className="hint" role="status">
              Loading review detail…
            </p>
          ) : (
            <ReviewDetailPanel
              entry={selected}
              events={detail?.events ?? []}
              history={detail?.history ?? []}
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

function ReviewDetailPanel({
  entry,
  events,
  history,
  note,
  onNote,
  confirm,
  busy,
  onAsk,
  onCancel,
  onConfirm,
}: {
  entry: ReviewEntry;
  events: ReviewEvent[];
  history: ReviewDetailResponse["history"];
  note: string;
  onNote: (value: string) => void;
  confirm: ReviewDecision | null;
  busy: boolean;
  onAsk: (decision: ReviewDecision) => void;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const revision = entry.revision;
  const mediaField =
    revision.field_key === "profile_photo" ||
    revision.field_key === "personal_document";
  const requiresNote = confirm === "reject" || confirm === "needs_changes";
  const noteReady = !requiresNote || note.trim().length > 0;

  return (
    <div data-testid={`review-detail-${revision.revision_id}`}>
      <header className="review-detail-heading">
        <div>
          <p className="eyebrow">Submission {revision.revision_id}</p>
          <h2>{entry.display_name}</h2>
          <p className="hint">
            {fieldLabel(revision.field_key)} · revision{" "}
            {revision.revision_number} · submitted{" "}
            {formatDate(revision.submitted_at)}
          </p>
        </div>
        <StatusChip status={revision.status} />
      </header>

      <div className="review-meta">
        <span className="badge">{visibilityLabel(revision.visibility)}</span>
        <span className="badge">
          {mediaField ? "Private media" : "Profile text"}
        </span>
      </div>

      <div className="review-compare">
        <section aria-labelledby="review-current-heading">
          <h3 id="review-current-heading">Current approved information</h3>
          {mediaField ? (
            <MediaSummary
              media={entry.current_media}
              label="No approved media"
              testId="review-current"
            />
          ) : entry.current_approved ? (
            <div data-testid="review-current">
              <p className="review-long-value">
                {entry.current_approved.value}
              </p>
              <p className="hint">
                {visibilityLabel(entry.current_approved.visibility)} ·
                approved revision{" "}
                {entry.current_approved.approved_revision_number}
              </p>
            </div>
          ) : (
            <p className="hint" data-testid="review-current">
              No approved value yet.
            </p>
          )}
        </section>

        <section aria-labelledby="review-proposed-heading">
          <h3 id="review-proposed-heading">Proposed change</h3>
          {mediaField ? (
            <MediaProposal
              media={entry.media}
              pending={revision.status === "pending_review"}
            />
          ) : (
            <div data-testid="review-proposed">
              <p className="review-long-value">{revision.value}</p>
              <p className="hint">
                Requested visibility: {visibilityLabel(revision.visibility)}
              </p>
            </div>
          )}
        </section>
      </div>

      {entry.can_decide ? (
        <section className="review-decision" aria-labelledby="review-decision-heading">
          <h3 id="review-decision-heading">Decision</h3>
          <label htmlFor="review-note">Reviewer note</label>
          <textarea
            id="review-note"
            value={note}
            rows={4}
            maxLength={2000}
            onChange={(event) => onNote(event.target.value)}
            placeholder="Optional for approval; required for rejection or changes."
            data-testid="review-note"
          />
          <p className="hint">
            {note.length}/2000 · Rejection and requests for changes require a
            note. Approval preserves the last approved value until it commits.
          </p>

          {confirm ? (
            <div className="review-confirm" data-testid="review-confirm" role="alert">
              <p>
                {confirm === "approve"
                  ? "Approve this submission? The approved value will be replaced atomically."
                  : confirm === "reject"
                    ? "Reject this submission? The last approved value stays unchanged."
                    : "Request changes? The last approved value stays unchanged."}
              </p>
              {!noteReady ? (
                <p className="hint">Add a reviewer note before confirming.</p>
              ) : null}
              <div className="review-actions">
                <button
                  type="button"
                  onClick={onConfirm}
                  disabled={busy || !noteReady}
                  data-testid="review-confirm-button"
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
                data-testid="review-approve"
              >
                Approve
              </button>
              <button
                type="button"
                className="button-secondary"
                onClick={() => onAsk("needs_changes")}
                data-testid="review-needs-changes"
              >
                Request changes
              </button>
              <button
                type="button"
                className="button-danger"
                onClick={() => onAsk("reject")}
                data-testid="review-reject"
              >
                Reject
              </button>
            </div>
          )}
        </section>
      ) : (
        <p className="review-blocked" data-testid="review-decision-blocked">
          {blockedMessage(entry.decision_block)}
        </p>
      )}

      <section className="review-history" aria-labelledby="review-history-heading">
        <h3 id="review-history-heading">Review history</h3>
        {events.length === 0 ? (
          <p className="hint">No review events recorded.</p>
        ) : (
          <ol data-testid="review-events">
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
        {history.length > 0 ? (
          <>
            <h3>Earlier submitted versions</h3>
            <ol className="review-history-list" data-testid="review-version-history">
              {history.map((item) => (
                <li key={item.revision_id}>
                  <div className="review-history-heading">
                    <strong>Revision {item.revision_number}</strong>
                    <StatusChip status={item.status} />
                    <span className="hint">
                      {formatDate(item.reviewed_at ?? item.submitted_at)}
                    </span>
                  </div>
                  {item.review_note ? <p>{item.review_note}</p> : null}
                </li>
              ))}
            </ol>
          </>
        ) : null}
      </section>
    </div>
  );
}

function MediaProposal({
  media,
  pending,
}: {
  media: ReviewMedia | null;
  pending: boolean;
}) {
  if (!media) {
    return (
      <p className="hint" data-testid="review-proposed">
        No media metadata is attached to this submission.
      </p>
    );
  }
  return (
    <div data-testid="review-proposed">
      <MediaSummary media={media} label="Media" testId="review-proposed-meta" />
      {pending && media.kind === "profile_photo" ? (
        <img
          className="review-photo"
          src={reviewMediaUrl(media.media_id)}
          alt={`Submitted profile photo for review`}
          data-testid="review-media-preview"
        />
      ) : pending ? (
        <a
          href={reviewMediaUrl(media.media_id)}
          target="_blank"
          rel="noreferrer"
          data-testid="review-media-download"
        >
          Open submitted document
        </a>
      ) : (
        <p className="hint">
          This media is no longer pending, so the review preview is closed.
        </p>
      )}
    </div>
  );
}

function MediaSummary({
  media,
  label,
  testId,
}: {
  media: ReviewMedia | null;
  label: string;
  testId: string;
}) {
  if (!media) {
    return (
      <p className="hint" data-testid={testId}>
        {label}.
      </p>
    );
  }
  return (
    <div data-testid={testId}>
      <p>
        <strong>{media.kind === "profile_photo" ? "Photo" : "Personal document"}</strong>
      </p>
      <p className="hint">
        {media.file_name ?? "Unnamed file"}
        {media.byte_size !== null ? ` · ${formatBytes(media.byte_size)}` : ""}
        {media.width && media.height ? ` · ${media.width}×${media.height}` : ""}
      </p>
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
    case "draft":
      return "Draft";
    case "pending_review":
      return "Pending review";
    case "approved":
      return "Approved";
    case "rejected":
      return "Rejected";
    case "needs_changes":
      return "Needs changes";
    default:
      return status;
  }
}

function fieldLabel(fieldKey: string): string {
  if (fieldKey === "profile_photo") return "Profile photograph";
  if (fieldKey === "personal_document") return "Personal document";
  return profileField(fieldKey)?.label ?? fieldKey;
}

function visibilityLabel(visibility: string): string {
  return visibility === "family" ? "Authorized family" : "Owner-private";
}

function blockedMessage(block: ReviewEntry["decision_block"]): string {
  switch (block) {
    case "self_submitted":
      return "You submitted this change. An independent administrator must decide it.";
    case "reviewer_owns_person":
      return "You are linked to this genealogy person. Another administrator must decide this submission.";
    default:
      return "This submission is no longer pending review.";
  }
}

function mediaSummary(entry: ReviewEntry): string | null {
  if (!entry.media) return null;
  return entry.media.kind === "profile_photo"
    ? "Profile photograph"
    : `Personal document${entry.media.file_name ? `: ${entry.media.file_name}` : ""}`;
}

function truncate(value: string, max: number): string {
  return value.length > max ? `${value.slice(0, max)}…` : value;
}

function shortActor(actor: string): string {
  return actor.length > 16 ? `${actor.slice(0, 12)}…` : actor;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
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
      return "Your account is not permitted to review this submission.";
    }
    if (error.body?.message) return error.body.message;
  }
  return error instanceof Error ? error.message : "The request failed.";
}
