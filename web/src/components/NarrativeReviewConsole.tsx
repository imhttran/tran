"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { FormStatus } from "@/components/FormStatus";
import type { FormStatusState } from "@/components/FormStatus";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { SourceDialog } from "@/components/SourceDetails";
import type { OpenSource } from "@/components/SourceDetails";
import {
  ApiRequestError,
  getNarrativeReviewDetail,
  getNarrativeReviewQueue,
  reviewNarrativeLink,
  reviewNarrativePassage,
  type NarrativeLinkStatus,
  type NarrativePassageDecision,
  type NarrativePassageVisibility,
  type NarrativeReviewDetailResponse,
  type NarrativeReviewEvent,
  type NarrativeReviewLink,
  type NarrativeReviewPassage,
  type NarrativeReviewPerson,
  type NarrativeReviewQueueEntry,
  type NarrativeReviewStatus,
} from "@/lib/api";
import type { Role } from "@/lib/experience-contract";

const STATUS_OPTIONS: { value: NarrativeReviewStatus; label: string }[] = [
  { value: "pending", label: "Pending" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
  { value: "all", label: "All statuses" },
];

const LANGUAGE_OPTIONS: { value: "all" | "vi" | "en"; label: string }[] = [
  { value: "all", label: "All languages" },
  { value: "vi", label: "Vietnamese" },
  { value: "en", label: "English" },
];

const LINK_ORDER: NarrativeLinkStatus[] = [
  "confirmed",
  "resolved",
  "ambiguous",
  "rejected",
];

const LINK_LABEL: Record<NarrativeLinkStatus, string> = {
  confirmed: "Confirmed",
  resolved: "Resolved",
  ambiguous: "Ambiguous",
  rejected: "Rejected",
};

type Filters = {
  status: NarrativeReviewStatus;
  language: "all" | "vi" | "en";
  person: string;
  unmatched: boolean;
};

const DEFAULT_FILTERS: Filters = {
  status: "pending",
  language: "all",
  person: "",
  unmatched: false,
};

export function NarrativeReviewConsole({ role }: { role: Role | null }) {
  const [filters, setFilters] = useState<Filters>(DEFAULT_FILTERS);
  const [applied, setApplied] = useState<Filters>(DEFAULT_FILTERS);
  const [entries, setEntries] = useState<NarrativeReviewQueueEntry[]>([]);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<FormStatusState>(null);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [detail, setDetail] = useState<NarrativeReviewDetailResponse | null>(
    null,
  );
  const [detailLoading, setDetailLoading] = useState(false);
  const [selectedPerson, setSelectedPerson] = useState<string | null>(null);
  const [linkNote, setLinkNote] = useState("");
  const [passageNote, setPassageNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [openSource, setOpenSource] = useState<OpenSource | null>(null);
  const loadSequence = useRef(0);

  const load = useCallback(async (current: Filters) => {
    const sequence = ++loadSequence.current;
    setLoading(true);
    setError(null);
    try {
      const body = await getNarrativeReviewQueue({
        status: current.status,
        language: current.language,
        person: current.person,
        unmatched: current.unmatched,
        limit: 200,
      });
      if (sequence !== loadSequence.current) return;
      setEntries(body.entries);
      setLoaded(true);
    } catch (err) {
      if (sequence !== loadSequence.current) return;
      setError(messageOf(err));
    } finally {
      if (sequence === loadSequence.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (role !== "admin") return;
    void load(applied);
  }, [role, applied, load]);

  useEffect(() => {
    if (!selectedId) {
      setDetail(null);
      setSelectedPerson(null);
      return;
    }
    let cancelled = false;
    setDetailLoading(true);
    setDetail(null);
    setSelectedPerson(null);
    setLinkNote("");
    setPassageNote("");
    getNarrativeReviewDetail(selectedId)
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

  const people = useMemo(
    () =>
      new Map(
        (detail?.people ?? []).map((person) => [person.person_key, person]),
      ),
    [detail],
  );
  const accepted = Boolean(
    detail?.links.some(
      (link) =>
        link.link_status === "resolved" || link.link_status === "confirmed",
    ),
  );

  function applyFilters() {
    setSelectedId(null);
    setDetail(null);
    setMessage(null);
    setApplied({ ...filters });
  }

  async function reloadAfterConflict(passageId: number) {
    try {
      const body = await getNarrativeReviewDetail(passageId);
      setDetail(body);
    } catch {
      setDetail(null);
      setSelectedId(null);
    }
    await load(applied);
  }

  async function decideLink(accept: boolean) {
    if (!detail || !selectedPerson) return;
    const link = detail.links.find(
      (candidate) => candidate.person_key === selectedPerson,
    );
    if (!link) return;
    setBusy(true);
    setMessage(null);
    try {
      const body = await reviewNarrativeLink(
        detail.passage.passage_id,
        link.person_key,
        accept ? "confirm" : "reject",
        link.link_status,
        linkNote.trim(),
      );
      setDetail(body);
      setSelectedPerson(null);
      setLinkNote("");
      setMessage({
        kind: "ok",
        text: accept
          ? `Confirmed ${link.display_name}. The passage status is still ${body.passage.review_status}; no passage approval was made.`
          : `Rejected ${link.display_name}. The passage status is still ${body.passage.review_status}.`,
      });
      await load(applied);
    } catch (err) {
      if (err instanceof ApiRequestError && err.status === 409) {
        setMessage({
          kind: "error",
          text: "This person link changed since you loaded it. The latest state is being reloaded; review it again before deciding.",
        });
        await reloadAfterConflict(detail.passage.passage_id);
      } else {
        setMessage({ kind: "error", text: messageOf(err) });
      }
    } finally {
      setBusy(false);
    }
  }

  async function decidePassage(
    decision: NarrativePassageDecision,
    visibility: NarrativePassageVisibility,
  ) {
    if (!detail) return;
    setBusy(true);
    setMessage(null);
    try {
      const body = await reviewNarrativePassage(
        detail.passage.passage_id,
        decision,
        visibility,
        detail.passage.review_status,
        detail.passage.updated_at,
        passageNote.trim(),
      );
      setDetail(body);
      setPassageNote("");
      setSelectedPerson(null);
      setMessage({
        kind: "ok",
        text:
          decision === "approved" && visibility === "family"
            ? "Approved for family. The passage is now visible to authorized family readers."
            : decision === "approved"
              ? "Approved for administrators only. The passage remains hidden from ordinary family readers."
              : decision === "rejected"
                ? "Passage rejected. It is hidden from family readers."
                : "Passage returned to pending. It is hidden from family readers.",
      });
      await load(applied);
    } catch (err) {
      if (err instanceof ApiRequestError && err.status === 409) {
        setMessage({
          kind: "error",
          text: "This passage changed since you loaded it. The latest state is being reloaded; review it again before deciding.",
        });
        await reloadAfterConflict(detail.passage.passage_id);
      } else {
        setMessage({ kind: "error", text: messageOf(err) });
      }
    } finally {
      setBusy(false);
    }
  }

  function openCitation(passage: NarrativeReviewPassage) {
    const section = passage.section_title?.trim() || "Untitled section";
    const pages =
      passage.page_start === passage.page_end
        ? `page ${passage.page_start}`
        : `pages ${passage.page_start}–${passage.page_end}`;
    setOpenSource({
      evidence: {
        evidence_id: `narrative-review-${passage.passage_id}`,
        citation: `${section} — ${passage.document_ref}, PDF ${pages}`,
        document_ref: passage.document_ref,
        page: passage.page_start,
        locator: passage.section_title,
        excerpt: null,
      },
      citedFor: [
        {
          label: `Passage ${passage.passage_id}`,
          status: passage.review_status,
        },
      ],
    });
  }

  if (role !== "admin") {
    return (
      <>
        <PageTitle title="Personal history review" />
        <PageHeader title="Personal history review" />
        <DenialView
          kind="denied"
          reason="Only an administrator with the review permission may inspect source narrative passages."
        />
      </>
    );
  }

  return (
    <>
      <PageTitle title="Personal history review" />
      <PageHeader
        title="Personal history review"
        subtitle="Inspect source-backed passages, confirm or reject person links, and approve family visibility. Link decisions and passage decisions are separate and audited."
      />

      <section
        className="review-toolbar"
        data-testid="narrative-review-filters"
      >
        <label htmlFor="narrative-status-filter">Status</label>
        <select
          id="narrative-status-filter"
          value={filters.status}
          onChange={(event) =>
            setFilters({
              ...filters,
              status: event.target.value as NarrativeReviewStatus,
            })
          }
          data-testid="narrative-status-filter"
        >
          {STATUS_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        <label htmlFor="narrative-language-filter">Language</label>
        <select
          id="narrative-language-filter"
          value={filters.language}
          onChange={(event) =>
            setFilters({
              ...filters,
              language: event.target.value as Filters["language"],
            })
          }
          data-testid="narrative-language-filter"
        >
          {LANGUAGE_OPTIONS.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
        <label htmlFor="narrative-person-filter">Person key</label>
        <input
          id="narrative-person-filter"
          type="search"
          value={filters.person}
          placeholder="Exact person key"
          onChange={(event) =>
            setFilters({ ...filters, person: event.target.value })
          }
          data-testid="narrative-person-filter"
        />
        <label htmlFor="narrative-unmatched-filter">
          <input
            id="narrative-unmatched-filter"
            type="checkbox"
            checked={filters.unmatched}
            onChange={(event) =>
              setFilters({ ...filters, unmatched: event.target.checked })
            }
            data-testid="narrative-unmatched-filter"
          />{" "}
          Unmatched only
        </label>
        <button
          type="button"
          onClick={applyFilters}
          data-testid="narrative-apply-filters"
        >
          Apply filters
        </button>
        <span className="hint">{entries.length} loaded</span>
      </section>

      {message ? <FormStatus status={message} /> : null}
      {error ? (
        <div data-testid="narrative-review-error">
          <FormStatus status={{ kind: "error", text: error }} />
          <button type="button" onClick={() => void load(applied)}>
            Try again
          </button>
        </div>
      ) : null}

      <div className="review-workspace" data-testid="narrative-review-console">
        <section className="review-queue" aria-label="Narrative passages">
          {loading && !loaded ? (
            <p
              className="hint"
              role="status"
              data-testid="narrative-review-loading"
            >
              Loading narrative passages…
            </p>
          ) : entries.length === 0 ? (
            <p className="hint" data-testid="narrative-review-empty">
              No passages match these filters.
            </p>
          ) : (
            <ul className="review-list" data-testid="narrative-review-list">
              {entries.map((entry) => {
                const passage = entry.passage;
                const summary = linkSummary(entry.links);
                return (
                  <li key={passage.passage_id}>
                    <button
                      type="button"
                      className={
                        selectedId === passage.passage_id
                          ? "review-row selected"
                          : "review-row"
                      }
                      aria-pressed={selectedId === passage.passage_id}
                      onClick={() => setSelectedId(passage.passage_id)}
                      data-testid={`narrative-review-row-${passage.passage_id}`}
                    >
                      <span className="review-row-heading">
                        <strong>
                          {passage.section_title ||
                            `Passage ${passage.passage_id}`}
                        </strong>
                        <span
                          className={`badge narrative-link-${summary.primary}`}
                        >
                          {summary.primaryLabel}
                        </span>
                      </span>
                      <span>
                        {languageLabel(passage.language)} ·{" "}
                        {passage.document_ref} · {pagesLabel(passage)}
                      </span>
                      <span className="hint">
                        Passage {passage.passage_id} ·{" "}
                        {passage.review_status.replace("_", " ")} ·{" "}
                        {passage.visibility.replace("_", " ")}
                      </span>
                      <span className="review-row-value">{summary.text}</span>
                      <span className="review-row-value">
                        {passage.body_excerpt}
                      </span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </section>

        <section
          className="review-detail"
          aria-label="Narrative review detail"
          data-testid="narrative-review-detail"
        >
          {!selectedId ? (
            <p className="hint">
              Select a passage to inspect its wording, source, and person
              candidates.
            </p>
          ) : detailLoading || !detail ? (
            <p className="hint" role="status">
              Loading passage review…
            </p>
          ) : (
            <NarrativeReviewDetailPanel
              detail={detail}
              people={people}
              selectedPerson={selectedPerson}
              onSelectPerson={setSelectedPerson}
              linkNote={linkNote}
              onLinkNote={setLinkNote}
              passageNote={passageNote}
              onPassageNote={setPassageNote}
              busy={busy}
              accepted={accepted}
              onConfirmLink={() => void decideLink(true)}
              onRejectLink={() => void decideLink(false)}
              onApproveFamily={() => void decidePassage("approved", "family")}
              onApproveAdmin={() =>
                void decidePassage("approved", "admin_only")
              }
              onReject={() => void decidePassage("rejected", "admin_only")}
              onReturnPending={() =>
                void decidePassage("pending", "admin_only")
              }
              onOpenSource={() => openCitation(detail.passage)}
            />
          )}
        </section>
      </div>

      <SourceDialog source={openSource} onClose={() => setOpenSource(null)} />
    </>
  );
}

function NarrativeReviewDetailPanel({
  detail,
  people,
  selectedPerson,
  onSelectPerson,
  linkNote,
  onLinkNote,
  passageNote,
  onPassageNote,
  busy,
  accepted,
  onConfirmLink,
  onRejectLink,
  onApproveFamily,
  onApproveAdmin,
  onReject,
  onReturnPending,
  onOpenSource,
}: {
  detail: NarrativeReviewDetailResponse;
  people: Map<string, NarrativeReviewPerson>;
  selectedPerson: string | null;
  onSelectPerson: (personKey: string) => void;
  linkNote: string;
  onLinkNote: (value: string) => void;
  passageNote: string;
  onPassageNote: (value: string) => void;
  busy: boolean;
  accepted: boolean;
  onConfirmLink: () => void;
  onRejectLink: () => void;
  onApproveFamily: () => void;
  onApproveAdmin: () => void;
  onReject: () => void;
  onReturnPending: () => void;
  onOpenSource: () => void;
}) {
  const passage = detail.passage;
  const body = passage.body ?? passage.body_excerpt;
  const paragraphs = body.split(/\n{2,}/);

  return (
    <div data-testid={`narrative-review-detail-${passage.passage_id}`}>
      <header className="review-detail-heading">
        <div>
          <p className="eyebrow">Passage {passage.passage_id}</p>
          <h2>{passage.section_title || "Untitled section"}</h2>
          <p className="hint">
            {languageLabel(passage.language)} · {passage.document_ref} ·{" "}
            {pagesLabel(passage)}
          </p>
        </div>
        <span
          className={`badge narrative-link-${passage.review_status}`}
          data-testid="narrative-passage-status"
        >
          {passage.review_status.replace("_", " ")}
        </span>
      </header>

      <div className="review-meta">
        <span className="badge" data-testid="narrative-passage-visibility">
          {passage.visibility.replace("_", " ")}
        </span>
        <span className="badge">{languageLabel(passage.language)}</span>
        <button
          type="button"
          className="family-text-button"
          onClick={onOpenSource}
          data-testid="narrative-source-button"
        >
          View source citation
        </button>
      </div>

      <p className="hint" data-testid="narrative-source-note">
        Only citation metadata is available here. The original document is not
        served by the archive; the source reference and pages are shown for
        review.
      </p>

      <div
        className="narrative-review-body"
        data-testid="narrative-passage-body"
      >
        {paragraphs.map((paragraph, index) => (
          <p key={index}>{paragraph}</p>
        ))}
      </div>

      <section
        className="review-decision"
        aria-labelledby="narrative-links-heading"
      >
        <h3 id="narrative-links-heading">Person links</h3>
        <p className="hint">
          Confirming or rejecting a link changes only that person association.
          It never approves the passage. An ambiguous match requires an explicit
          person selection.
        </p>
        {detail.links.length === 0 ? (
          <p className="review-blocked" data-testid="narrative-links-unmatched">
            <strong>Unmatched.</strong> No person link exists for this passage.
            It cannot be approved for family until an import or reviewer creates
            a supported link.
          </p>
        ) : (
          <>
            <ul
              className="narrative-link-list"
              data-testid="narrative-link-candidates"
            >
              {detail.links.map((link) => (
                <li
                  key={link.person_key}
                  className={
                    selectedPerson === link.person_key
                      ? "narrative-link-candidate selected"
                      : "narrative-link-candidate"
                  }
                  data-testid={`narrative-link-candidate-${link.person_key}`}
                >
                  <label className="narrative-link-choice">
                    <input
                      type="radio"
                      name="narrative-person-link"
                      value={link.person_key}
                      checked={selectedPerson === link.person_key}
                      onChange={() => onSelectPerson(link.person_key)}
                      data-testid={`narrative-link-radio-${link.person_key}`}
                    />
                    <span>
                      <strong>{link.display_name}</strong>{" "}
                      <span
                        className={`badge narrative-link-${link.link_status}`}
                        data-testid={`narrative-link-status-${link.link_status}`}
                      >
                        {LINK_LABEL[link.link_status] ?? link.link_status}
                      </span>
                    </span>
                  </label>
                  <p className="meta" data-testid="narrative-link-person-key">
                    {link.person_key} · {link.match_kind.replace("_", " ")}
                  </p>
                  {link.evidence_note ? (
                    <p className="hint">{link.evidence_note}</p>
                  ) : null}
                  {link.reviewed_by ? (
                    <p className="meta">
                      Reviewed by {link.reviewed_by}
                      {link.reviewed_at
                        ? ` · ${formatDate(link.reviewed_at)}`
                        : ""}
                    </p>
                  ) : null}
                  <PersonContext person={people.get(link.person_key)} />
                </li>
              ))}
            </ul>
            <label htmlFor="narrative-link-note">
              Link review note (optional)
            </label>
            <textarea
              id="narrative-link-note"
              value={linkNote}
              onChange={(event) => onLinkNote(event.target.value)}
              rows={2}
              maxLength={2000}
              data-testid="narrative-link-note"
            />
            <div className="review-actions">
              <button
                type="button"
                disabled={!selectedPerson || busy}
                onClick={onConfirmLink}
                data-testid="narrative-link-confirm"
              >
                Confirm selected person
              </button>
              <button
                type="button"
                className="button-danger"
                disabled={!selectedPerson || busy}
                onClick={onRejectLink}
                data-testid="narrative-link-reject"
              >
                Reject selected person
              </button>
            </div>
          </>
        )}
      </section>

      <section
        className="review-decision"
        aria-labelledby="narrative-passage-heading"
      >
        <h3 id="narrative-passage-heading">Passage decision</h3>
        <p className="hint">
          Passage review is separate from link review. Opening a passage or
          confirming a link does not approve it.
        </p>
        <label htmlFor="narrative-passage-note">
          Passage review note (optional)
        </label>
        <textarea
          id="narrative-passage-note"
          value={passageNote}
          onChange={(event) => onPassageNote(event.target.value)}
          rows={2}
          maxLength={2000}
          data-testid="narrative-passage-note"
        />
        {!accepted ? (
          <p className="review-blocked" data-testid="narrative-family-blocked">
            <strong>Approve for family is unavailable.</strong> Confirm a
            resolved or ambiguous candidate first; a family-visible passage
            needs an accepted person link.
          </p>
        ) : null}
        <div className="review-actions">
          <button
            type="button"
            disabled={!accepted || busy}
            onClick={onApproveFamily}
            data-testid="narrative-approve-family"
          >
            Approve for family
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={onApproveAdmin}
            data-testid="narrative-approve-admin"
          >
            Approve for admins only
          </button>
          <button
            type="button"
            className="button-danger"
            disabled={busy}
            onClick={onReject}
            data-testid="narrative-reject-passage"
          >
            Reject passage
          </button>
          <button
            type="button"
            className="button-secondary"
            disabled={busy}
            onClick={onReturnPending}
            data-testid="narrative-return-pending"
          >
            Return to pending
          </button>
        </div>
      </section>

      <section
        className="review-history"
        aria-labelledby="narrative-events-heading"
      >
        <h3 id="narrative-events-heading">Audit trail</h3>
        {detail.events.length === 0 ? (
          <p className="hint">No review events yet.</p>
        ) : (
          <ol data-testid="narrative-review-events">
            {detail.events.map((event) => (
              <ReviewEventRow key={event.event_id} event={event} />
            ))}
          </ol>
        )}
      </section>
    </div>
  );
}

function PersonContext({ person }: { person?: NarrativeReviewPerson }) {
  if (!person) return null;
  const lifespan = [person.birth, person.death].filter(Boolean).join("–");
  return (
    <div
      className="narrative-person-context"
      data-testid="narrative-person-context"
    >
      {person.aliases.length > 0 ? (
        <p className="meta">Also known as: {person.aliases.join(", ")}</p>
      ) : null}
      {lifespan ? <p className="meta">Verified lifespan: {lifespan}</p> : null}
      {person.parents.length > 0 ? (
        <p className="meta">Parents: {person.parents.join(", ")}</p>
      ) : null}
      {person.spouses.length > 0 ? (
        <p className="meta">Spouses: {person.spouses.join(", ")}</p>
      ) : null}
      {person.children.length > 0 ? (
        <p className="meta">Children: {person.children.join(", ")}</p>
      ) : null}
    </div>
  );
}

function ReviewEventRow({ event }: { event: NarrativeReviewEvent }) {
  const transition = event.from_status
    ? `${event.from_status} → ${event.to_status}`
    : event.to_status;
  return (
    <li data-testid={`narrative-event-${event.event_id}`}>
      <div className="review-history-heading">
        <strong>{event.action.replace("_", " ")}</strong>
        <span className="meta">{formatDate(event.created_at)}</span>
      </div>
      <p className="meta">
        {transition}
        {event.visibility ? ` · ${event.visibility.replace("_", " ")}` : ""}
        {event.person_key ? ` · ${event.person_key}` : ""}
      </p>
      <p className="meta">Reviewer: {event.actor}</p>
      {event.note ? <p>{event.note}</p> : null}
    </li>
  );
}

function linkSummary(links: NarrativeReviewLink[]): {
  primary: NarrativeLinkStatus | "unmatched";
  primaryLabel: string;
  text: string;
} {
  if (links.length === 0) {
    return {
      primary: "unmatched",
      primaryLabel: "Unmatched",
      text: "No person link — unmatched passage",
    };
  }
  const parts: string[] = [];
  for (const status of LINK_ORDER) {
    const names = links
      .filter((link) => link.link_status === status)
      .map((link) => link.display_name);
    if (names.length > 0)
      parts.push(`${LINK_LABEL[status]}: ${names.join(", ")}`);
  }
  const primary =
    LINK_ORDER.find((status) =>
      links.some((link) => link.link_status === status),
    ) ?? "rejected";
  return {
    primary,
    primaryLabel: LINK_LABEL[primary],
    text: parts.join(" · "),
  };
}

function languageLabel(language: string): string {
  return language === "vi" ? "Vietnamese" : "English";
}

function pagesLabel(passage: NarrativeReviewPassage): string {
  return passage.page_start === passage.page_end
    ? `PDF page ${passage.page_start}`
    : `PDF pages ${passage.page_start}–${passage.page_end}`;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError) {
    if (error.status === 401) return "Your session expired. Sign in again.";
    if (error.status === 403)
      return "Only an administrator may review personal history.";
    if (error.status === 404)
      return "That narrative passage is no longer available.";
    if (error.status === 409) return error.message;
    return error.message;
  }
  return error instanceof Error ? error.message : "The request failed.";
}
