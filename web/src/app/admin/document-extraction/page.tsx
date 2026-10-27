"use client";

import { useCallback, useEffect, useMemo, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  applyDocumentExtraction,
  decideExtractionProposal,
  extractDocumentRecords,
  getDocumentExtraction,
  listDocumentSubmissions,
  type DocumentExtraction,
  type DocumentSubmission,
  type ExtractionProposal,
} from "@/lib/api";
import { canAdmin, type Role } from "@/lib/experience-contract";

// Admin review screen for the reviewed document-to-genealogy workflow.
//
// Extraction only produces drafts. Nothing reaches the family tree until a
// reviewer approves an individual proposal and applies the extraction, and an
// applied assertion is always unverified: extraction approval is not evidence
// review.

const EXTRACTABLE = /\.(md|markdown|txt)$/i;

const KIND_LABEL: Record<ExtractionProposal["kind"], string> = {
  person: "Person",
  parent_child: "Parent link",
  spouse: "Spouse link",
  alias: "Alias",
  social_link: "Social link",
};

function isEligible(submission: DocumentSubmission): boolean {
  return (
    submission.review_status === "approved" &&
    submission.processing_status === "ready" &&
    submission.removed_at === null &&
    submission.access_scope === "family" &&
    EXTRACTABLE.test(submission.original_filename)
  );
}

function errorMessage(error: unknown): string {
  if (error instanceof ApiRequestError) return error.message;
  return error instanceof Error ? error.message : "The request failed.";
}

function resolutionLabel(proposal: ExtractionProposal): string {
  if (proposal.kind !== "person") return "";
  if (proposal.resolution === "create") return "New person";
  if (proposal.resolution === "matched") {
    return proposal.candidates[0]?.display_name ?? "Existing person";
  }
  if (proposal.resolution === "ambiguous") return "Choose a person";
  return "No recorded match";
}

export default function DocumentExtractionPage() {
  return (
    <ExperienceShell>
      {(role) => <ExtractionContent role={role} />}
    </ExperienceShell>
  );
}

