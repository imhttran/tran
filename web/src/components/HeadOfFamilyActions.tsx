"use client";

import { useEffect, useRef, useState } from "react";

import { canApproveOrRevoke, canNominate } from "@/lib/experience-contract";
import type { Role } from "@/lib/experience-contract";

type PersonMatch = {
  person_key: string;
  display_name: string;
  generation: number | null;
  evidence_status: string;
  birth_year: number | null;
  branch: string | null;
  match: string | null;
  matched_name: string | null;
  recorded_parents: Array<{
    person_key: string;
    display_name: string;
    evidence_status: string;
  }>;
};

type NominationView = {
  nomination_id: number;
  record_id: string;
  status: string;
};

type Pending =
  | { kind: "nominate"; recordId: string; label: string }
  | { kind: "approve"; nominationId: number }
  | { kind: "reject"; nominationId: number };

type SearchState = "idle" | "loading" | "ready" | "empty" | "error";

function csrfToken(): string | null {
  if (typeof document === "undefined") return null;
  const entry = document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith("rag_csrf="));
  return entry ? decodeURIComponent(entry.slice("rag_csrf=".length)) : null;
}

async function post(
  path: string,
  body: unknown,
): Promise<{ ok: boolean; message?: string }> {
  const headers = new Headers({ "content-type": "application/json" });
  const token = csrfToken();
  if (token) headers.set("x-csrf-token", token);
  try {
    const response = await fetch(path, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      let message = `The API returned HTTP ${response.status}.`;
      try {
        const payload = (await response.json()) as { message?: unknown };
        if (typeof payload.message === "string") message = payload.message;
      } catch {
        // Keep the status message.
      }
      return { ok: false, message };
    }
    return { ok: true };
  } catch {
    return { ok: false, message: "The API is unreachable." };
  }
}

function personDetails(person: PersonMatch): string {
  const details: string[] = [];
  if (person.branch) details.push(person.branch);
  if (person.birth_year !== null) details.push(`Born ${person.birth_year}`);
  if (person.generation !== null) details.push(`Generation ${person.generation}`);
  return details.join(" · ");
}

