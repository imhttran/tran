"use client";

import { useEffect, useState } from "react";

import { canApproveOrRevoke, canNominate } from "@/lib/experience-contract";
import type { Role } from "@/lib/experience-contract";

type RecordView = { record_id: string; person_label: string };
type NominationView = {
  nomination_id: number;
  record_id: string;
  status: string;
};

type Pending =
  | { kind: "nominate"; recordId: string; label: string }
  | { kind: "approve"; nominationId: number }
  | { kind: "reject"; nominationId: number };

function csrfToken(): string | null {
  if (typeof document === "undefined") return null;
  const entry = document.cookie
    .split(";")
    .map((part) => part.trim())
    .find((part) => part.startsWith("rag_csrf="));
  return entry ? decodeURIComponent(entry.slice("rag_csrf=".length)) : null;
}

async function post(path: string, body: unknown): Promise<{ ok: boolean; message?: string }> {
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

// Real nomination/approval controls backed by the Rust API. Success is shown
// only by reloading the server state; a failed response is displayed instead of
// pretending the mutation happened.
export function HeadOfFamilyActions({ role }: { role: Role | null }) {
  const [records, setRecords] = useState<RecordView[]>([]);
  const [nominations, setNominations] = useState<NominationView[]>([]);
  const [selected, setSelected] = useState("");
  const [label, setLabel] = useState("");
  const [pending, setPending] = useState<Pending | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (!canNominate(role) && !canApproveOrRevoke(role)) {
      return;
    }
    let cancelled = false;
    const requests: Array<Promise<Response>> = [
      fetch("/api/genealogy/records", { cache: "no-store" }),
    ];
    if (canApproveOrRevoke(role)) {
      requests.push(fetch("/api/genealogy/nominations", { cache: "no-store" }));
    }
    Promise.all(requests)
      .then(async ([recordResponse, nominationResponse]) => {
        if (cancelled) return;
        if (recordResponse.ok) {
          const body = (await recordResponse.json()) as { records?: RecordView[] };
          setRecords(Array.isArray(body.records) ? body.records : []);
        }
        if (nominationResponse?.ok) {
          const body = (await nominationResponse.json()) as {
            nominations?: NominationView[];
          };
          setNominations(Array.isArray(body.nominations) ? body.nominations : []);
        }
      })
      .catch(() => {
        if (!cancelled) setError("The genealogy service is unavailable.");
      });
    return () => {
      cancelled = true;
    };
  }, [role]);

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

  const nominee = canNominate(role);
  const approver = canApproveOrRevoke(role);
  if (!nominee && !approver) return null;

  return (
    <section className="card" data-testid="hof-actions">
      <h2>Nomination and approval</h2>
      {error ? (
        <p className="insufficient" role="alert" data-testid="hof-actions-error">
          {error}
        </p>
      ) : null}

      {nominee ? (
        <form
          onSubmit={(event) => {
            event.preventDefault();
            if (!selected || label.trim() === "") return;
            setPending({ kind: "nominate", recordId: selected, label: label.trim() });
          }}
        >
          <label htmlFor="hof-record-select">Catalog record</label>
          <select
            id="hof-record-select"
            data-testid="hof-record-select"
            value={selected}
            onChange={(event) => setSelected(event.target.value)}
          >
            <option value="">Select a record</option>
            {records.map((record) => (
              <option key={record.record_id} value={record.record_id}>
                {record.person_label}
              </option>
            ))}
          </select>
          <label htmlFor="hof-nomination-label">Candidate label</label>
          <input
            id="hof-nomination-label"
            data-testid="hof-nomination-label"
            value={label}
            onChange={(event) => setLabel(event.target.value)}
          />
          <button type="submit" data-testid="hof-nominate-submit">
            Nominate
          </button>
        </form>
      ) : null}

      {approver ? (
        <div>
          <h3>Pending nominations</h3>
          {nominations.length === 0 ? (
            <p className="hint" data-testid="hof-no-nominations">
              No pending nominations.
            </p>
          ) : (
            <ul>
              {nominations.map((nomination) => (
                <li key={nomination.nomination_id} data-testid="hof-pending-nomination">
                  <span>{nomination.record_id}</span>
                  <button
                    type="button"
                    data-testid="hof-approve-submit"
                    onClick={() =>
                      setPending({ kind: "approve", nominationId: nomination.nomination_id })
                    }
                  >
                    Approve
                  </button>
                  <button
                    type="button"
                    data-testid="hof-reject-submit"
                    onClick={() =>
                      setPending({ kind: "reject", nominationId: nomination.nomination_id })
                    }
                  >
                    Reject
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      ) : null}

      {pending ? (
        <div className="confirm" role="dialog" aria-modal="true" data-testid="hof-confirm">
          <p>
            {pending.kind === "nominate"
              ? `Nominate ${pending.label}?`
              : pending.kind === "approve"
                ? "Approve this nomination?"
                : "Reject this nomination?"}
          </p>
          <button type="button" onClick={() => void confirm()} data-testid="hof-confirm-yes">
            Confirm
          </button>
          <button type="button" onClick={() => setPending(null)} data-testid="hof-confirm-cancel">
            Cancel
          </button>
        </div>
      ) : null}
    </section>
  );
}