function ExtractionContent({ role }: { role: Role | null }) {
  const [submissions, setSubmissions] = useState<DocumentSubmission[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [extraction, setExtraction] = useState<DocumentExtraction | null>(null);
  const [choice, setChoice] = useState<Record<number, string>>({});
  const [valueEdits, setValueEdits] = useState<Record<number, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);

  const eligible = useMemo(() => submissions.filter(isEligible), [submissions]);
  const selected = eligible.find(
    (submission) => submission.version_id === selectedId,
  );

  const loadSubmissions = useCallback(async () => {
    if (!canAdmin(role)) return;
    try {
      const body = await listDocumentSubmissions();
      setSubmissions(body.submissions);
      const first = body.submissions.find(isEligible);
      setSelectedId((current) => current ?? first?.version_id ?? null);
    } catch (err) {
      setError(errorMessage(err));
    }
  }, [role]);

  const loadExtraction = useCallback(async (versionId: number) => {
    setLoading(true);
    setError(null);
    try {
      const body = await getDocumentExtraction(versionId);
      setExtraction(body);
      setChoice({});
      setValueEdits({});
    } catch (err) {
      setError(errorMessage(err));
      setExtraction(null);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void loadSubmissions();
  }, [loadSubmissions]);

  useEffect(() => {
    if (selectedId === null) {
      setExtraction(null);
      return;
    }
    void loadExtraction(selectedId);
  }, [selectedId, loadExtraction]);

  async function act(action: () => Promise<unknown>, success: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await action();
      setMessage(success);
      if (selectedId !== null) await loadExtraction(selectedId);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  function decide(proposal: ExtractionProposal, decision: "approve" | "reject") {
    const body: {
      decision: "approve" | "reject";
      person_key?: string;
      create_person?: boolean;
      value_text?: string;
    } = { decision };
    if (proposal.kind === "person" && decision === "approve") {
      const selectedChoice = choice[proposal.proposal_id];
      if (selectedChoice === "create") {
        body.create_person = true;
      } else if (selectedChoice?.startsWith("person:")) {
        body.person_key = selectedChoice.slice("person:".length);
      }
    }
    if (
      (proposal.kind === "alias" || proposal.kind === "social_link") &&
      decision === "approve"
    ) {
      const edited = valueEdits[proposal.proposal_id];
      if (edited !== undefined) body.value_text = edited.trim();
    }
    void act(
      () => decideExtractionProposal(proposal.proposal_id, body),
      decision === "approve"
        ? `Proposal ${proposal.proposal_id} approved. Apply the extraction to write it.`
        : `Proposal ${proposal.proposal_id} rejected.`,
    );
  }

  if (!canAdmin(role)) {
    return (
      <DenialView
        kind="denied"
        reason="Family record extraction is restricted to administrators."
      />
    );
  }

  const approvedCount =
    extraction?.proposals.filter((proposal) => proposal.status === "approved")
      .length ?? 0;

  return (
    <>
      <PageTitle title="Family record extraction" />
      <PageHeader
        title="Family record extraction"
        subtitle="Turn an approved family document into reviewed genealogy proposals."
      />
      {error ? (
        <p
          className="insufficient state-panel state-panel-unavailable"
          role="alert"
          data-testid="extraction-error"
        >
          {error}
        </p>
      ) : null}
      {message ? (
        <p className="form-status ok" data-testid="extraction-message">
          {message}
        </p>
      ) : null}

      {loading && !extraction ? (
        <p className="hint state-panel state-panel-loading" role="status">
          Loading extraction proposals…
        </p>
      ) : null}

      <section className="status-panel" data-testid="extraction-page">
        <h2>How this works</h2>
        <p className="hint">
          Extraction reads the document text into draft proposals. Nothing is
          added to the family tree until you approve a proposal and apply the
          extraction. Applying writes <strong>unverified</strong> assertions:
          they keep the document reference and the exact supporting line, and
          verifying them is a separate review.
        </p>
        <p className="hint">
          <strong>Also known as</strong> is a searchable name, not a link. A
          social field becomes a link only when the document states a full
          http(s) URL; a bare handle is never turned into a URL.
        </p>
      </section>

      {eligible.length === 0 ? (
        <p
          className="hint state-panel state-panel-empty"
          data-testid="extraction-none"
        >
          No document is ready for extraction. Approve and process a Markdown or
          text submission first.
        </p>
      ) : (
        <section className="card">
          <label htmlFor="extraction-document">Document</label>
          <select
            id="extraction-document"
            data-testid="extraction-document"
            value={selectedId ?? ""}
            onChange={(event) => setSelectedId(Number(event.target.value))}
          >
            {eligible.map((submission) => (
              <option key={submission.version_id} value={submission.version_id}>
                {submission.title} (v{submission.version_number})
              </option>
            ))}
          </select>
          {selected ? (
            <p className="hint" data-testid="extraction-source">
              Source: {selected.source} · {selected.original_filename} ·{" "}
              {selected.content_sha256.slice(0, 12)}…
            </p>
          ) : null}
          {!extraction && !loading ? (
            <button
              type="button"
              data-testid="extract-button"
              disabled={busy || selectedId === null}
              onClick={() =>
                selectedId !== null &&
                void act(
                  () => extractDocumentRecords(selectedId),
                  "Draft proposals created. Review each one below.",
                )
              }
            >
              Extract family records
            </button>
          ) : null}
        </section>
      )}

      {extraction ? (
        <section className="card" data-testid="extraction-panel">
          <h2>Proposals</h2>
          <p className="meta" data-testid="extraction-status">
            Extraction {extraction.extraction.extraction_id} ·{" "}
            {extraction.extraction.status} · {extraction.proposals.length}{" "}
            proposal(s)
          </p>
          {extraction.proposals.map((proposal) => {
            const decided =
              proposal.status === "rejected" || proposal.status === "applied";
            const value =
              valueEdits[proposal.proposal_id] ?? proposal.value_text ?? "";
            const choiceValue = choice[proposal.proposal_id] ?? "";
            return (
              <article
                className="proposal-card"
                key={proposal.proposal_id}
                data-testid="proposal"
                data-kind={proposal.kind}
              >
                <div className="proposal-head">
                  <strong data-testid="proposal-kind">
                    {KIND_LABEL[proposal.kind]}
                  </strong>
                  <span className="meta" data-testid="proposal-subject">
                    {proposal.kind === "parent_child" ||
                    proposal.kind === "spouse"
                      ? `${proposal.subject_name} → ${proposal.target_name}`
                      : proposal.subject_name}
                  </span>
                  <span
                    className={`badge status-chip proposal-status-${proposal.status}`}
                    data-testid="proposal-status"
                  >
                    {proposal.status}
                  </span>
                  {resolutionLabel(proposal) ? (
                    <span className="meta" data-testid="proposal-resolution">
                      {resolutionLabel(proposal)}
                    </span>
                  ) : null}
                </div>

                <blockquote
                  className="extraction-support"
                  data-testid="proposal-support"
                >
                  {proposal.support_text}
                </blockquote>

                {proposal.kind === "person" && !decided ? (
                  proposal.resolution === "matched" ? (
                    <p className="meta" data-testid="proposal-match">
                      Matches{" "}
                      {proposal.candidates[0]?.display_name ??
                        "the recorded person"}{" "}
                      — approve to keep this person, or create a new one.
                    </p>
                  ) : (
                    <fieldset className="proposal-candidates">
                      <legend>Choose the person</legend>
                      {proposal.candidates.map((candidate) => (
                        <label key={candidate.person_key}>
                          <input
                            type="radio"
                            name={`choice-${proposal.proposal_id}`}
                            data-testid="proposal-candidate"
                            value={`person:${candidate.person_key}`}
                            checked={
                              choiceValue === `person:${candidate.person_key}`
                            }
                            onChange={() =>
                              setChoice((current) => ({
                                ...current,
                                [proposal.proposal_id]: `person:${candidate.person_key}`,
                              }))
                            }
                          />
                          {candidate.display_name}{" "}
                          <span className="meta">
                            ({candidate.evidence_status.replace("_", " ")} ·{" "}
                            {candidate.match_kind})
                          </span>
                        </label>
                      ))}
                      <label>
                        <input
                          type="radio"
                          name={`choice-${proposal.proposal_id}`}
                          data-testid="proposal-create"
                          value="create"
                          checked={choiceValue === "create"}
                          onChange={() =>
                            setChoice((current) => ({
                              ...current,
                              [proposal.proposal_id]: "create",
                            }))
                          }
                        />
                        Create a new person
                      </label>
                    </fieldset>
                  )
                ) : null}

                {proposal.kind === "person" &&
                proposal.resolution === "matched" &&
                !decided ? (
                  <label className="proposal-value">
                    <input
                      type="radio"
                      name={`choice-${proposal.proposal_id}`}
                      data-testid="proposal-create"
                      value="create"
                      checked={choiceValue === "create"}
                      onChange={() =>
                        setChoice((current) => ({
                          ...current,
                          [proposal.proposal_id]: "create",
                        }))
                      }
                    />
                    Create a new person instead
                  </label>
                ) : null}

                {(proposal.kind === "alias" ||
                  proposal.kind === "social_link") &&
                !decided ? (
                  <div className="proposal-value">
                    <label htmlFor={`value-${proposal.proposal_id}`}>
                      {proposal.kind === "alias"
                        ? "Alias"
                        : "URL (http or https)"}
                    </label>
                    <input
                      id={`value-${proposal.proposal_id}`}
                      type="text"
                      data-testid="proposal-value"
                      value={value}
                      onChange={(event) =>
                        setValueEdits((current) => ({
                          ...current,
                          [proposal.proposal_id]: event.target.value,
                        }))
                      }
                    />
                  </div>
                ) : null}

                {proposal.kind === "social_link" && decided ? (
                  <p className="meta" data-testid="proposal-url">
                    {proposal.value_text}
                  </p>
                ) : null}

                {!decided ? (
                  <div className="proposal-actions">
                    <button
                      type="button"
                      data-testid="proposal-approve"
                      disabled={busy}
                      onClick={() => decide(proposal, "approve")}
                    >
                      Approve
                    </button>
                    <button
                      type="button"
                      className="button-secondary"
                      data-testid="proposal-reject"
                      disabled={busy}
                      onClick={() => decide(proposal, "reject")}
                    >
                      Reject
                    </button>
                  </div>
                ) : null}
              </article>
            );
          })}

          <button
            type="button"
            data-testid="apply-extraction"
            disabled={busy || approvedCount === 0}
            onClick={() =>
              selectedId !== null &&
              void act(
                () => applyDocumentExtraction(selectedId),
                "Approved proposals applied as unverified genealogy records.",
              )
            }
          >
            Apply approved proposals ({approvedCount})
          </button>
        </section>
      ) : null}
    </>
  );
}
