"use client";

import { useCallback, useEffect, useMemo, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  applyDocumentAuthority,
  listAuthorityDecisions,
  listDocumentSubmissions,
  previewDocumentAuthority,
  reverseAuthorityDecision,
  type AuthorityAssertion,
  type AuthorityDecision,
  type AuthorityPreview,
  type DocumentSubmission,
} from "@/lib/api";
import { canAdmin, type Role } from "@/lib/experience-contract";

// Admin console for the "current source of truth" decision.
//
// Nothing here verifies a claim. A decision changes which stored version the
// current views read; the superseded version keeps its value, citations, and
// review history, and a reversal is a new audited decision.

const EXTRACTABLE = /\.(md|markdown|txt)$/i;

type Scope = "document" | "person" | "assertions";

const KIND_LABEL: Record<string, string> = {
  display_name: "Recorded name",
  name_variant: "Name variant",
  parent_child: "Parent link",
  spouse: "Spouse link",
  social_link: "Social link",
  birth_date: "Birth date",
  death_date: "Death date",
  branch: "Branch",
};

function kindLabel(kind: string): string {
  return KIND_LABEL[kind] ?? kind.replace(/_/g, " ");
}

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

function citationLabel(assertion: AuthorityAssertion): string {
  const first = assertion.citations[0];
  if (!first) return "No current citation";
  const extra =
    assertion.citations.length > 1
      ? ` (+${assertion.citations.length - 1} more)`
      : "";
  return `${first.label}${first.withdrawn ? " — withdrawn" : ""}${extra}`;
}

function valueLabel(assertion: AuthorityAssertion): string {
  return assertion.value_text ?? "—";
}

function AssertionLine({ assertion }: { assertion: AuthorityAssertion }) {
  return (
    <span data-testid="authority-assertion">
      <strong>{assertion.display_name}</strong> · {kindLabel(assertion.kind)}:{" "}
      {valueLabel(assertion)} ·{" "}
      <span className="meta">
        {assertion.review_status.replace("_", " ")} · {citationLabel(assertion)}
      </span>
    </span>
  );
}

export default function SourceOfTruthPage() {
  return (
    <ExperienceShell>
      {(role) => <AuthorityContent role={role} />}
    </ExperienceShell>
  );
}

