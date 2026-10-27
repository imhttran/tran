"use client";

import { useEffect, useRef, useState } from "react";

import { EvidenceBadge, EvidenceList } from "@/components/EvidenceBadge";
import { SourceDialog } from "@/components/SourceDetails";
import type { OpenSource } from "@/components/SourceDetails";
import {
  canApproveOrRevoke,
  canNominate,
  evidenceStatusLabel,
} from "@/lib/experience-contract";
import type { HeadOfFamilyResult, Role } from "@/lib/experience-contract";

// The Head of Family view renders only what Rust returned. Each conclusion opens
// its cited evidence in a metadata-only source dialog. There is no code path that sets a head
// from a model answer: a head label appears only when Rust recorded an
// appointment backed by evidence.

type Props = {
  role: Role | null;
  result: HeadOfFamilyResult;
  onApprove: (nominationId: string) => void;
  onRevoke: (appointmentId: string, reason: string) => void;
};

// Focus returns to the control that opened the dialog. The opener records itself
// here so a cancel or confirm can restore keyboard position.
const dialogOpener = { current: null as HTMLElement | null };

function rememberOpener() {
  if (typeof document === "undefined") return;
  dialogOpener.current = document.activeElement as HTMLElement | null;
}

function restoreOpener() {
  const opener = dialogOpener.current;
  dialogOpener.current = null;
  if (opener && typeof opener.focus === "function") opener.focus();
}

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
  const confirmRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    confirmRef.current?.focus();
  }, []);

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
      restoreOpener();
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

  return (
    <div
      ref={dialogRef}
      className="confirm hof-confirm-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="hof-confirm-title"
      onKeyDown={handleKeyDown}
      data-testid="confirm-dialog"
    >
      <h3 id="hof-confirm-title">{title}</h3>
      <p>{description}</p>
      <div className="hof-confirm-actions">
        <button
          ref={confirmRef}
          type="button"
          className="hof-confirm-action"
          onClick={() => {
            onConfirm();
            restoreOpener();
          }}
          data-testid="confirm-button"
        >
          {confirmLabel}
        </button>
        <button
          type="button"
          className="button-secondary hof-confirm-action"
          onClick={() => {
            onCancel();
            restoreOpener();
          }}
          data-testid="cancel-button"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

export function HeadOfFamilyView({ role, result, onApprove, onRevoke }: Props) {
  const [pendingApprove, setPendingApprove] = useState<string | null>(null);
  const [pendingRevoke, setPendingRevoke] = useState<{
    appointmentId: string;
    reason: string;
  } | null>(null);
  const [revokeReason, setRevokeReason] = useState("");
  const [openSource, setOpenSource] = useState<OpenSource | null>(null);

  useEffect(() => {
    setPendingApprove(null);
    setPendingRevoke(null);
  }, [result.kind]);

  if (result.kind === "denied") {
    return (
      <section
        className="state-panel hof-state hof-state-denied"
        role="alert"
        data-testid="hof-denied"
      >
        <h2>Access denied</h2>
        <p>{result.reason}</p>
        <p className="hint">
          Your role does not permit this request. Ask an administrator to review
          your access.
        </p>
        <div className="state-panel-actions">
          <a className="button-secondary" href="/genealogy">
            Back to the family tree
          </a>
        </div>
      </section>
    );
  }
  if (result.kind === "no_access") {
    return (
      <section
        className="state-panel hof-state hof-state-no-access"
        role="alert"
        data-testid="hof-no-access"
      >
        <h2>No access</h2>
        <p>{result.reason}</p>
        <p className="hint">
          No record is shown for this account. The archive never reveals whether
          a record exists outside your access.
        </p>
        <div className="state-panel-actions">
          <a className="button-secondary" href="/genealogy">
            Back to the family tree
          </a>
        </div>
      </section>
    );
  }
  if (result.kind === "expired") {
    return (
      <section
        className="state-panel state-panel-expired hof-state hof-state-expired"
        role="alert"
        data-testid="hof-expired"
      >
        <h2>Session expired</h2>
        <p>Sign in again to view the Head of Family record.</p>
        <div className="state-panel-actions">
          <a className="button-secondary" href={result.signInUrl || "/signin"}>
            Sign in again
          </a>
        </div>
      </section>
    );
  }

  const view = result.data;
  if (view.catalog === "unavailable") {
    return (
      <section
        className="state-panel state-panel-unavailable hof-state hof-state-catalog-unavailable"
        data-testid="hof-catalog-unavailable"
      >
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

  const conclusions = view.conclusions;
  const appointmentCount = conclusions.filter(
    (conclusion) =>
      Boolean(conclusion.appointment_id) || Boolean(conclusion.head_label),
  ).length;
  const citedHead = conclusions.find((conclusion) =>
    Boolean(conclusion.head_label),
  );
  const headEvidenceStatus = citedHead?.evidence_status;

  return (
    <section className="card hof-surface" data-testid="hof-view">
      <header className="hof-hero">
        <p className="hof-eyebrow">Genealogy designation</p>
        <h2 className="hof-hero-title">Head of Family</h2>
        {citedHead?.head_label ? (
          <p className="hof-hero-name" data-testid="hof-head-name">
            {citedHead.head_label}
          </p>
        ) : (
          <p className="hof-hero-name hof-hero-name-empty">
            No head is recorded with cited evidence.
          </p>
        )}
        <p className="hof-hero-scope">
          This designation is a genealogy fact backed by cited evidence. It is
          separate from application administrator permissions.
        </p>
        <div className="hof-hero-meta">
          <span className="hof-chip" data-testid="hof-appointment-count">
            {appointmentCount === 1
              ? "1 appointment"
              : `${appointmentCount} appointments`}
          </span>
          {headEvidenceStatus ? (
            <span className={`status-chip status-${headEvidenceStatus}`}>
              {evidenceStatusLabel(headEvidenceStatus)}
            </span>
          ) : null}
        </div>
      </header>

      {conclusions.length === 0 ? (
        <section
          className="state-panel state-panel-empty hof-empty"
          data-testid="hof-empty"
        >
          <h3>No active appointment</h3>
          <p>
            No family head is recorded with cited evidence. No successor is
            named or implied here; only a recorded appointment with evidence
            creates this designation.
          </p>
        </section>
      ) : (
        <ul className="conclusions hof-conclusions">
          {conclusions.map((conclusion, index) => (
            <li
              key={`${conclusion.conclusion}-${index}`}
              className="hof-conclusion"
              data-testid="hof-conclusion"
            >
              <p className="hof-conclusion-text">{conclusion.conclusion}</p>
              {conclusion.head_label ? (
                <p className="meta" data-testid="hof-cited-head">
                  Cited head: {conclusion.head_label}
                </p>
              ) : null}
              <dl className="hof-record-meta">
                {conclusion.record_id ? (
                  <div>
                    <dt>Record</dt>
                    <dd data-testid="hof-record-id">{conclusion.record_id}</dd>
                  </div>
                ) : null}
                {conclusion.appointment_id ? (
                  <div>
                    <dt>Appointment</dt>
                    <dd data-testid="hof-appointment-id">
                      {conclusion.appointment_id}
                    </dd>
                  </div>
                ) : null}
              </dl>

              <div className="hof-evidence-group">
                <div className="hof-evidence-head">
                  <h3>Evidence</h3>
                  <EvidenceBadge status={conclusion.evidence_status} />
                </div>
                <EvidenceList
                  evidence={conclusion.evidence}
                  onOpen={(evidence) =>
                    setOpenSource({
                      evidence,
                      citedFor: [
                        {
                          label: conclusion.head_label
                            ? `${conclusion.conclusion}: ${conclusion.head_label}`
                            : conclusion.conclusion,
                          status: conclusion.evidence_status,
                        },
                      ],
                    })
                  }
                />
              </div>

              {canNominate(role) ? (
                <a
                  href="#hof-record-select"
                  onClick={(event) => {
                    const input = document.getElementById("hof-record-select");
                    if (input instanceof HTMLInputElement) {
                      event.preventDefault();
                      input.focus();
                      input.scrollIntoView({ block: "center" });
                    }
                  }}
                >
                  Find a person to nominate
                </a>
              ) : null}

              {canApproveOrRevoke(role) ? (
                <div className="admin-controls hof-admin-controls">
                  <div className="hof-admin-approve">
                    <button
                      type="button"
                      onClick={() => {
                        rememberOpener();
                        setPendingApprove(
                          conclusion.nomination_id ?? conclusion.conclusion,
                        );
                      }}
                      data-testid="approve-button"
                    >
                      Approve nomination
                    </button>
                  </div>
                  <form
                    className="hof-form hof-form-danger"
                    onSubmit={(event) => {
                      event.preventDefault();
                      if (revokeReason.trim() === "") return;
                      rememberOpener();
                      setPendingRevoke({
                        appointmentId:
                          conclusion.appointment_id ?? conclusion.conclusion,
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
                    <button
                      type="submit"
                      className="button-danger"
                      data-testid="revoke-button"
                    >
                      Revoke appointment
                    </button>
                  </form>
                </div>
              ) : null}
            </li>
          ))}
        </ul>
      )}

      <SourceDialog source={openSource} onClose={() => setOpenSource(null)} />

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
