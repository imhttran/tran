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
  searchNarrativeReviewPeople,
  type NarrativeLinkFilter,
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
  type NarrativeVisibilityFilter,
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

const LINK_FILTER_OPTIONS: {
  value: NarrativeLinkFilter;
  label: string;
}[] = [
  { value: "all", label: "All person links" },
  { value: "needs_confirmation", label: "Needs confirmation" },
  { value: "no_accepted", label: "No accepted links" },
  { value: "has_accepted", label: "Has accepted links" },
  { value: "no_candidates", label: "No candidates" },
];

const LINK_FILTER_HELP =
  "Needs confirmation: at least one ambiguous link. No accepted links: no resolved or confirmed link. Has accepted links: at least one resolved or confirmed link. No candidates: no person links.";

const VISIBILITY_OPTIONS: {
  value: NarrativeVisibilityFilter;
  label: string;
}[] = [
  { value: "all", label: "All visibility" },
  { value: "admin_only", label: "Admin only" },
  { value: "family", label: "Family" },
];

const PAGE_SIZE = 10;
const SEARCH_DEBOUNCE_MS = 300;
const PERSON_SEARCH_DEBOUNCE_MS = 300;

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

// Map a link/passage status onto the shared semantic chip variants. Every
// mapped value reuses an existing stage-3 status-* token; unknown states fall
// back to the neutral draft chip so no new primitive is invented.
const CHIP_CLASS: Record<string, string> = {
  confirmed: "status-approved",
  resolved: "status-draft",
  ambiguous: "status-pending_review",
  rejected: "status-rejected",
  unmatched: "status-rejected",
  approved: "status-approved",
  pending: "status-pending_review",
};

function chipClass(status: string): string {
  return `status-chip ${CHIP_CLASS[status] ?? "status-draft"}`;
}

type Filters = {
  status: NarrativeReviewStatus;
  language: "all" | "vi" | "en";
  person: string;
  link: NarrativeLinkFilter;
  visibility: NarrativeVisibilityFilter;
  document: string;
  pageFrom: string;
  pageTo: string;
  search: string;
};

const DEFAULT_FILTERS: Filters = {
  status: "pending",
  language: "all",
  person: "",
  link: "all",
  visibility: "all",
  document: "",
  pageFrom: "",
  pageTo: "",
  search: "",
};

function parsePageRange(filters: Filters): {
  from?: number;
  to?: number;
  error?: string;
} {
  const parse = (value: string): number | undefined => {
    if (!value.trim()) return undefined;
    const parsed = Number(value);
    return Number.isInteger(parsed) && parsed >= 1 ? parsed : Number.NaN;
  };
  const from = parse(filters.pageFrom);
  const to = parse(filters.pageTo);
  if (Number.isNaN(from) || Number.isNaN(to)) {
    return { error: "Page numbers must be positive whole numbers." };
  }
  if (from != null && to != null && from > to) {
    return {
      error: "The first page must be less than or equal to the last page.",
    };
  }
  return { from, to };
}

