"use client";

import { useEffect, useState } from "react";

import { EvidenceBadge, EvidenceList } from "@/components/EvidenceBadge";
import { canApproveOrRevoke, canNominate } from "@/lib/experience-contract";
import type { HeadOfFamilyResult, Role } from "@/lib/experience-contract";

// The Head of Family view renders only what Rust returned. Each conclusion links
// to its cited evidence by identifier. There is no code path that sets a head
// from a model answer: a head label appears only when Rust recorded an
// appointment backed by evidence.

type Props = {
  role: Role | null;
  result: HeadOfFamilyResult;
  onNominate: (recordId: string, candidateLabel: string) => void;
  onApprove: (nominationId: string) => void;
  onRevoke: (appointmentId: string, reason: string) => void;
};

function ConfirmDialog({
  title,
  description,
  confirmLabel,
  onConfirm,
  onCancel,
}: {
  title: string;
  description: string;
  confirmLabel: string;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  return (
    <div className="confirm" role="dialog" aria-modal="true" data-testid="confirm-dialog">
      <h3>{title}</h3>
      <p>{description}</p>
      <button type="button" onClick={onConfirm} data-testid="confirm-button">
        {confirmLabel}
      </button>
      <button type="button" onClick={onCancel} data-testid="cancel-button">
        Cancel
      </button>
    </div>
  );
}

export function HeadOfFamilyView({ role, result, onNominate, onApprove, onRevoke }: Props) {
  const [pendingNomination, setPendingNomination] = useState<{
    recordId: string;
    candidateLabel: string;
  } | null>(null);
  const [pendingApprove, setPendingApprove] = useState<string | null>(null);
  const [pendingRevoke, setPendingRevoke] = useState<{
    appointmentId: string;
    reason: string;
  } | null>(null);
  const [candidateLabel, setCandidateLabel] = useState("");
  const [revokeReason, setRevokeReason] = useState("");

  useEffect(() => {
    setPendingNomination(null);
    setPendingApprove(null);
    setPendingRevoke(null);
  }, [result.kind]);

  if (result.kind === "denied") {
    return (
      <section className="card denial" role="alert" data-testid="hof-denied">
        <h2>Access denied</h2>
        <p>{result.reason}</p>
      </section>
    );
  }
  if (result.kind === "no_access") {
    return (
      <section className="card denial" role="alert" data-testid="hof-no-access">
        <h2>No access</h2>
        <p>{result.reason}</p>
      </section>
    );
  }
  if (result.kind === "expired") {
    return (
      <section className="card" data-testid="hof-expired" role="alert">
        <h2>Session expired</h2>
        <p>Sign in again to view the Head of Family record.</p>
      </section>
    );
  }

  const view = result.data;
  if (view.catalog === "unavailable") {
    return (
      <section className="card" data-testid="hof-catalog-unavailable">
        <h2>Head of Family</h2>
        <p className="insufficient">
          The genealogy catalog is not available. {view.reason}
        </p>
        <p className="hint">
          No family head is shown until cited evidence is recorded.
        </p>
      </section>
    );
  }

  return (
    <section className="card" data-testid="hof-view">
      <h2>Head of Family</h2>
      <ul className="conclusions">
        {view.conclusions.map((conclusion, index) => (
          <li key={`${conclusion.conclusion}-${index}`} data-testid="hof-conclusion">
            <p>{conclusion.conclusion}</p>
            {conclusion.head_label ? (
              <p className="meta">Cited head: {conclusion.head_label}</p>
            ) : null}
            <EvidenceBadge status={conclusion.evidence_status} />
            <EvidenceList evidence={conclusion.evidence} />

            {canNominate(role) ? (
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  if (candidateLabel.trim() === "") return;
                  setPendingNomination({
                    recordId: conclusion.conclusion,
                    candidateLabel: candidateLabel.trim(),
                  });
                }}
              >
                <label htmlFor={`nominate-${index}`}>Nominate a candidate</label>
                <input
                  id={`nominate-${index}`}
                  value={candidateLabel}
                  onChange={(event) => setCandidateLabel(event.target.value)}
                  data-testid="nomination-input"
                />
                <button type="submit" data-testid="nominate-button">
                  Nominate
                </button>
              </form>
            ) : null}

            {canApproveOrRevoke(role) ? (
              <div className="admin-controls">
                <button
                  type="button"
                  onClick={() => setPendingApprove(conclusion.conclusion)}
                  data-testid="approve-button"
                >
                  Approve nomination
                </button>
                <form
                  onSubmit={(event) => {
                    event.preventDefault();
                    if (revokeReason.trim() === "") return;
                    setPendingRevoke({
                      appointmentId: conclusion.conclusion,
                      reason: revokeReason.trim(),
                    });
                  }}
                >
                  <label htmlFor={`revoke-${index}`}>Revocation reason</label>
                  <input
                    id={`revoke-${index}`}
                    value={revokeReason}
                    onChange={(event) => setRevokeReason(event.target.value)}
                    data-testid="revoke-reason-input"
                  />
                  <button type="submit" data-testid="revoke-button">
                    Revoke appointment
                  </button>
                </form>
              </div>
            ) : null}
          </li>
        ))}
      </ul>

      {pendingNomination ? (
        <ConfirmDialog
          title="Confirm nomination"
          description={`Nominate ${pendingNomination.candidateLabel}?`}
          confirmLabel="Confirm nomination"
          onConfirm={() => {
            onNominate(pendingNomination.recordId, pendingNomination.candidateLabel);
            setPendingNomination(null);
          }}
          onCancel={() => setPendingNomination(null)}
        />
      ) : null}

      {pendingApprove ? (
        <ConfirmDialog
          title="Confirm approval"
          description="Approve this nomination? The Rust API records the appointment."
          confirmLabel="Confirm approval"
          onConfirm={() => {
            onApprove(pendingApprove);
            setPendingApprove(null);
          }}
          onCancel={() => setPendingApprove(null)}
        />
      ) : null}

      {pendingRevoke ? (
        <ConfirmDialog
          title="Confirm revocation"
          description={`Revoke the appointment? Reason: ${pendingRevoke.reason}`}
          confirmLabel="Confirm revocation"
          onConfirm={() => {
            onRevoke(pendingRevoke.appointmentId, pendingRevoke.reason);
            setPendingRevoke(null);
          }}
          onCancel={() => setPendingRevoke(null)}
        />
      ) : null}
    </section>
  );
}
