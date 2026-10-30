"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { AccountsAdmin } from "@/components/AccountsAdmin";
import { InvitationLinksAdmin } from "@/components/InvitationLinksAdmin";
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
  type BulkClassification,
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

// Focus returns to the control that opened a confirmation. The opener records
// itself so a cancel or confirm can restore keyboard position. Local to this
// module: the confirmation holds no library and no shared global.
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
  testId,
  title,
  children,
  confirmLabel,
  confirmTestId,
  busy,
  onConfirm,
  onCancel,
}: {
  testId: string;
  title: string;
  children: React.ReactNode;
  confirmLabel: string;
  confirmTestId: string;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  // Cancel takes focus on open so Escape or Enter never commits the action.
  useEffect(() => {
    cancelRef.current?.focus();
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
      className="confirm fm-bulk-confirm"
      role="dialog"
      aria-modal="true"
      aria-labelledby={`${testId}-title`}
      onKeyDown={handleKeyDown}
      data-testid={testId}
    >
      <h3 id={`${testId}-title`}>{title}</h3>
      {children}
      <div className="fm-bulk-confirm-actions">
        <button
          ref={cancelRef}
          type="button"
          className="button-secondary"
          disabled={busy}
          onClick={() => {
            onCancel();
            restoreOpener();
          }}
          data-testid={`${testId}-cancel`}
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onConfirm}
          data-testid={confirmTestId}
        >
          {confirmLabel}
        </button>
      </div>
    </div>
  );
}

// Rows requested per preview page. The API caps a page at 200.
const PAGE_LIMIT = 200;
// A safety cap on pages; 200 rows per page covers 20,000 people.
const MAX_PAGES = 100;

// The six classifications, in the order the stat cards show them.
const CLASSIFICATIONS: { key: BulkClassification; label: string }[] = [
  { key: "eligible_for_invite", label: "Eligible" },
  { key: "needs_review", label: "Needs review" },
  { key: "already_linked", label: "Already linked" },
  { key: "already_prepared", label: "Already prepared" },
  { key: "invited", label: "Invited" },
  { key: "not_eligible", label: "Not eligible" },
];

// The default view after a preview.
const DEFAULT_CLASSIFICATIONS: BulkClassification[] = [
  "eligible_for_invite",
  "needs_review",
];

// The total people a preview holds. `blocked_missing_recipient` is a flag on a
// row, not a classification, so it is not part of the sum.
function summaryTotal(summary: BulkPreview["summary"]): number {
  return (
    summary.eligible_for_invite +
    summary.needs_review +
    summary.already_linked +
    summary.already_prepared +
    summary.invited +
    summary.not_eligible
  );
}

// Case- and accent-insensitive key: decompose with NFD and drop combining
// marks, so "tran" matches "Trần".
function fold(value: string): string {
  return value.normalize("NFD").replace(/\p{M}/gu, "").toLocaleLowerCase();
}

// Reason codes the Contact column now carries, so the Warnings column hides
// them instead of repeating the same state.
const CONTACT_REASONS = new Set([
  "missing_verified_contact",
  "blocked_missing_recipient",
]);

// An ambiguously segmented name is where the proposed username most needs a
// second look.
function isAmbiguousName(row: BulkPreviewRow): boolean {
  return row.reasons.includes("ambiguous_segmentation");
}

