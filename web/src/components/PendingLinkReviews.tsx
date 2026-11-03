"use client";

import { useCallback, useEffect, useState } from "react";

import {
  ApiRequestError,
  decideLinkRequest,
  listLinkRequests,
  type LinkRequestView,
} from "@/lib/api";

// PIR-008: the administrator's explicit review of accepted invitations.
//
// An invitation acceptance records a *request*; possessing the token never
// approves a person link. This panel is the administrative action that applies
// the existing reviewed-link policy. Approval delegates to the server's reviewed
// link path (which refuses to replace an existing link); rejection records the
// decision without a link.

function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError) {
    return error.body?.message ?? "The review action failed.";
  }
  return "The review action failed.";
}

export function PendingLinkReviews() {
  const [requests, setRequests] = useState<LinkRequestView[]>([]);
  const [busy, setBusy] = useState<number | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const result = await listLinkRequests();
      setRequests(result.requests);
    } catch {
      // An unauthorized caller sees nothing; this panel is an administrative
      // convenience, not an authorization decision.
      setRequests([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const decide = useCallback(
    async (requestId: number, decision: "approve" | "reject") => {
      setBusy(requestId);
      setNotice(null);
      try {
        const result = await decideLinkRequest(requestId, decision);
        setNotice(
          result.linked
            ? "Link approved."
            : result.already_decided
              ? "This request was already decided."
              : "Link rejected.",
        );
        await load();
      } catch (err) {
        setNotice(messageOf(err));
      } finally {
        setBusy(null);
      }
    },
    [load],
  );

  if (requests.length === 0 && !notice) {
    return null;
  }

  return (
    <section
      className="status-panel"
      data-testid="link-reviews"
      aria-labelledby="link-reviews-heading"
    >
      <h2 id="link-reviews-heading">Pending link reviews</h2>
      <p className="hint">
        Someone accepted an invitation and is waiting to be linked to the person
        below. Approving uses the reviewed-link policy; an existing link is
        never replaced.
      </p>
      {requests.length === 0 ? (
        <p className="hint" data-testid="link-review-empty">
          No pending link reviews.
        </p>
      ) : (
        <ul className="link-review-list">
          {requests.map((request) => (
            <li
              key={request.request_id}
              data-testid={`link-review-${request.request_id}`}
            >
              <div>
                <strong>{request.display_name}</strong>
                <div className="hint">
                  <small>{request.person_key}</small>
                </div>
              </div>
              <div className="review-actions">
                <button
                  type="button"
                  onClick={() => void decide(request.request_id, "approve")}
                  disabled={busy === request.request_id}
                  data-testid={`link-review-approve-${request.request_id}`}
                >
                  Approve link
                </button>
                <button
                  type="button"
                  className="button-danger"
                  onClick={() => void decide(request.request_id, "reject")}
                  disabled={busy === request.request_id}
                  data-testid={`link-review-reject-${request.request_id}`}
                >
                  Reject
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      {notice ? (
        <p className="hint" role="status" data-testid="link-review-notice">
          {notice}
        </p>
      ) : null}
    </section>
  );
}
