"use client";

import { useState } from "react";

import { EvidenceBadge, EvidenceList } from "@/components/EvidenceBadge";
import { SourceDialog } from "@/components/SourceDetails";
import type { OpenSource } from "@/components/SourceDetails";
import type {
  PersonAnswer,
  PersonAssertion,
  PersonCandidate,
  PersonMatchKind,
} from "@/lib/api";

// Rendering for a name question answered from the genealogy store. Every value
// comes from a stored assertion with its recorded review status; nothing here
// infers a relative, a biography, or a verification.

const ASSERTION_LABELS: Record<string, string> = {
  display_name: "Recorded name",
  name_variant: "Recorded name variant",
  branch: "Recorded branch",
  parent_child: "Recorded parent link",
  spouse: "Recorded spouse link",
};

function assertionLabel(kind: string): string {
  return ASSERTION_LABELS[kind] ?? kind.replace(/_/g, " ");
}

function reviewStatusLabel(status: string): string {
  if (status === "verified") return "Verified";
  if (status === "needs_review") return "Needs review";
  if (status === "disputed") return "Disputed";
  return "Unverified";
}

function reviewStatusClass(status: string): string {
  if (status === "verified") return "verified";
  if (status === "needs_review") return "needs_review";
  if (status === "disputed") return "disputed";
  return "unverified";
}

function matchNote(match: PersonMatchKind | null): string | null {
  if (match === "alias") return "Matched a recorded name variant.";
  if (match === "reordered") {
    return "Matched the recorded name tokens in a different order; treat this as a candidate.";
  }
  if (match === "partial") {
    return "Matched part of the recorded name; treat this as a candidate and check the profile.";
  }
  return null;
}

function CandidateFacts({ candidate }: { candidate: PersonCandidate }) {
  return (
    <>
      {candidate.generation !== null && (
        <p className="meta">Recorded generation: {candidate.generation}</p>
      )}
      {candidate.matched_name && candidate.match !== "exact" && (
        <p className="meta">
          Matched the recorded spelling “{candidate.matched_name}”.
        </p>
      )}
      {candidate.recorded_parents.length > 0 && (
        <p className="meta">
          Recorded parent
          {candidate.recorded_parents.length > 1 ? "s" : ""}:{" "}
          {candidate.recorded_parents
            .map((parent) => {
              const status =
                parent.evidence_status === "verified"
                  ? ""
                  : ` (${parent.evidence_status.replace("_", " ")})`;
              return `${parent.display_name}${status}`;
            })
            .join(", ")}
        </p>
      )}
      {matchNote(candidate.match) && (
        <p className="meta">{matchNote(candidate.match)}</p>
      )}
    </>
  );
}

function AssertionRows({
  assertions,
  onOpen,
}: {
  assertions: PersonAssertion[];
  onOpen: (source: OpenSource) => void;
}) {
  return (
    <ul className="person-assertions" data-testid="person-assertions">
      {assertions.map((assertion) => (
        <li key={assertion.assertion_id}>
          <div className="person-assertion-head">
            <span>{assertionLabel(assertion.kind)}</span>
            <span
              className={`badge evidence-${reviewStatusClass(assertion.review_status)}`}
              data-testid={`assertion-status-${reviewStatusClass(assertion.review_status)}`}
            >
              {reviewStatusLabel(assertion.review_status)}
            </span>
            {assertion.authority ? (
              <span className="badge" data-testid="assertion-authoritative">
                Authoritative
              </span>
            ) : null}
          </div>
          {assertion.authority ? (
            <p className="meta" data-testid="assertion-authority-reason">
              {`Set as the current version by decision ${assertion.authority.decision_id}: “${assertion.authority.reason}”.`}
            </p>
          ) : null}
          {assertion.value_text && <p>{assertion.value_text}</p>}
          {assertion.review_status !== "verified" && (
            <p className="meta">
              This is a recorded candidate, not a confirmed fact.
            </p>
          )}
          <EvidenceList
            evidence={assertion.evidence}
            onOpen={(evidence) =>
              onOpen({
                evidence,
                citedFor: [
                  {
                    label: assertionLabel(assertion.kind),
                    status: reviewStatusLabel(assertion.review_status),
                  },
                ],
              })
            }
          />
        </li>
      ))}
    </ul>
  );
}

export function PersonAnswerView({
  person,
  busy,
  onSelect,
}: {
  person: PersonAnswer;
  busy: boolean;
  onSelect: (personKey: string) => void;
}) {
  const [source, setSource] = useState<OpenSource | null>(null);

  if (person.outcome === "none") {
    return (
      <section className="card person-answer" data-testid="person-none">
        <h2>No matching person</h2>
        {person.catalog === "empty" ? (
          <p className="meta" data-testid="person-catalog-empty">
            The family catalog has not been imported yet, so person questions
            cannot be answered. An operator must import the family catalog
            before these questions work.
          </p>
        ) : (
          <p className="meta">
            No person matching “{person.query_name}” is recorded in the family
            archive. A missing person record is separate from whether the
            archive documents mention that name.
          </p>
        )}
        {person.truncated && (
          <p className="meta" data-testid="person-truncated">
            The family catalog is larger than this screen can search, so another
            person may not have been checked.
          </p>
        )}
      </section>
    );
  }

  if (person.outcome === "ambiguous") {
    return (
      <section className="card person-answer" data-testid="person-ambiguous">
        <h2>Several possible matches</h2>
        <p className="meta">
          “{person.query_name}” matches more than one recorded person. Choose
          the person you mean. The archive never merges people who share a name.
        </p>
        <ul className="person-candidates">
          {person.candidates.map((candidate) => (
            <li key={candidate.person_key} className="person-candidate">
              <div className="person-assertion-head">
                <strong>{candidate.display_name}</strong>
                <EvidenceBadge status={candidate.evidence_status} />
              </div>
              <CandidateFacts candidate={candidate} />
              <button
                type="button"
                data-testid="person-candidate"
                disabled={busy}
                onClick={() => onSelect(candidate.person_key)}
              >
                Choose this person
              </button>
            </li>
          ))}
        </ul>
        {person.candidates_truncated && (
          <p className="meta" data-testid="person-candidates-truncated">
            More people matched than can be listed here. Narrow the name or
            check the family tree.
          </p>
        )}
      </section>
    );
  }

  const selected = person.selected;
  if (!selected) {
    return null;
  }

  return (
    <section className="card person-answer" data-testid="person-found">
      <h2>{selected.display_name}</h2>
      <div className="person-assertion-head">
        <EvidenceBadge status={selected.evidence_status} />
        {selected.generation !== null && (
          <span className="meta">
            Recorded generation: {selected.generation}
          </span>
        )}
      </div>
      {matchNote(selected.match) && (
        <p className="meta">{matchNote(selected.match)}</p>
      )}
      {selected.name_only ? (
        <p className="meta" data-testid="person-name-only">
          Only the name is recorded for this person. No other approved detail is
          available, so no biography can be given.
        </p>
      ) : (
        <>
          <p className="meta">
            These are the stored assertions. Each keeps its recorded review
            status and source citation; an unverified assertion is not a
            confirmed fact.
          </p>
          <AssertionRows assertions={selected.assertions} onOpen={setSource} />
        </>
      )}
      <a
        className="person-profile-link"
        href={selected.profile_url}
        data-testid="person-profile-link"
      >
        View family profile
      </a>
      {person.truncated && (
        <p className="meta" data-testid="person-truncated">
          The family catalog is larger than this screen can search, so another
          person with the same name may exist.
        </p>
      )}
      <SourceDialog source={source} onClose={() => setSource(null)} />
    </section>
  );
}