function BulkOnboardContent({ role }: { role: Role | null }) {
  const [preview, setPreview] = useState<BulkPreview | null>(null);
  const [state, setState] = useState<LoadState>("idle");
  const [error, setError] = useState<string | null>(null);
  const [classifications, setClassifications] = useState<
    Set<BulkClassification>
  >(new Set(DEFAULT_CLASSIFICATIONS));
  const [noContact, setNoContact] = useState(false);
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [confirmPrepare, setConfirmPrepare] = useState(false);
  const [prepareResult, setPrepareResult] = useState<BulkPrepare | null>(null);
  const [batch, setBatch] = useState<BulkBatchStatus | null>(null);
  const [confirmSend, setConfirmSend] = useState(false);
  const [sendMessage, setSendMessage] = useState<string | null>(null);
  const [emails, setEmails] = useState<Record<string, string>>({});
  const [usernameEdits, setUsernameEdits] = useState<Record<string, string>>(
    {},
  );
  const [busy, setBusy] = useState(false);

  // Page the preview until every person is loaded. The summary counts the full
  // selected set before pagination, so their sum is the expected total. A short
  // page also ends the loop. A page cap prevents an endless loop.
  const load = useCallback(async () => {
    setState("loading");
    setError(null);
    try {
      const limit = PAGE_LIMIT;
      const rows: BulkPreviewRow[] = [];
      let merged: BulkPreview | null = null;
      let total = Number.POSITIVE_INFINITY;
      for (let page = 0; page < MAX_PAGES; page += 1) {
        const body = await bulkPreview({
          selector: "all",
          limit,
          offset: rows.length,
        });
        if (!merged) merged = body;
        total = summaryTotal(body.summary);
        rows.push(...body.rows);
        if (body.rows.length < limit) break;
        if (rows.length >= total) break;
      }
      if (!merged) return;
      setPreview({ ...merged, rows });
      setSelected(new Set());
      setUsernameEdits({});
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

  function openPrepareConfirm() {
    rememberOpener();
    setConfirmPrepare(true);
  }

  function openSendConfirm() {
    rememberOpener();
    setConfirmSend(true);
  }

  // Overrides for the selected rows whose edited username differs from the
  // proposal, keyed by person key. The server validates each one per row.
  function changedUsernames(): Record<string, string> {
    const changed: Record<string, string> = {};
    for (const row of preview?.rows ?? []) {
      const edit = usernameEdits[row.person_key]?.trim();
      if (
        selected.has(row.person_key) &&
        edit !== undefined &&
        edit !== (row.proposed_username ?? "")
      ) {
        changed[row.person_key] = edit;
      }
    }
    return changed;
  }
  const changedCount = Object.keys(changedUsernames()).length;

  async function prepare() {
    if (selected.size === 0) return;
    setBusy(true);
    setError(null);
    setSendMessage(null);
    try {
      const overrides = changedUsernames();
      const result = await bulkPrepare({
        person_keys: [...selected],
        confirmed_count: selected.size,
        ...(Object.keys(overrides).length > 0 ? { usernames: overrides } : {}),
      });
      setPrepareResult(result);
      setConfirmPrepare(false);
      restoreOpener();
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
      await bulkVerifyRecipient({
        person_key: personKey,
        email,
        confirmed: true,
      });
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
      const queued = result.results.filter(
        (row) => row.status === "queued",
      ).length;
      const failed = result.results.filter(
        (row) => row.status === "failed",
      ).length;
      setSendMessage(
        `Provider confirmed ${sent} sent, ${queued} queued, ${failed} failed. No account is activated until the recipient completes the identity flow and an admin reviews the person link.`,
      );
      setConfirmSend(false);
      restoreOpener();
      await refreshBatch(prepareResult.batch_id);
    } catch (err) {
      setError(errorMessage(err));
      setConfirmSend(false);
      restoreOpener();
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

  function toggleClassification(key: BulkClassification) {
    setClassifications((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  // Every filter is client-side over the loaded rows. An empty classification
  // selection means no classification filter, so all rows show.
  const foldedQuery = fold(query.trim());
  const allRows = preview?.rows ?? [];
  const visibleRows = allRows.filter((row) => {
    if (classifications.size > 0 && !classifications.has(row.classification)) {
      return false;
    }
    if (noContact && row.contact?.type !== "none") {
      return false;
    }
    if (foldedQuery) {
      const haystack = [
        row.display_name,
        row.person_key,
        row.proposed_username ?? "",
      ];
      if (!haystack.some((value) => fold(value).includes(foldedQuery))) {
        return false;
      }
    }
    return true;
  });

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

      <div className="fm-workspace">
        {error ? (
          <p
            className="state-panel state-panel-unavailable fm-state"
            role="alert"
            data-testid="bulk-error"
          >
            {error}
          </p>
        ) : null}

        <div className="bulk-actions fm-action-bar">
          <button
            type="button"
            onClick={() => void load()}
            disabled={busy || state === "loading"}
          >
            Preview eligible people
          </button>
          <button
            type="button"
            onClick={selectEligible}
            disabled={!preview || busy}
          >
            Select eligible
          </button>
          <button
            type="button"
            onClick={openPrepareConfirm}
            disabled={selected.size === 0 || busy}
            data-testid="bulk-prepare"
          >
            Prepare selected ({selected.size})
          </button>
        </div>

        {state === "loading" ? (
          <p className="state-panel state-panel-loading fm-state">
            Loading the private candidate set…
          </p>
        ) : null}

        {preview ? (
          <section
            className="status-panel fm-preview"
            data-testid="bulk-preview"
          >
            <h2>Preview</h2>
            <div
              className="fm-stat-grid"
              role="group"
              aria-label="Filter by classification"
            >
              {CLASSIFICATIONS.map((entry) => (
                <button
                  key={entry.key}
                  type="button"
                  aria-pressed={classifications.has(entry.key)}
                  onClick={() => toggleClassification(entry.key)}
                  data-testid={`bulk-filter-${entry.key}`}
                >
                  <span>{entry.label}</span>{" "}
                  <span>{(summary?.[entry.key] ?? 0).toString()}</span>
                </button>
              ))}
            </div>
            <div className="bulk-actions fm-action-bar">
              <button
                type="button"
                aria-pressed={noContact}
                onClick={() => setNoContact((current) => !current)}
                data-testid="bulk-filter-no-contact"
              >
                No contact
              </button>
              <input
                type="search"
                aria-label="Search people"
                placeholder="Search name, person key, or username"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                data-testid="bulk-filter-search"
              />
            </div>
            <p className="hint fm-dry-run" data-testid="bulk-showing">
              Showing {visibleRows.length} of {allRows.length} people
            </p>
            {preview.review_needed ? (
              <p className="insufficient" data-testid="bulk-review-needed">
                {preview.review_needed}
              </p>
            ) : null}
            <p className="hint fm-dry-run">
              Preview is a dry run. It changes nothing: no username is reserved,
              no invitation is created, and no email is sent.
            </p>
            <div className="fm-table-wrap">
              <table
                className="bulk-table fm-preview-table"
                data-testid="bulk-table"
              >
                <colgroup>
                  <col className="fm-col-select" />
                  <col className="fm-col-person" />
                  <col className="fm-col-contact" />
                  <col className="fm-col-status" />
                  <col className="fm-col-username" />
                  <col className="fm-col-warnings" />
                </colgroup>
                <thead>
                  <tr>
                    <th scope="col">Select</th>
                    <th scope="col">Person</th>
                    <th scope="col">Contact</th>
                    <th scope="col">Status</th>
                    <th scope="col">Proposed username</th>
                    <th scope="col">Warnings</th>
                  </tr>
                </thead>
                <tbody>
                  {visibleRows.map((row) => (
                    <PreviewRow
                      key={row.person_key}
                      row={row}
                      checked={selected.has(row.person_key)}
                      onToggle={() => toggle(row.person_key)}
                      username={
                        usernameEdits[row.person_key] ??
                        row.proposed_username ??
                        ""
                      }
                      onUsername={(value) =>
                        setUsernameEdits((current) => ({
                          ...current,
                          [row.person_key]: value,
                        }))
                      }
                    />
                  ))}
                </tbody>
              </table>
            </div>
          </section>
        ) : null}

        {confirmPrepare ? (
          <ConfirmDialog
            testId="bulk-prepare-confirm"
            title="Confirm preparation"
            confirmLabel="Confirm prepare"
            confirmTestId="bulk-prepare-confirm-button"
            busy={busy}
            onConfirm={() => void prepare()}
            onCancel={() => setConfirmPrepare(false)}
          >
            <p>
              Prepare <strong>{selected.size}</strong> selected people.
              Preparation reserves usernames and records a resumable batch.
            </p>
            <p data-testid="bulk-prepare-changed">
              {changedCount === 0
                ? "All usernames use the proposal."
                : `${changedCount} ${changedCount === 1 ? "username was" : "usernames were"} changed from the proposal.`}
            </p>
            <p className="insufficient">
              No accounts are activated and no emails are sent by this step.
            </p>
          </ConfirmDialog>
        ) : null}

        {prepareResult ? (
          <section
            className="status-panel fm-results"
            data-testid="bulk-results"
          >
            <h2>Batch {prepareResult.batch_id}</h2>
            <p className="hint">
              Prepared {prepareResult.prepared}, needs review{" "}
              {prepareResult.needs_review}, failed {prepareResult.failed}.
              Duplicate submission created{" "}
              {prepareResult.created ? "a new batch" : "no new batch"}.
            </p>
            {batch ? (
              <div className="fm-table-wrap">
                <table className="bulk-table" data-testid="bulk-status-table">
                  <thead>
                    <tr>
                      <th scope="col">Person</th>
                      <th scope="col">Preparation</th>
                      <th scope="col">Username</th>
                      <th scope="col">Delivery</th>
                      <th scope="col">Verified recipient</th>
                    </tr>
                  </thead>
                  <tbody>
                    {batch.items.map((item) => (
                      <tr key={item.person_key}>
                        <td data-label="Person" className="fm-cell-person">
                          <strong className="fm-person-name">
                            {item.display_name || item.person_key}
                          </strong>
                          <div className="hint fm-person-key">
                            <small>{item.person_key}</small>
                          </div>
                        </td>
                        <td data-label="Preparation">
                          {item.status}
                          {item.reason ? (
                            <span className="hint"> ({item.reason})</span>
                          ) : null}
                        </td>
                        <td data-label="Username">{item.username ?? "—"}</td>
                        <td data-label="Delivery">
                          {item.delivery_status}
                          {item.invitation_status ? (
                            <span className="hint">
                              {" "}
                              ({item.invitation_status})
                            </span>
                          ) : null}
                        </td>
                        <td data-label="Verified recipient">
                          {item.has_verified_contact ? (
                            <span className="badge">Verified</span>
                          ) : item.status === "prepared" ? (
                            <div className="bulk-email">
                              <input
                                type="email"
                                aria-label={`Verified email for ${item.display_name || item.person_key}`}
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
                                onClick={() =>
                                  void verifyEmail(item.person_key)
                                }
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
              </div>
            ) : null}

            <div className="bulk-actions fm-send-bar">
              <button
                type="button"
                onClick={openSendConfirm}
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
              <ConfirmDialog
                testId="bulk-send-confirm"
                title="Confirm send"
                confirmLabel="Confirm send"
                confirmTestId="bulk-send-confirm-button"
                busy={busy}
                onConfirm={() => void sendInvitations()}
                onCancel={() => setConfirmSend(false)}
              >
                <p>
                  Send exactly <strong>{sendableCount}</strong> invitations
                  through the configured adapter. Each recipient receives a
                  single-use, expiring activation link.
                </p>
                <p>
                  No account is activated until the recipient completes the
                  identity flow and an administrator reviews the link.
                </p>
              </ConfirmDialog>
            ) : null}

            {sendMessage ? (
              <p className="form-status ok" data-testid="bulk-send-result">
                {sendMessage}
              </p>
            ) : null}
          </section>
        ) : null}

        <InvitationLinksAdmin />

        <AccountsAdmin />
      </div>
    </>
  );
}

function warningsFor(row: BulkPreviewRow): string[] {
  return row.reasons.filter((code) => !CONTACT_REASONS.has(code));
}

function PreviewRow({
  row,
  checked,
  onToggle,
  username,
  onUsername,
}: {
  row: BulkPreviewRow;
  checked: boolean;
  onToggle: () => void;
  username: string;
  onUsername: (value: string) => void;
}) {
  const ambiguous = isAmbiguousName(row);
  return (
    <tr
      data-person-key={row.person_key}
      className={ambiguous ? "fm-row-ambiguous" : undefined}
    >
      <td data-label="Select">
        <input
          type="checkbox"
          checked={checked}
          onChange={onToggle}
          disabled={!row.preparable}
          aria-label={`Select ${row.person_key}`}
        />
      </td>
      <td data-label="Person" className="fm-cell-person">
        <strong className="fm-person-name">
          {row.display_name || row.person_key}
        </strong>
        <div className="hint fm-person-key">
          <small>{row.person_key}</small>
        </div>
      </td>
      <td data-label="Contact" className="fm-cell-contact">
        {row.contact?.type === "email" ? (
          <>
            <span>Email</span>{" "}
            <span className="hint">{row.contact.masked}</span>
          </>
        ) : (
          "None"
        )}
      </td>
      <td data-label="Status" className="fm-cell-status">
        <span className="badge">{row.classification}</span>
      </td>
      <td data-label="Proposed username" className="fm-cell-username">
        {row.preparable ? (
          <input
            type="text"
            className="fm-username-input"
            aria-label={`Username for ${row.display_name || row.person_key}`}
            aria-describedby={
              ambiguous ? `fm-ambiguous-${row.person_key}` : undefined
            }
            value={username}
            onChange={(event) => onUsername(event.target.value)}
            spellCheck={false}
            autoCapitalize="none"
          />
        ) : (
          (row.proposed_username ?? "—")
        )}
        {ambiguous ? (
          <div className="hint" id={`fm-ambiguous-${row.person_key}`}>
            <small>Check this username: the name splits ambiguously.</small>
          </div>
        ) : null}
      </td>
      <td data-label="Warnings" className="fm-cell-warnings">
        {warningsFor(row).length > 0 ? (
          <span className="fm-warnings">
            {warningsFor(row).map((code) => (
              <span key={code} className="badge hint fm-warning-chip">
                {code}
              </span>
            ))}
          </span>
        ) : (
          "—"
        )}
      </td>
    </tr>
  );
}
