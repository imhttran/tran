"use client";

import { useEffect, useRef, useState } from "react";

export type NominationPerson = {
  record_id: string;
  person_label: string;
  birth?: string | null;
  branches: string[];
};

// Search text never becomes an identity. Only a returned record can be selected.
export function NominationPersonSelector({
  onSelect,
  endpoint = "/api/genealogy/nomination-persons",
  isDisabled,
  disabledText = "Unavailable",
  idPrefix = "hof",
}: {
  onSelect: (person: NominationPerson | null) => void;
  endpoint?: string;
  isDisabled?: (person: NominationPerson) => boolean;
  disabledText?: string;
  // Element ids must be unique when several selectors share a page.
  idPrefix?: string;
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(false);
  const [results, setResults] = useState<NominationPerson[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const sequence = useRef(0);

  function isBlocked(person: NominationPerson): boolean {
    return isDisabled ? isDisabled(person) : false;
  }

  useEffect(() => {
    const version = ++sequence.current;
    const controller = new AbortController();
    const term = query.trim();
    if (!term || selected) return;
    const timer = setTimeout(async () => {
      try {
        const response = await fetch(
          `${endpoint}?q=${encodeURIComponent(term)}&limit=20`,
          {
            cache: "no-store",
            signal: controller.signal,
          },
        );
        if (!response.ok)
          throw new Error(`Person search failed (HTTP ${response.status}).`);
        const body = (await response.json()) as { people?: NominationPerson[] };
        if (version !== sequence.current) return;
        setResults(Array.isArray(body.people) ? body.people : []);
      } catch (err) {
        if (version !== sequence.current || controller.signal.aborted) return;
        setError(err instanceof Error ? err.message : "Person search failed.");
      } finally {
        if (version === sequence.current) setLoading(false);
      }
    }, 250);
    return () => {
      ++sequence.current;
      clearTimeout(timer);
      controller.abort();
    };
  }, [query, selected, endpoint]);

  function choose(person: NominationPerson) {
    if (isBlocked(person)) return;
    ++sequence.current;
    setQuery(person.person_label);
    setSelected(true);
    setOpen(false);
    setLoading(false);
    onSelect(person);
  }

  return (
    <div className="person-selector">
      <label htmlFor={`${idPrefix}-record-select`}>Search for a person</label>
      <input
        id={`${idPrefix}-record-select`}
        data-testid="hof-record-select"
        role="combobox"
        aria-autocomplete="list"
        aria-expanded={open}
        aria-controls={`${idPrefix}-person-results`}
        aria-activedescendant={
          open && active >= 0 ? `${idPrefix}-person-${active}` : undefined
        }
        aria-describedby={`${idPrefix}-person-help`}
        autoComplete="off"
        maxLength={200}
        value={query}
        onChange={(event) => {
          ++sequence.current;
          const value = event.target.value;
          setQuery(value);
          setSelected(false);
          onSelect(null);
          setResults([]);
          setActive(-1);
          setError(null);
          setLoading(Boolean(value.trim()));
          setOpen(Boolean(value.trim()));
        }}
        onFocus={() => {
          if (query.trim() && !selected) setOpen(true);
        }}
        onBlur={() => setOpen(false)}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            setOpen(true);
            setActive((current) =>
              results.length
                ? current < 0
                  ? event.key === "ArrowDown"
                    ? 0
                    : results.length - 1
                  : (current +
                      (event.key === "ArrowDown" ? 1 : -1) +
                      results.length) %
                    results.length
                : -1,
            );
          } else if (event.key === "Enter" && !selected) {
            event.preventDefault();
            if (open && active >= 0 && results[active]) choose(results[active]);
          } else if (event.key === "Escape") {
            event.preventDefault();
            setOpen(false);
            setActive(-1);
          }
        }}
      />
      <p className="hint" id={`${idPrefix}-person-help`}>
        Type part of a name, with or without Vietnamese accents, then select a
        person. Search does not establish eligibility.
      </p>
      <p className="hint" id={`${idPrefix}-person-status`} role="status">
        {loading
          ? "Searching…"
          : error
            ? error
            : selected
              ? "Person selected."
              : query.trim() && !results.length
                ? "No people found."
                : results.length
                  ? `${results.length} matches. Refine your search if needed.`
                  : ""}
      </p>
      {open && results.length > 0 && (
        <ul id={`${idPrefix}-person-results`} role="listbox" aria-label="Matching people">
          {results.map((person, index) => {
            const blocked = isBlocked(person);
            return (
              <li
                key={person.record_id}
                id={`${idPrefix}-person-${index}`}
                role="option"
                aria-selected={active === index}
                aria-disabled={blocked ? true : undefined}
                data-disabled={blocked ? "true" : undefined}
                onMouseDown={(event) => event.preventDefault()}
                onClick={() => choose(person)}
                onMouseMove={() => setActive(index)}
              >
                <strong>{person.person_label}</strong>
                {person.branches.length > 0 && (
                  <span> — {person.branches.join(", ")}</span>
                )}
                {person.birth && <span> — Born {person.birth}</span>}
                <span className="hint"> — {person.record_id}</span>
                {blocked && <span className="hint"> — {disabledText}</span>}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