// Real nomination/approval controls backed by the Rust API. Success is shown
// only by reloading the server state; a failed response is displayed instead of
// pretending the mutation happened.
export function HeadOfFamilyActions({ role }: { role: Role | null }) {
  const [nominations, setNominations] = useState<NominationView[]>([]);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<PersonMatch[]>([]);
  const [selectedPerson, setSelectedPerson] = useState<PersonMatch | null>(null);
  const [label, setLabel] = useState("");
  const [searchState, setSearchState] = useState<SearchState>("idle");
  const [searchError, setSearchError] = useState<string | null>(null);
  const [catalogTruncated, setCatalogTruncated] = useState(false);
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const [pending, setPending] = useState<Pending | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const requestSeq = useRef(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const confirmRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!canApproveOrRevoke(role)) {
      return;
    }
    let cancelled = false;
    fetch("/api/genealogy/nominations", { cache: "no-store" })
      .then(async (response) => {
        if (cancelled || !response.ok) return;
        const body = (await response.json()) as {
          nominations?: NominationView[];
        };
        setNominations(
          Array.isArray(body.nominations) ? body.nominations : [],
        );
      })
      .catch(() => {
        if (!cancelled) setError("The genealogy service is unavailable.");
      });
    return () => {
      cancelled = true;
    };
  }, [role]);

  useEffect(() => {
    const term = query.trim();
    if (selectedPerson && term === selectedPerson.display_name) {
      setResults([]);
      setSearchState("idle");
      setOpen(false);
      setActiveIndex(-1);
      return;
    }
    if (term === "") {
      requestSeq.current += 1;
      setResults([]);
      setSearchState("idle");
      setSearchError(null);
      setCatalogTruncated(false);
      setOpen(false);
      setActiveIndex(-1);
      return;
    }
    const seq = requestSeq.current + 1;
    requestSeq.current = seq;
    setSearchState("loading");
    setSearchError(null);
    setCatalogTruncated(false);
    const timer = window.setTimeout(() => {
      void (async () => {
        try {
          const response = await fetch(
            `/api/genealogy/person-search?q=${encodeURIComponent(term)}`,
            { cache: "no-store" },
          );
          if (requestSeq.current !== seq) return;
          if (!response.ok) {
            setResults([]);
            setSearchState("error");
            setSearchError("The person search is unavailable.");
            return;
          }
          const body = (await response.json()) as {
            candidates?: PersonMatch[];
            catalog_truncated?: boolean;
          };
          if (requestSeq.current !== seq) return;
          const candidates = Array.isArray(body.candidates)
            ? body.candidates
            : [];
          setResults(candidates);
          setSearchState(candidates.length > 0 ? "ready" : "empty");
          setCatalogTruncated(Boolean(body.catalog_truncated));
          setActiveIndex(candidates.length > 0 ? 0 : -1);
          setOpen(true);
        } catch {
          if (requestSeq.current !== seq) return;
          setResults([]);
          setSearchState("error");
          setSearchError("The person search is unavailable.");
        }
      })();
    }, 250);
    return () => window.clearTimeout(timer);
  }, [query, selectedPerson]);

  // Move focus to the confirm action when the dialog opens.
  useEffect(() => {
    if (pending) confirmRef.current?.focus();
  }, [pending]);

  function closePending() {
    setPending(null);
    const opener = openerRef.current;
    openerRef.current = null;
    if (opener && typeof opener.focus === "function") opener.focus();
  }

  function handleDialogKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      closePending();
      return;
    }
    if (event.key !== "Tab") return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const focusable = dialog.querySelectorAll<HTMLElement>(
      'button, [href], input, select, textarea, [tabindex]:not([tabindex="-1"])',
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  async function confirm() {
    if (!pending || busy) return;
    setBusy(true);
    const result =
      pending.kind === "nominate"
        ? await post("/api/genealogy/nominations", {
            record_id: pending.recordId,
            candidate_label: pending.label,
          })
        : await post(
            `/api/genealogy/nominations/${encodeURIComponent(String(pending.nominationId))}`,
            { approve: pending.kind === "approve" },
          );
    setBusy(false);
    if (result.ok) {
      window.location.reload();
    } else {
      setError(result.message ?? "The request failed.");
      setPending(null);
    }
  }

  function handleQueryChange(value: string) {
    setQuery(value);
    setOpen(true);
    setResults([]);
    setActiveIndex(-1);
    setSearchError(null);
    setCatalogTruncated(false);
    setSearchState(value.trim() === "" ? "idle" : "loading");
    if (selectedPerson) {
      setSelectedPerson(null);
      setLabel("");
    }
  }

  function selectPerson(person: PersonMatch) {
    setSelectedPerson(person);
    setQuery(person.display_name);
    setLabel(person.display_name);
    setResults([]);
    setOpen(false);
    setActiveIndex(-1);
    setSearchState("idle");
    setSearchError(null);
    setCatalogTruncated(false);
    inputRef.current?.focus();
  }

  function handleSearchKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === "ArrowDown") {
      if (results.length === 0) return;
      event.preventDefault();
      setOpen(true);
      setActiveIndex((current) => (current + 1) % results.length);
      return;
    }
    if (event.key === "ArrowUp") {
      if (results.length === 0) return;
      event.preventDefault();
      setOpen(true);
      setActiveIndex((current) =>
        current <= 0 ? results.length - 1 : current - 1,
      );
      return;
    }
    if (event.key === "Enter") {
      if (open && activeIndex >= 0 && results[activeIndex]) {
        event.preventDefault();
        selectPerson(results[activeIndex]);
      }
      return;
    }
    if (event.key === "Escape" && open) {
      event.preventDefault();
      setOpen(false);
      setActiveIndex(-1);
    }
  }

  function handleSearchBlur(event: React.FocusEvent<HTMLDivElement>) {
    const next = event.relatedTarget as Node | null;
    if (!next || !event.currentTarget.contains(next)) {
      setOpen(false);
      setActiveIndex(-1);
    }
  }

  const nominee = canNominate(role);
  const approver = canApproveOrRevoke(role);
  if (!nominee && !approver) return null;

  return (
    <section className="card hof-actions" data-testid="hof-actions">
      <h2>Nomination and approval</h2>
      <p className="hint">
        A nomination records a candidate for the Head of Family designation. It
        does not appoint anyone. An administrator approves or revokes with cited
        evidence.
      </p>
      {error ? (
        <p
          className="insufficient"
          role="alert"
          data-testid="hof-actions-error"
        >
          {error}
        </p>
      ) : null}

      {nominee ? (
        <form
          className="hof-form hof-actions-form"
          onSubmit={(event) => {
            event.preventDefault();
            if (!selectedPerson || label.trim() === "") return;
            openerRef.current = document.activeElement as HTMLElement | null;
            setPending({
              kind: "nominate",
              recordId: selectedPerson.person_key,
              label: label.trim(),
            });
          }}
        >
          <div className="hof-person-search" onBlur={handleSearchBlur}>
            <label htmlFor="hof-record-select">Catalog record</label>
            <input
              ref={inputRef}
              id="hof-record-select"
              data-testid="hof-record-select"
              role="combobox"
              aria-expanded={open}
              aria-controls="hof-person-results"
              aria-autocomplete="list"
              aria-activedescendant={
                open && activeIndex >= 0
                  ? `hof-person-option-${activeIndex}`
                  : undefined
              }
              autoComplete="off"
              placeholder="Search by name"
              value={query}
              onChange={(event) => handleQueryChange(event.target.value)}
              onFocus={() => {
                if (query.trim() !== "") setOpen(true);
              }}
              onKeyDown={handleSearchKeyDown}
            />
            {selectedPerson ? (
              <p className="hof-selected-person" data-testid="hof-selected-person">
                Selected: <strong>{selectedPerson.display_name}</strong>
                {personDetails(selectedPerson) ? (
                  <span> · {personDetails(selectedPerson)}</span>
                ) : null}
              </p>
            ) : null}
            {open && query.trim() !== "" ? (
              <div className="hof-search-popover">
                {searchState === "loading" ? (
                  <p className="hint" role="status" data-testid="hof-search-loading">
                    Searching…
                  </p>
                ) : null}
                {searchState === "error" ? (
                  <p
                    className="insufficient"
                    role="alert"
                    data-testid="hof-search-error"
                  >
                    {searchError}
                  </p>
                ) : null}
                {searchState === "empty" ? (
                  <p className="hint" role="status" data-testid="hof-search-empty">
                    No matching people.
                  </p>
                ) : null}
                {searchState === "ready" && results.length > 0 ? (
                  <ul
                    id="hof-person-results"
                    role="listbox"
                    aria-label="Matching people"
                    className="hof-person-results"
                    data-testid="hof-record-results"
                  >
                    {results.map((person, index) => (
                      <li
                        key={person.person_key}
                        id={`hof-person-option-${index}`}
                        role="option"
                        aria-selected={index === activeIndex}
                        data-testid="hof-record-option"
                        className={
                          index === activeIndex
                            ? "hof-person-option is-active"
                            : "hof-person-option"
                        }
                        onMouseDown={(event) => event.preventDefault()}
                        onClick={() => selectPerson(person)}
                      >
                        <span className="hof-person-name">
                          {person.display_name}
                        </span>
                        {personDetails(person) ? (
                          <span className="hof-person-meta">
                            {personDetails(person)}
                          </span>
                        ) : null}
                      </li>
                    ))}
                  </ul>
                ) : null}
                {catalogTruncated ? (
                  <p className="hint" data-testid="hof-search-truncated">
                    Some catalog records are outside the search index; refine
                    the name.
                  </p>
                ) : null}
              </div>
            ) : null}
          </div>
          <label htmlFor="hof-nomination-label">Candidate label</label>
          <input
            id="hof-nomination-label"
            data-testid="hof-nomination-label"
            value={label}
            onChange={(event) => setLabel(event.target.value)}
          />
          <button
            type="submit"
            data-testid="hof-nominate-submit"
            disabled={!selectedPerson || label.trim() === "" || busy}
          >
            Nominate
          </button>
        </form>
      ) : null}

      {approver ? (
        <div className="hof-pending-group">
          <h3>Pending nominations</h3>
          {nominations.length === 0 ? (
            <p className="hint" data-testid="hof-no-nominations">
              No pending nominations.
            </p>
          ) : (
            <ul className="hof-pending-list">
              {nominations.map((nomination) => (
                <li
                  key={nomination.nomination_id}
                  className="hof-pending-row"
                  data-testid="hof-pending-nomination"
                >
                  <div className="hof-pending-person">
                    <strong>{nomination.record_id}</strong>
                    <span className="hint">{nomination.status}</span>
                  </div>
                  <div className="hof-pending-actions">
                    <button
                      type="button"
                      data-testid="hof-approve-submit"
                      onClick={(event) => {
                        openerRef.current = event.currentTarget;
                        setPending({
                          kind: "approve",
                          nominationId: nomination.nomination_id,
                        });
                      }}
                    >
                      Approve
                    </button>
                    <button
                      type="button"
                      className="button-danger"
                      data-testid="hof-reject-submit"
                      onClick={(event) => {
                        openerRef.current = event.currentTarget;
                        setPending({
                          kind: "reject",
                          nominationId: nomination.nomination_id,
                        });
                      }}
                    >
                      Reject
                    </button>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      {pending ? (
        <div
          ref={dialogRef}
          className="confirm hof-confirm-dialog"
          role="dialog"
          aria-modal="true"
          onKeyDown={handleDialogKeyDown}
          data-testid="hof-confirm"
        >
          <p>
            {pending.kind === "nominate"
              ? `Nominate ${pending.label}?`
              : pending.kind === "approve"
                ? "Approve this nomination?"
                : "Reject this nomination?"}
          </p>
          <div className="hof-confirm-actions">
            <button
              ref={confirmRef}
              type="button"
              className="hof-confirm-action"
              onClick={() => void confirm()}
              data-testid="hof-confirm-yes"
            >
              Confirm
            </button>
            <button
              type="button"
              className="button-secondary hof-confirm-action"
              onClick={closePending}
              data-testid="hof-confirm-cancel"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : null}
    </section>
  );
}
