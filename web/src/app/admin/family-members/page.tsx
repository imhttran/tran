"use client";

import { useCallback, useEffect, useState } from "react";

import { AccountsAdmin } from "@/components/AccountsAdmin";
import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  bulkBatchStatus,
  bulkPrepare,
  bulkPreview,
  bulkSend,
  bulkVerifyRecipient,
  type BulkBatchStatus,
  type BulkPrepare,
  type BulkPreview,
  type BulkPreviewRow,
} from "@/lib/api";
import { canAdmin, type Role } from "@/lib/experience-contract";

// Admin-only bulk onboarding. Preview and dry-run are read-only. Preparation
// reserves usernames and records a batch, but never activates an account.
// Sending is a separate, explicitly confirmed action and stays blocked until a
// delivery adapter and a verified recipient exist.
export default function BulkOnboardPage() {
  return (
    <ExperienceShell>
      {(role) => <BulkOnboardContent role={role} />}
    </ExperienceShell>
  );
}

type LoadState = "idle" | "loading" | "ready" | "error";

function errorMessage(error: unknown): string {
  if (error instanceof ApiRequestError) {
    if (error.body?.error === "not_configured") {
      return "No invitation delivery adapter is configured. A production operator must configure and approve one before real invitations can be sent.";
    }
    return error.message;
  }
  return error instanceof Error ? error.message : "The request failed.";
}