function AuthorityContent({ role }: { role: Role | null }) {
  const [submissions, setSubmissions] = useState<DocumentSubmission[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [scope, setScope] = useState<Scope>("document");
  const [personKey, setPersonKey] = useState("");
  const [selectedAssertions, setSelectedAssertions] = useState<number[]>([]);
  const [removals, setRemovals] = useState<number[]>([]);
  const [preview, setPreview] = useState<AuthorityPreview | null>(null);
  const [decisions, setDecisions] = useState<AuthorityDecision[]>([]);
  const [reason, setReason] = useState("");
  const [reversalReasons, setReversalReasons] = useState<
    Record<number, string>
  >({});
  const [confirm, setConfirm] = useState(false);
  const [acknowledge, setAcknowledge] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

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

  const loadDecisions = useCallback(async () => {
    if (!canAdmin(role)) return;
    try {
      const body = await listAuthorityDecisions();
      setDecisions(body.decisions);
    } catch (err) {
      setError(errorMessage(err));
    }
  }, [role]);

  useEffect(() => {
    void loadSubmissions();
    void loadDecisions();
  }, [loadSubmissions, loadDecisions]);

  function scopeBody() {
    return {
      scope,
      person_key: scope === "person" ? personKey.trim() : undefined,
      assertion_ids: scope === "assertions" ? selectedAssertions : [],
      remove_assertion_ids: removals,
    };
  }

  async function refreshPreview(overrides?: {
    scope?: Scope;
    assertions?: number[];
  }) {
    if (selectedId === null) return;
    const nextScope = overrides?.scope ?? scope;
    const nextAssertions = overrides?.assertions ?? selectedAssertions;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await previewDocumentAuthority(selectedId, {
        scope: nextScope,
        person_key: nextScope === "person" ? personKey.trim() : undefined,
        assertion_ids: nextScope === "assertions" ? nextAssertions : [],
        remove_assertion_ids: removals,
      });
      setPreview(result.preview);
      setConfirm(false);
      setAcknowledge(false);
    } catch (err) {
      setPreview(null);
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function apply() {
    if (!preview || selectedId === null) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await applyDocumentAuthority(selectedId, {
        ...scopeBody(),
        reason,
        preview_hash: preview.preview_hash,
        confirm,
        acknowledge_conflicts: acknowledge,
      });
      setMessage(
        `Decision ${result.decision.decision_id} applied: ${result.decision.applied} made current, ${result.decision.superseded} superseded, ${result.decision.removed} removed.`,
      );
      setReason("");
      setConfirm(false);
      setAcknowledge(false);
      setRemovals([]);
      setPreview(null);
      await loadDecisions();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function reverse(decisionId: number) {
    const reversalReason = (reversalReasons[decisionId] ?? "").trim();
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const result = await reverseAuthorityDecision(decisionId, reversalReason);
      setMessage(
        `Decision ${decisionId} reversed by decision ${result.decision.decision_id}. The earlier version is current again; both records are kept.`,
      );
      setReversalReasons((current) => ({ ...current, [decisionId]: "" }));
      await loadDecisions();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  function toggleRemoval(assertionId: number) {
    setRemovals((current) =>
      current.includes(assertionId)
        ? current.filter((id) => id !== assertionId)
        : [...current, assertionId],
    );
  }

  function toggleAssertion(assertionId: number) {
    setSelectedAssertions((current) =>
      current.includes(assertionId)
        ? current.filter((id) => id !== assertionId)
        : [...current, assertionId],
    );
  }

  if (!canAdmin(role)) {
    return (
      <DenialView
        kind="denied"
        reason="Source-of-truth decisions are restricted to administrators."
      />
    );
  }

  const changed = preview
    ? preview.items.filter((item) => item.action !== "unchanged").length
    : 0;

  return (
    <>
      <PageTitle title="Current source of truth" />
      <PageHeader
        title="Current source of truth"
        subtitle="Designate an approved document version as authoritative for a document, a person, or selected records."
      />
      {error ? (
        <p className="insufficient" role="alert" data-testid="authority-error">
          {error}
        </p>
      ) : null}
      {message ? (
        <p className="form-status ok" data-testid="authority-message">
          {message}
        </p>
      ) : null}

      <section className="status-panel" data-testid="authority-intro">
        <h2>What a decision does</h2>
        <p className="hint">
          The designated version becomes the one the family tree, the family
          profile, and archive answers read. A conflicting current record is
          superseded, not deleted: its value, its citations, and its review
          history stay available in the history below and can be restored by a
          reversal.
        </p>
        <p className="hint">
          <strong>Authoritative is not verified.</strong> This decision changes
          which version is current. Verifying a claim still needs its own
          evidence review.
        </p>
        <p className="hint">
          Information the document omits is never treated as deleted. Only the
          records you explicitly tick for removal are superseded.
        </p>
      </section>

      {eligible.length === 0 ? (
        <p className="hint" data-testid="authority-none">
          No approved and processed Markdown or text document is available.
          Approve and process a submission, then extract its family records.
        </p>
      ) : (
        <section className="card">
          <label htmlFor="authority-document">Document</label>
          <select
            id="authority-document"
            data-testid="authority-document"
            value={selectedId ?? ""}
            onChange={(event) => {
              setSelectedId(Number(event.target.value));
              setPreview(null);
              setSelectedAssertions([]);
              setRemovals([]);
            }}
          >
            {eligible.map((submission) => (
              <option key={submission.version_id} value={submission.version_id}>
                {submission.title} (v{submission.version_number})
              </option>
            ))}
          </select>
          {selected ? (
            <p className="hint" data-testid="authority-source">
              Source: {selected.source} · {selected.original_filename} ·{" "}
              {selected.content_sha256.slice(0, 12)}…
            </p>
          ) : null}

          <fieldset>
            <legend>Scope</legend>
            {(
              [
                ["document", "Whole document"],
                ["person", "One person"],
                ["assertions", "Selected records"],
              ] as Array<[Scope, string]>
            ).map(([value, label]) => (
              <label key={value} style={{ marginRight: 16 }}>
                <input
                  type="radio"
                  name="authority-scope"
                  data-testid={`authority-scope-${value}`}
                  value={value}
                  checked={scope === value}
                  onChange={() => {
                    setScope(value);
                    setPreview(null);
                    setMessage(null);
                  }}
                />{" "}
                {label}
              </label>
            ))}
          </fieldset>

          {scope === "person" ? (
            <div className="proposal-value">
              <label htmlFor="authority-person">Person key</label>
              <input
                id="authority-person"
                type="text"
                data-testid="authority-person"
                placeholder="P123"
                value={personKey}
                onChange={(event) => setPersonKey(event.target.value)}
              />
              <p className="hint">
                Use the stable person key from the person profile, not a name.
              </p>
            </div>
          ) : null}

          {scope === "assertions" && preview ? (
            <fieldset className="proposal-candidates">
              <legend>Records this decision governs</legend>
              {preview.items
                .filter((item) => item.incoming)
                .map((item) => {
                  const assertion = item.incoming as AuthorityAssertion;
                  return (
                    <label key={assertion.assertion_id}>
                      <input
                        type="checkbox"
                        data-testid="authority-assertion-choice"
                        checked={selectedAssertions.includes(
                          assertion.assertion_id,
                        )}
                        onChange={() => toggleAssertion(assertion.assertion_id)}
                      />{" "}
                      <AssertionLine assertion={assertion} />
                    </label>
                  );
                })}
              <p className="hint">
                Choose at least one record, then preview again to see exactly
                what the decision changes.
              </p>
            </fieldset>
          ) : null}

          <div className="proposal-actions">
            <button
              type="button"
              data-testid="authority-preview"
              disabled={busy || selectedId === null}
              onClick={() => void refreshPreview()}
            >
              Preview effect
            </button>
            {scope === "assertions" && selectedAssertions.length > 0 ? (
              <button
                type="button"
                className="button-secondary"
                data-testid="authority-preview-selection"
                disabled={busy}
                onClick={() =>
                  void refreshPreview({
                    scope: "assertions",
                    assertions: selectedAssertions,
                  })
                }
              >
                Preview selected records ({selectedAssertions.length})
              </button>
            ) : null}
          </div>
        </section>
      )}

      {preview ? (
        <section className="card" data-testid="authority-preview-panel">
          <h2>Effect</h2>
          <p className="meta" data-testid="authority-preview-summary">
            Scope {preview.scope} · {preview.items.length} record(s) · {changed}{" "}
            change(s) · {preview.omissions.length} omission(s) ·{" "}
            {preview.conflicts.length} conflict(s)
          </p>
          <p className="hint">
            Source: {preview.document_ref}. Confirmation token:{" "}
            <code>{preview.preview_hash}</code>
          </p>

          {preview.items.map((item) => (
            <article
              className="proposal-card"
              key={`${item.action}-${item.incoming?.assertion_id ?? item.replaces[0]?.assertion_id ?? 0}`}
              data-testid="authority-item"
              data-action={item.action}
            >
              <div className="proposal-head">
                <strong>{item.action}</strong>
                {item.incoming ? (
                  <AssertionLine assertion={item.incoming} />
                ) : null}
              </div>
              {item.replaces.map((previous) => (
                <p className="meta" key={previous.assertion_id}>
                  Replaces: <AssertionLine assertion={previous} />
                </p>
              ))}
            </article>
          ))}

          {preview.omissions.length > 0 ? (
            <fieldset className="proposal-candidates">
              <legend>
                Current records this document does not mention (kept unless you
                remove them)
              </legend>
              {preview.omissions.map((assertion) => (
                <label key={assertion.assertion_id}>
                  <input
                    type="checkbox"
                    data-testid="authority-omission"
                    checked={removals.includes(assertion.assertion_id)}
                    onChange={() => toggleRemoval(assertion.assertion_id)}
                  />{" "}
                  Remove <AssertionLine assertion={assertion} />
                </label>
              ))}
              <p className="hint">
                Omission is not deletion. Ticking a record makes it an explicit
                removal, which is recorded and reversible.
              </p>
            </fieldset>
          ) : null}

          {preview.conflicts.length > 0 ? (
            <div className="insufficient" data-testid="authority-conflicts">
              <p>
                {preview.conflicts.length} reviewed record(s) would lose
                authority. Their values stay in the history.
              </p>
              <ul>
                {preview.conflicts.map((conflict) => (
                  <li
                    key={`${conflict.code}-${conflict.assertion.assertion_id}`}
                  >
                    {conflict.code.replace(/_/g, " ")}:{" "}
                    <AssertionLine assertion={conflict.assertion} />
                  </li>
                ))}
              </ul>
            </div>
          ) : null}

          {removals.length > 0 ? (
            <p className="hint" data-testid="authority-removals-pending">
              {removals.length} record(s) selected for removal. Preview again to
              confirm the effect.
            </p>
          ) : null}

          <div className="proposal-value">
            <label htmlFor="authority-reason">
              Reason (recorded with the decision)
            </label>
            <textarea
              id="authority-reason"
              data-testid="authority-reason"
              rows={3}
              value={reason}
              onChange={(event) => setReason(event.target.value)}
            />
          </div>
          <label>
            <input
              type="checkbox"
              data-testid="authority-confirm"
              checked={confirm}
              onChange={(event) => setConfirm(event.target.checked)}
            />{" "}
            I confirm this scoped effect.
          </label>
          {preview.conflicts.length > 0 ? (
            <label>
              <input
                type="checkbox"
                data-testid="authority-acknowledge"
                checked={acknowledge}
                onChange={(event) => setAcknowledge(event.target.checked)}
              />{" "}
              I acknowledge that reviewed records lose authority.
            </label>
          ) : null}
          <div className="proposal-actions">
            <button
              type="button"
              data-testid="authority-apply"
              disabled={
                busy ||
                !confirm ||
                reason.trim() === "" ||
                (preview.conflicts.length > 0 && !acknowledge)
              }
              onClick={() => void apply()}
            >
              Set as current source of truth
            </button>
          </div>
        </section>
      ) : null}

      <section className="card" data-testid="authority-history">
        <h2>Decision history</h2>
        {decisions.length === 0 ? (
          <p className="hint" data-testid="authority-history-empty">
            No source-of-truth decision has been made.
          </p>
        ) : null}
        {decisions.map((decision) => (
          <article
            className="proposal-card"
            key={decision.decision_id}
            data-testid="authority-decision"
            data-status={decision.status}
          >
            <div className="proposal-head">
              <strong>Decision {decision.decision_id}</strong>
              <span className={`badge proposal-status-${decision.status}`}>
                {decision.status}
              </span>
              {decision.needs_attention ? (
                <span
                  className="badge badge-feedback"
                  data-testid="authority-attention-badge"
                >
                  Needs attention
                </span>
              ) : null}
              <span className="meta">
                {decision.kind === "reversal" ? "Reversal" : "Designation"} ·{" "}
                {decision.scope} scope
                {decision.person_key ? ` · ${decision.person_key}` : ""}
              </span>
            </div>
            <p className="meta">
              {decision.document_ref} · by {decision.actor} ·{" "}
              {decision.created_at}
            </p>
            <p>
              <strong>Reason:</strong> {decision.reason}
            </p>
            {decision.reverses_decision_id !== null ? (
              <p className="meta">
                Reverses decision {decision.reverses_decision_id}.
              </p>
            ) : null}
            {decision.supersedes_decision_id !== null ? (
              <p className="meta">
                Supersedes decision {decision.supersedes_decision_id}.
              </p>
            ) : null}
            <ul>
              {decision.items.map((item) => (
                <li key={`${item.action}-${item.incoming?.assertion_id ?? 0}`}>
                  <strong>{item.action}</strong>{" "}
                  {item.incoming ? (
                    <AssertionLine assertion={item.incoming} />
                  ) : item.replaces[0] ? (
                    <AssertionLine assertion={item.replaces[0]} />
                  ) : null}
                  {item.incoming
                    ? item.replaces.map((previous) => (
                        <span className="meta" key={previous.assertion_id}>
                          {" "}
                          — replaced: <AssertionLine assertion={previous} />
                        </span>
                      ))
                    : null}
                </li>
              ))}
            </ul>
            {decision.needs_attention ? (
              <div
                className="insufficient"
                data-testid="authority-needs-attention"
              >
                <p>
                  This decision needs attention: the source document was
                  removed, so {decision.unsupported_assertions.length} current
                  record(s) no longer have supporting evidence. They are not
                  shown in the tree, search, profile, or archive answers.
                </p>
                <p className="hint">
                  Reverse this decision to restore the previous version, or
                  designate another reviewed source. Nothing was deleted and no
                  review status changed.
                </p>
                <ul>
                  {decision.unsupported_assertions.map((assertion) => (
                    <li key={assertion.assertion_id}>
                      <AssertionLine assertion={assertion} />
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            {decision.status === "active" ? (
              <div className="proposal-value">
                <label htmlFor={`reverse-${decision.decision_id}`}>
                  Reversal reason
                </label>
                <input
                  id={`reverse-${decision.decision_id}`}
                  type="text"
                  data-testid="authority-reverse-reason"
                  value={reversalReasons[decision.decision_id] ?? ""}
                  onChange={(event) =>
                    setReversalReasons((current) => ({
                      ...current,
                      [decision.decision_id]: event.target.value,
                    }))
                  }
                />
                <div className="proposal-actions">
                  <button
                    type="button"
                    className="button-secondary"
                    data-testid="authority-reverse"
                    disabled={
                      busy ||
                      (reversalReasons[decision.decision_id] ?? "").trim() ===
                        ""
                    }
                    onClick={() => void reverse(decision.decision_id)}
                  >
                    Reverse this decision
                  </button>
                </div>
              </div>
            ) : null}
          </article>
        ))}
      </section>
    </>
  );
}