export function NarrativeReviewConsole({ role }: { role: Role | null }) {
  const [filters, setFilters] = useState<Filters>(DEFAULT_FILTERS);
  const [applied, setApplied] = useState<Filters>(DEFAULT_FILTERS);
  const [entries, setEntries] = useState<NarrativeReviewQueueEntry[]>([]);
  const [documents, setDocuments] = useState<string[]>([]);
  const [total, setTotal] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [page, setPage] = useState(1);
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
  const [personQuery, setPersonQuery] = useState("");
  const [personResults, setPersonResults] = useState<NarrativeReviewPerson[]>(
    [],
  );
  const [personLoading, setPersonLoading] = useState(false);
  const [personError, setPersonError] = useState<string | null>(null);
  const [personOpen, setPersonOpen] = useState(false);
  const [selectedPersonLabel, setSelectedPersonLabel] = useState("");
  const loadSequence = useRef(0);
  const personSequence = useRef(0);
  const latestFilters = useRef(filters);

  useEffect(() => {
    latestFilters.current = filters;
  }, [filters]);

  const load = useCallback(async (current: Filters, pageNumber: number) => {
    const sequence = ++loadSequence.current;
    const range = parsePageRange(current);
    if (range.error) {
      setError(range.error);
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      const body = await getNarrativeReviewQueue({
        status: current.status,
        language: current.language,
        person: current.person,
        link: current.link,
        visibility: current.visibility,
        document: current.document,
        pageFrom: range.from,
        pageTo: range.to,
        search: current.search,
        offset: (pageNumber - 1) * PAGE_SIZE,
        limit: PAGE_SIZE,
      });
      if (sequence !== loadSequence.current) return;
      setEntries(body.entries);
      setDocuments(body.documents);
      setTotal(body.total);
      setTruncated(body.truncated);
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
    void load(applied, page);
  }, [role, applied, page, load]);

  // Person search is separate from the passage text search. Typing only
  // changes the suggestions; the queue is filtered only after a person is
  // selected, and the exact person key is what reaches the server.
  useEffect(() => {
    if (role !== "admin") return;
    const term = personQuery.trim();
    if (!term || (filters.person && term === selectedPersonLabel)) {
      setPersonResults([]);
      setPersonOpen(false);
      setPersonLoading(false);
      setPersonError(null);
      return;
    }
    const sequence = ++personSequence.current;
    const timer = window.setTimeout(() => {
      setPersonLoading(true);
      setPersonError(null);
      searchNarrativeReviewPeople(term)
        .then((body) => {
          if (sequence !== personSequence.current) return;
          setPersonResults(body.people);
          setPersonOpen(true);
        })
        .catch((err) => {
          if (sequence !== personSequence.current) return;
          setPersonError(messageOf(err));
          setPersonResults([]);
          setPersonOpen(true);
        })
        .finally(() => {
          if (sequence === personSequence.current) setPersonLoading(false);
        });
    }, PERSON_SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [personQuery, role, filters.person, selectedPersonLabel]);

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

  const applyFilters = useCallback((next: Filters) => {
    const range = parsePageRange(next);
    if (range.error) {
      setError(range.error);
      return;
    }
    setError(null);
    setSelectedId(null);
    setDetail(null);
    setMessage(null);
    setPage(1);
    setApplied({ ...next });
  }, []);

  const clearFilters = useCallback(() => {
    setFilters({ ...DEFAULT_FILTERS });
    setApplied({ ...DEFAULT_FILTERS });
    setPage(1);
    setSelectedId(null);
    setDetail(null);
    setMessage(null);
    setError(null);
    setPersonQuery("");
    setPersonResults([]);
    setPersonOpen(false);
    setPersonError(null);
    setSelectedPersonLabel("");
  }, []);

  function updateFilters(patch: Partial<Filters>) {
    setFilters((current) => ({ ...current, ...patch }));
  }

  function selectPerson(person: NarrativeReviewPerson) {
    const next = { ...filters, person: person.person_key };
    setFilters(next);
    setPersonQuery(person.display_name);
    setSelectedPersonLabel(person.display_name);
    setPersonResults([]);
    setPersonOpen(false);
    setPersonError(null);
    applyFilters(next);
  }

  function clearPersonFilter() {
    const next = { ...filters, person: "" };
    setFilters(next);
    setPersonQuery("");
    setSelectedPersonLabel("");
    setPersonResults([]);
    setPersonOpen(false);
    setPersonError(null);
    applyFilters(next);
  }

  // Search is debounced so a request is not sent for every keystroke. The
  // sequence guard in `load` ensures an older response cannot replace a newer
  // one when the debounce or another filter changes the query again.
  useEffect(() => {
    if (role !== "admin" || filters.search === applied.search) return;
    const timer = window.setTimeout(() => {
      applyFilters(latestFilters.current);
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [filters.search, applied.search, role, applyFilters]);

  async function reloadAfterConflict(passageId: number) {
    try {
      const body = await getNarrativeReviewDetail(passageId);
      setDetail(body);
    } catch {
      setDetail(null);
      setSelectedId(null);
    }
    await load(applied, page);
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
      await load(applied, page);
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
      await load(applied, page);
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

  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const firstShown = total === 0 ? 0 : (page - 1) * PAGE_SIZE + 1;
  const lastShown = Math.min(page * PAGE_SIZE, total);
  const activeFilterLabels = [
    `Status: ${
      STATUS_OPTIONS.find((option) => option.value === applied.status)?.label ??
      applied.status
    }`,
    applied.search ? `Search: “${applied.search}”` : null,
    applied.link !== "all"
      ? `Links: ${
          LINK_FILTER_OPTIONS.find((option) => option.value === applied.link)
            ?.label ?? applied.link
        }`
      : null,
    applied.language !== "all"
      ? `Language: ${
          LANGUAGE_OPTIONS.find((option) => option.value === applied.language)
            ?.label ?? applied.language
        }`
      : null,
    applied.visibility !== "all"
      ? `Visibility: ${
          VISIBILITY_OPTIONS.find(
            (option) => option.value === applied.visibility,
          )?.label ?? applied.visibility
        }`
      : null,
    applied.document ? `Document: ${applied.document}` : null,
    applied.pageFrom || applied.pageTo
      ? `Pages: ${applied.pageFrom || "any"}–${applied.pageTo || "any"}`
      : null,
    applied.person
      ? `Person: ${
          selectedPersonLabel
            ? `${selectedPersonLabel} (${applied.person})`
            : applied.person
        }`
      : null,
  ].filter((label): label is string => Boolean(label));

  function changePage(next: number) {
    setPage(Math.max(1, Math.min(totalPages, next)));
    setSelectedId(null);
    setDetail(null);
  }

  return (
    <>
      <PageTitle title="Personal history review" />
      <PageHeader
        title="Personal history review"
        subtitle="Inspect source-backed passages, confirm or reject person links, and approve family visibility. Link decisions and passage decisions are separate and audited."
      />

      <section
        className="narrative-review-filters narrative-surface"
        data-testid="narrative-review-filters"
        aria-label="Narrative review filters"
      >
        <div className="narrative-section-head">
          <h2>Filters</h2>
          <p className="hint">
            Narrow the queue by status, person, source, and pages. Filters
            apply to the passage queue only.
          </p>
        </div>
        <div className="narrative-filter-grid">
          <div className="narrative-filter-field">
            <label htmlFor="narrative-status-filter">Review status</label>
            <select
              id="narrative-status-filter"
              value={filters.status}
              onChange={(event) =>
                updateFilters({
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
          </div>
          <div className="narrative-filter-field narrative-filter-search">
            <label htmlFor="narrative-search-filter">
              Search names, sections, or text
            </label>
            <input
              id="narrative-search-filter"
              type="search"
              value={filters.search}
              placeholder="Person name, section title, or passage text"
              maxLength={200}
              onChange={(event) =>
                updateFilters({ search: event.target.value })
              }
              data-testid="narrative-search-filter"
            />
          </div>
          <div className="narrative-filter-field">
            <label htmlFor="narrative-link-filter">Person links</label>
            <select
              id="narrative-link-filter"
              value={filters.link}
              aria-describedby="narrative-link-help"
              onChange={(event) =>
                updateFilters({
                  link: event.target.value as NarrativeLinkFilter,
                })
              }
              data-testid="narrative-link-filter"
            >
              {LINK_FILTER_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
            <p id="narrative-link-help" className="hint">
              {LINK_FILTER_HELP}
            </p>
          </div>
          <div className="narrative-filter-field">
            <label htmlFor="narrative-language-filter">Language</label>
            <select
              id="narrative-language-filter"
              value={filters.language}
              onChange={(event) =>
                updateFilters({
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
          </div>
          <div className="narrative-filter-field">
            <label htmlFor="narrative-visibility-filter">Visibility</label>
            <select
              id="narrative-visibility-filter"
              value={filters.visibility}
              onChange={(event) =>
                updateFilters({
                  visibility: event.target.value as NarrativeVisibilityFilter,
                })
              }
              data-testid="narrative-visibility-filter"
            >
              {VISIBILITY_OPTIONS.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </div>
          <div className="narrative-filter-field narrative-filter-document">
            <label htmlFor="narrative-document-filter">Source document</label>
            <select
              id="narrative-document-filter"
              value={filters.document}
              onChange={(event) =>
                updateFilters({ document: event.target.value })
              }
              data-testid="narrative-document-filter"
            >
              <option value="">All documents</option>
              {documents.map((document) => (
                <option key={document} value={document}>
                  {document}
                </option>
              ))}
              {filters.document && !documents.includes(filters.document) ? (
                <option value={filters.document}>{filters.document}</option>
              ) : null}
            </select>
          </div>
          <div className="narrative-filter-field narrative-filter-page">
            <label htmlFor="narrative-page-from-filter">Pages from</label>
            <input
              id="narrative-page-from-filter"
              type="number"
              min={1}
              inputMode="numeric"
              value={filters.pageFrom}
              onChange={(event) =>
                updateFilters({ pageFrom: event.target.value })
              }
              data-testid="narrative-page-from-filter"
            />
          </div>
          <div className="narrative-filter-field narrative-filter-page">
            <label htmlFor="narrative-page-to-filter">Pages to</label>
            <input
              id="narrative-page-to-filter"
              type="number"
              min={1}
              inputMode="numeric"
              value={filters.pageTo}
              onChange={(event) =>
                updateFilters({ pageTo: event.target.value })
              }
              data-testid="narrative-page-to-filter"
            />
          </div>
          <div className="narrative-filter-field narrative-filter-person">
            <label htmlFor="narrative-person-search">Person</label>
            <input
              id="narrative-person-search"
              type="search"
              value={personQuery}
              placeholder="Name, alias, or person key"
              autoComplete="off"
              aria-describedby="narrative-person-help"
              onChange={(event) => {
                setPersonQuery(event.target.value);
                setPersonOpen(true);
              }}
              onFocus={() => {
                if (personResults.length > 0) setPersonOpen(true);
              }}
              data-testid="narrative-person-search"
            />
            <p id="narrative-person-help" className="hint">
              Search names, aliases, or keys. The queue is filtered only after
              you select a person.
            </p>
            {filters.person ? (
              <p
                className="narrative-selected-person"
                data-testid="narrative-selected-person"
              >
                Selected: {selectedPersonLabel || filters.person} (
                {filters.person}){" "}
                <button
                  type="button"
                  className="family-text-button"
                  onClick={clearPersonFilter}
                  data-testid="narrative-person-clear"
                >
                  Clear person
                </button>
              </p>
            ) : null}
            {personLoading ? (
              <p
                className="hint"
                role="status"
                data-testid="narrative-person-loading"
              >
                Searching people…
              </p>
            ) : null}
            {personError ? (
              <p
                className="hint"
                role="alert"
                data-testid="narrative-person-error"
              >
                {personError}
              </p>
            ) : null}
            {personOpen && !personLoading && !personError ? (
              personResults.length > 0 ? (
                <ul
                  className="narrative-person-results"
                  role="list"
                  data-testid="narrative-person-results"
                >
                  {personResults.map((person) => (
                    <li key={person.person_key}>
                      <button
                        type="button"
                        className="narrative-person-option"
                        onClick={() => selectPerson(person)}
                        data-testid={`narrative-person-option-${person.person_key}`}
                      >
                        <strong>{person.display_name}</strong>
                        <span className="meta">{person.person_key}</span>
                        <span className="meta">
                          {personContextSummary(person)}
                        </span>
                      </button>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="hint" data-testid="narrative-person-empty">
                  No matching people.
                </p>
              )
            ) : null}
          </div>
        </div>
        <div className="narrative-filter-actions">
          <button
            type="button"
            onClick={() => applyFilters(filters)}
            data-testid="narrative-apply-filters"
          >
            Apply filters
          </button>
          <button
            type="button"
            className="button-secondary"
            onClick={clearFilters}
            data-testid="narrative-clear-filters"
          >
            Clear filters
          </button>
        </div>
      </section>

      <div
        className="narrative-filter-summary"
        data-testid="narrative-active-filters"
      >
        <p
          className="hint"
          role="status"
          aria-live="polite"
          data-testid="narrative-match-count"
        >
          {total} matching {total === 1 ? "passage" : "passages"}
          {total > 0 ? ` · showing ${firstShown}–${lastShown}` : ""}
        </p>
        <ul
          className="narrative-active-filter-list"
          aria-label="Active filters"
        >
          {activeFilterLabels.map((label) => (
            <li key={label} className="narrative-active-filter">
              {label}
            </li>
          ))}
        </ul>
      </div>

      {message ? <FormStatus status={message} /> : null}
      {error ? (
        <div data-testid="narrative-review-error">
          <FormStatus status={{ kind: "error", text: error }} />
          <button type="button" onClick={() => void load(applied, page)}>
            Try again
          </button>
        </div>
      ) : null}

      <div className="review-workspace" data-testid="narrative-review-console">
        <section className="review-queue" aria-label="Narrative passages">
          <div className="review-queue-heading">
            <h2>Passage queue</h2>
            <span className="review-queue-count" data-testid="narrative-queue-count">
              {total} {total === 1 ? "passage" : "passages"}
            </span>
          </div>
          {loading && loaded ? (
            <p
              className="hint"
              role="status"
              data-testid="narrative-review-updating"
            >
              Updating passages…
            </p>
          ) : null}
          {loading && !loaded ? (
            <div
              className="state-panel state-panel-loading"
              role="status"
              data-testid="narrative-review-loading"
            >
              <h2>Loading narrative passages</h2>
              <p className="hint">The queue is being fetched.</p>
            </div>
          ) : entries.length === 0 ? (
            <div
              className="state-panel state-panel-empty narrative-empty"
              data-testid="narrative-review-empty"
            >
              <h2>No passages match these filters</h2>
              <p className="hint">
                Adjust the filters, or clear them to see every status.
              </p>
              <button
                type="button"
                className="button-secondary"
                onClick={clearFilters}
                data-testid="narrative-empty-clear"
              >
                Clear filters
              </button>
            </div>
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
                          className={chipClass(summary.primary)}
                          data-testid="narrative-row-link-chip"
                        >
                          {summary.primaryLabel}
                        </span>
                      </span>
                      <span className="review-row-meta">
                        {languageLabel(passage.language)} ·{" "}
                        {passage.document_ref} · {pagesLabel(passage)}
                      </span>
                      <span className="review-row-chips">
                        <span className={chipClass(passage.review_status)}>
                          {passage.review_status.replace("_", " ")}
                        </span>
                        <span className="status-chip status-draft">
                          {passage.visibility.replace("_", " ")}
                        </span>
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
          {total > 0 ? (
            <nav
              className="narrative-pagination"
              aria-label="Narrative passage pages"
              data-testid="narrative-pagination"
            >
              <button
                type="button"
                className="button-secondary"
                disabled={page <= 1 || loading}
                onClick={() => changePage(page - 1)}
                data-testid="narrative-page-prev"
              >
                Previous
              </button>
              <span data-testid="narrative-page-label">
                Page {page} of {totalPages}
              </span>
              <button
                type="button"
                className="button-secondary"
                disabled={page >= totalPages || loading || !truncated}
                onClick={() => changePage(page + 1)}
                data-testid="narrative-page-next"
              >
                Next
              </button>
            </nav>
          ) : null}
        </section>

        <section
          className="review-detail"
          aria-label="Narrative review detail"
          data-testid="narrative-review-detail"
        >
          {!selectedId ? (
            <div className="review-placeholder">
              <h2>Review detail</h2>
              <p className="hint">
                Select a passage to inspect its wording, source, and person
                candidates.
              </p>
            </div>
          ) : detailLoading || !detail ? (
            <div className="state-panel state-panel-loading" role="status">
              <h2>Loading passage review</h2>
              <p className="hint">The passage detail is being fetched.</p>
            </div>
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
          className={chipClass(passage.review_status)}
          data-testid="narrative-passage-status"
        >
          {passage.review_status.replace("_", " ")}
        </span>
      </header>

      <div className="review-meta">
        <span
          className="status-chip status-draft"
          data-testid="narrative-passage-visibility"
        >
          {passage.visibility.replace("_", " ")}
        </span>
        <span className="status-chip status-draft">
          {languageLabel(passage.language)}
        </span>
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
                        className={chipClass(link.link_status)}
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
                className="button-danger review-action-destructive"
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
            className="button-secondary"
            disabled={busy}
            onClick={onApproveAdmin}
            data-testid="narrative-approve-admin"
          >
            Approve for admins only
          </button>
          <button
            type="button"
            className="button-danger review-action-destructive"
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

function personContextSummary(person: NarrativeReviewPerson): string {
  const lifespan = [person.birth, person.death].filter(Boolean).join("–");
  if (lifespan) return `Lifespan: ${lifespan}`;
  if (person.parents.length > 0) return `Parents: ${person.parents.join(", ")}`;
  if (person.aliases.length > 0) {
    return `Also known as: ${person.aliases.join(", ")}`;
  }
  return "No recorded lifespan or parent context";
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