function BulkOnboardContent({ role }: { role: Role | null }) {
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [state, setState] = useState<LoadState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmPrepare, setConfirmPrepare] = useState(false);
  const [prepareResult, setPrepareResult] = useState<BulkPrepare | null>(null);
  const [batch, setBatch] = useState<BulkBatchStatus | null>(null);
  const [confirmSend, setConfirmSend] = useState(false);
  const [sendMessage, setSendMessage] = useState<string | null>(null);
  const [emails, setEmails] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setState("loading");
    setError(null);
    try {
      const body = await bulkPreview({ selector: "all", limit: 200 });
      setPreview(body);
      setSelected(new Set());
      setConfirmPrepare(false);
      setState("ready");
    } catch (err) {
      setError(errorMessage(err));
      setState("error");
    }
  }, []);

  useEffect(() => {
    if (canAdmin(role) && state === "idle") {
      void load();
    }
  }, [role, state, load]);

  async function refreshBatch(batchId: number) {
    try {
      setBatch(await bulkBatchStatus(batchId));
    } catch (err) {
      setError(errorMessage(err));
    }
  }

  function toggle(key: string) {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  function selectEligible() {
    if (!preview) return;
    setSelected(
      new Set(
        preview.rows
          .filter((row) => row.preparable)
          .map((row) => row.person_key),
      ),
    );
  }

  async function prepare() {
    if (selected.size === 0) return;
    setBusy(true);
    setError(null);
    setSendMessage(null);
    try {
      const result = await bulkPrepare({
        person_keys: [...selected],
        confirmed_count: selected.size,
      });
      setPrepareResult(result);
      setConfirmPrepare(false);
      await refreshBatch(result.batch_id);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function verifyEmail(personKey: string) {
    const email = (emails[personKey] ?? "").trim();
    if (!email) return;
    setBusy(true);
    setError(null);
    try {
      await bulkVerifyRecipient({ person_key: personKey, email, confirmed: true });
      if (prepareResult) await refreshBatch(prepareResult.batch_id);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function sendInvitations() {
    if (!prepareResult) return;
    setBusy(true);
    setError(null);
    setSendMessage(null);
    try {
      const preparedKeys =
        batch?.items
          .filter(
            (item) =>
              item.status === "prepared" &&
              item.has_verified_contact &&
              item.delivery_status !== "sent" &&
              item.delivery_status !== "queued" &&
              item.delivery_status !== "accepted",
          )
          .map((item) => item.person_key) ?? [];
      const result = await bulkSend({
        batch_id: prepareResult.batch_id,
        person_keys: preparedKeys,
        approve: true,
      });
      const sent = result.results.filter((row) => row.status === "sent").length;
      const queued = result.results.filter((row) => row.status === "queued").length;
      const failed = result.results.filter((row) => row.status === "failed").length;
      setSendMessage(
        `Provider confirmed ${sent} sent, ${queued} queued, ${failed} failed. No account is activated until the recipient completes the identity flow and an admin reviews the person link.`,
      );
      setConfirmSend(false);
      await refreshBatch(prepareResult.batch_id);
    } catch (err) {
      setError(errorMessage(err));
      setConfirmSend(false);
    } finally {
      setBusy(false);
    }
  }

  if (!canAdmin(role)) {
    return (
      <DenialView
        kind="denied"
        reason="Bulk onboarding is restricted to application administrators."
      />
    );
  }

  const summary = preview?.summary;
  const preparedItems =
    batch?.items.filter((item) => item.status === "prepared") ?? [];
  const sendableCount = preparedItems.filter(
    (item) =>
      item.has_verified_contact &&
      item.delivery_status !== "sent" &&
      item.delivery_status !== "queued" &&
      item.delivery_status !== "accepted",
  ).length;

  return (
    <>
      <PageTitle title="Bulk onboard family members" />
      <PageHeader
        title="Bulk onboard"
        subtitle="Preview verified living family members, prepare accounts, and send invitations only after a separate approval. Preparation never activates an account."
      />

      {error ? (
        <p className="insufficient" role="alert" data-testid="bulk-error">
          {error}
        </p>
      ) : null}

      <div className="bulk-actions">
        <button type="button" onClick={() => void load()} disabled={busy || state === "loading"}>
          Preview eligible people
        </button>
        <button type="button" onClick={selectEligible} disabled={!preview || busy}>
          Select eligible
        </button>
        <button
          type="button"
          onClick={() => setConfirmPrepare(true)}
          disabled={selected.size === 0 || busy}
          data-testid="bulk-prepare"
        >
          Prepare selected ({selected.size})
        </button>
      </div>

      {state === "loading" ? <p className="hint">Loading the private candidate set…</p> : null}

      {preview ? (
        <section className="status-panel" data-testid="bulk-preview">
          <h2>Preview</h2>
          <dl>
            <div><dt>Eligible</dt><dd>{summary?.eligible_for_invite ?? 0}</dd></div>
            <div><dt>Needs review</dt><dd>{summary?.needs_review ?? 0}</dd></div>
            <div><dt>Already linked</dt><dd>{summary?.already_linked ?? 0}</dd></div>
            <div><dt>Already prepared</dt><dd>{summary?.already_prepared ?? 0}</dd></div>
            <div><dt>Invited</dt><dd>{summary?.invited ?? 0}</dd></div>
            <div><dt>Not eligible</dt><dd>{summary?.not_eligible ?? 0}</dd></div>
          </dl>
          {preview.review_needed ? (
            <p className="hint" data-testid="bulk-review-needed">{preview.review_needed}</p>
          ) : null}
          <p className="hint">
            Preview is a dry run. It changes nothing: no username is reserved, no
            invitation is created, and no email is sent.
          </p>
          <table className="bulk-table" data-testid="bulk-table">
            <thead>
              <tr>
                <th scope="col">Select</th>
                <th scope="col">Person key</th>
                <th scope="col">Status</th>
                <th scope="col">Proposed username</th>
                <th scope="col">Warnings</th>
              </tr>
            </thead>
            <tbody>
              {preview.rows.map((row) => (
                <PreviewRow
                  key={row.person_key}
                  row={row}
                  checked={selected.has(row.person_key)}
                  onToggle={() => toggle(row.person_key)}
                />
              ))}
            </tbody>
          </table>
        </section>
      ) : null}

      {confirmPrepare ? (
        <section className="status-panel" data-testid="bulk-prepare-confirm">
          <h2>Confirm preparation</h2>
          <p>
            Prepare <strong>{selected.size}</strong> selected people. Preparation
            reserves usernames and records a resumable batch.
          </p>
          <p className="insufficient">
            No accounts are activated and no emails are sent by this step.
          </p>
          <div className="bulk-actions">
            <button type="button" onClick={() => void prepare()} disabled={busy} data-testid="bulk-prepare-confirm-button">
              Confirm prepare
            </button>
            <button type="button" onClick={() => setConfirmPrepare(false)} disabled={busy}>
              Cancel
            </button>
          </div>
        </section>
      ) : null}

      {prepareResult ? (
        <section className="status-panel" data-testid="bulk-results">
          <h2>Batch {prepareResult.batch_id}</h2>
          <p className="hint">
            Prepared {prepareResult.prepared}, needs review {prepareResult.needs_review},
            failed {prepareResult.failed}. Duplicate submission created{" "}
            {prepareResult.created ? "a new batch" : "no new batch"}.
          </p>
          {batch ? (
            <table className="bulk-table" data-testid="bulk-status-table">
              <thead>
                <tr>
                  <th scope="col">Person key</th>
                  <th scope="col">Preparation</th>
                  <th scope="col">Username</th>
                  <th scope="col">Delivery</th>
                  <th scope="col">Verified recipient</th>
                </tr>
              </thead>
              <tbody>
                {batch.items.map((item) => (
                  <tr key={item.person_key}>
                    <td>{item.person_key}</td>
                    <td>
                      {item.status}
                      {item.reason ? <span className="hint"> ({item.reason})</span> : null}
                    </td>
                    <td>{item.username ?? "—"}</td>
                    <td>
                      {item.delivery_status}
                      {item.invitation_status ? (
                        <span className="hint"> ({item.invitation_status})</span>
                      ) : null}
                    </td>
                    <td>
                      {item.has_verified_contact ? (
                        <span className="badge">Verified</span>
                      ) : item.status === "prepared" ? (
                        <div className="bulk-email">
                          <input
                            type="email"
                            aria-label={`Verified email for ${item.person_key}`}
                            placeholder="verified@example.test"
                            value={emails[item.person_key] ?? ""}
                            onChange={(event) =>
                              setEmails((current) => ({
                                ...current,
                                [item.person_key]: event.target.value,
                              }))
                            }
                          />
                          <button
                            type="button"
                            onClick={() => void verifyEmail(item.person_key)}
                            disabled={busy}
                          >
                            Verify
                          </button>
                        </div>
                      ) : (
                        "—"
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : null}

          <div className="bulk-actions">
            <button
              type="button"
              onClick={() => setConfirmSend(true)}
              disabled={busy || sendableCount === 0}
              data-testid="bulk-send"
            >
              Send invitations ({sendableCount})
            </button>
            {sendableCount === 0 ? (
              <span className="hint">
                Sending stays disabled until a prepared person has a verified
                recipient email.
              </span>
            ) : null}
          </div>

          {confirmSend ? (
            <div className="insufficient" data-testid="bulk-send-confirm">
              <p>
                Send exactly <strong>{sendableCount}</strong> invitations through
                the configured adapter. Each recipient receives a single-use,
                expiring activation link.
              </p>
              <p>No account is activated until the recipient completes the identity flow and an administrator reviews the link.</p>
              <div className="bulk-actions">
                <button type="button" onClick={() => void sendInvitations()} disabled={busy} data-testid="bulk-send-confirm-button">
                  Confirm send
                </button>
                <button type="button" onClick={() => setConfirmSend(false)} disabled={busy}>
                  Cancel
                </button>
              </div>
            </div>
          ) : null}

          {sendMessage ? (
            <p className="form-status ok" data-testid="bulk-send-result">{sendMessage}</p>
          ) : null}
        </section>
      ) : null}

      <AccountsAdmin />
    </>
  );
}

function PreviewRow({
  row,
  checked,
  onToggle,
}: {
  row: BulkPreviewRow;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <tr>
      <td>
        <input
          type="checkbox"
          checked={checked}
          onChange={onToggle}
          disabled={!row.preparable}
          aria-label={`Select ${row.person_key}`}
        />
      </td>
      <td>{row.person_key}</td>
      <td>
        <span className="badge">{row.classification}</span>
        {row.blocked_missing_recipient ? (
          <span className="hint"> recipient missing</span>
        ) : null}
      </td>
      <td>{row.proposed_username ?? "—"}</td>
      <td className="hint">{row.reasons.join(", ") || "—"}</td>
    </tr>
  );
}
