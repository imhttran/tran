"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import {
  ApiRequestError,
  getProfileInvitationState,
  getProfileInvitationStatus,
  prepareProfileInvitation,
  resendProfileInvitation,
  revokeProfileInvitation,
  saveProfileInvitationContact,
  sendProfileInvitation,
  type ProfileInvitationPrepareResponse,
  type ProfileInvitationStateResponse,
  type ProfileInvitationStatusResponse,
} from "@/lib/api";

// PIR-005: the invite action and status panel for one family profile.
//
// Presentation only. The Rust API decides whether this caller may invite the
// person: a 401 or 403 renders nothing at all, so the panel cannot advertise an
// action the server would refuse. Every value rendered here is already safe:
// the contact is masked by the server, the blockers carry a plain-language
// message, and a delivery failure is shown as prose rather than a raw code.
//
// Sending happens only from the explicit "Send invitation" action. Saving a
// draft, preparing a review, and opening this panel never deliver a message.

type PanelState =
  | { kind: "loading" }
  | { kind: "hidden" }
  | { kind: "unavailable" }
  | {
      kind: "ready";
      state: ProfileInvitationStateResponse;
      status: ProfileInvitationStatusResponse;
    };

const DELIVERY_LABELS: Record<string, string> = {
  not_sent: "Not sent",
  pending: "Invitation prepared — not sent",
  queued: "Queued — not yet confirmed",
  sent: "Sent",
  failed: "Delivery failed",
  expired: "Expired",
  accepted: "Accepted",
  revoked: "Revoked",
};

function deliveryLabel(value: string): string {
  return DELIVERY_LABELS[value] ?? "Status unavailable";
}

/// Maps the server's safe failure code to prose. An unknown code is never
/// rendered: it would leak an implementation detail and confuse the operator.
function deliveryErrorText(code: string | null): string | null {
  switch (code) {
    case null:
    case "":
      return null;
    case "not_configured":
      return "Email delivery is not configured, so nothing was sent.";
    case "provider_failed":
      return "The email provider did not accept the message.";
    case "delivery_failed":
      return "The message could not be delivered.";
    case "invalid_recipient":
      return "The saved contact address is not valid.";
    case "missing_verified_contact":
      return "No verified contact is recorded for this person.";
    case "not_prepared":
      return "The invitation was not ready to send.";
    default:
      return "The last delivery attempt failed.";
  }
}

function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError) {
    return error.body?.message ?? "The invitation action failed.";
  }
  return "The invitation action failed.";
}

function isAuthorizationFailure(error: unknown): boolean {
  return (
    error instanceof ApiRequestError &&
    (error.status === 401 || error.status === 403)
  );
}

export function ProfileInvitationPanel({
  personKey,
  personName,
}: {
  personKey: string;
  personName: string;
}) {
  const [panel, setPanel] = useState<PanelState>({ kind: "loading" });
  const [email, setEmail] = useState("");
  const [review, setReview] = useState<ProfileInvitationPrepareResponse | null>(
    null,
  );
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const sequence = useRef(0);

  const load = useCallback(async () => {
    const current = ++sequence.current;
    try {
      const [state, status] = await Promise.all([
        getProfileInvitationState(personKey),
        getProfileInvitationStatus(personKey),
      ]);
      if (sequence.current !== current) return;
      setPanel({ kind: "ready", state, status });
    } catch (err) {
      if (sequence.current !== current) return;
      setPanel({
        kind: isAuthorizationFailure(err) ? "hidden" : "unavailable",
      });
    }
  }, [personKey]);

  useEffect(() => {
    void load();
    return () => {
      sequence.current += 1;
    };
  }, [load]);

  const run = useCallback(
    async (action: () => Promise<string>) => {
      setBusy(true);
      setNotice(null);
      try {
        const message = await action();
        await load();
        setNotice(message);
      } catch (err) {
        setNotice(messageOf(err));
      } finally {
        setBusy(false);
      }
    },
    [load],
  );

  const saveDraft = () =>
    run(async () => {
      await saveProfileInvitationContact(personKey, email);
      setReview(null);
      return "Contact saved. Nothing has been sent.";
    });

  const startReview = () =>
    run(async () => {
      const prepared = await prepareProfileInvitation(personKey);
      setReview(prepared);
      return "Review the invitation before sending.";
    });

  const send = () =>
    run(async () => {
      const result = await sendProfileInvitation(personKey);
      setReview(null);
      return result.delivered
        ? "Invitation sent."
        : "The provider queued the invitation; delivery is not confirmed yet.";
    });

  const resend = () =>
    run(async () => {
      const result = await resendProfileInvitation(personKey);
      return result.delivered
        ? "A new invitation was sent. The previous link no longer works."
        : "The provider queued the new invitation; delivery is not confirmed yet.";
    });

  const revoke = () =>
    run(async () => {
      const result = await revokeProfileInvitation(personKey);
      setReview(null);
      return result.revoked
        ? "Invitation revoked. The link no longer works."
        : "There was no pending invitation to revoke.";
    });

  if (panel.kind === "loading" || panel.kind === "hidden") {
    // Nothing is rendered until the server has authorized this caller: an
    // unauthorized viewer must never see an invite affordance, not even briefly.
    return null;
  }

  if (panel.kind === "unavailable") {
    return (
      <section
        className="profile-section"
        data-testid="profile-invitation"
        aria-labelledby="profile-invitation-heading"
      >
        <div className="profile-section-heading">
          <h2 id="profile-invitation-heading">Invite to family archive</h2>
          <p>The invitation status is unavailable right now.</p>
        </div>
        <div className="review-actions">
          <button type="button" onClick={() => void load()} disabled={busy}>
            Try again
          </button>
        </div>
      </section>
    );
  }

  const { state, status } = panel;
  const maskedContact = state.masked_contact;
  const effectiveDelivery =
    status.delivery_state || state.invitation.delivery_state;
  const errorText = deliveryErrorText(status.last_delivery_error);

  return (
    <section
      className="profile-section"
      data-testid="profile-invitation"
      aria-labelledby="profile-invitation-heading"
    >
      <div className="profile-section-heading">
        <h2 id="profile-invitation-heading">Invite to family archive</h2>
        <p>
          Invite {personName} to create an account that can view the family
          archive. They decide whether to accept; this page never creates an
          account or sends anything by itself.
        </p>
      </div>

      {state.linked_account ? (
        <p className="meta" data-testid="profile-invitation-linked">
          Account linked. This person already has a family archive account, so
          there is nothing to invite.
        </p>
      ) : (
        <>
          <dl
            className="family-profile-counts"
            data-testid="profile-invitation-summary"
          >
            <div>
              <dt>Status</dt>
              <dd data-testid="profile-invitation-delivery">
                {deliveryLabel(effectiveDelivery)}
              </dd>
            </div>
            <div>
              <dt>Access</dt>
              <dd data-testid="profile-invitation-access">Member</dd>
            </div>
            <div>
              <dt>Contact</dt>
              <dd data-testid="profile-invitation-contact-state">
                {maskedContact ? "Contact saved" : "Needs contact"}
              </dd>
            </div>
          </dl>

          {maskedContact ? (
            <p className="meta" data-testid="profile-invitation-masked">
              Destination: {maskedContact}
            </p>
          ) : null}

          {state.proposed_username ? (
            <p className="meta" data-testid="profile-invitation-username">
              Proposed username: <code>{state.proposed_username}</code>
            </p>
          ) : null}

          {errorText ? (
            <p
              className="meta"
              role="status"
              data-testid="profile-invitation-delivery-error"
            >
              {errorText}
              {status.delivery_attempts > 0
                ? ` Attempts: ${status.delivery_attempts}.`
                : null}
            </p>
          ) : null}

          {state.blockers.length > 0 ? (
            <div data-testid="profile-invitation-blockers">
              <p className="meta">Before inviting this person:</p>
              <ul>
                {state.blockers.map((blocker) => (
                  <li key={blocker.code}>{blocker.message}</li>
                ))}
              </ul>
            </div>
          ) : null}

          <form
            className="profile-invitation-contact"
            onSubmit={(event) => {
              event.preventDefault();
              if (!busy) void saveDraft();
            }}
          >
            <label htmlFor="profile-invitation-email">
              {maskedContact ? "Update contact email" : "Contact email"}
            </label>
            <input
              id="profile-invitation-email"
              type="email"
              name="email"
              autoComplete="off"
              value={email}
              onChange={(event) => setEmail(event.target.value)}
              disabled={busy}
              data-testid="profile-invitation-contact"
            />
            <button type="submit" disabled={busy || email.trim() === ""}>
              {maskedContact ? "Update contact" : "Save contact"}
            </button>
          </form>

          <div className="review-actions">
            <button
              type="button"
              onClick={() => void startReview()}
              disabled={busy || !maskedContact}
              data-testid="profile-invitation-prepare"
            >
              Review invitation
            </button>
            {state.invitation.pending ? (
              <>
                <button
                  type="button"
                  onClick={() => void resend()}
                  disabled={busy}
                  data-testid="profile-invitation-resend"
                >
                  Resend invitation
                </button>
                <button
                  type="button"
                  className="button-danger"
                  onClick={() => void revoke()}
                  disabled={busy}
                  data-testid="profile-invitation-revoke"
                >
                  Revoke invitation
                </button>
              </>
            ) : null}
          </div>

          {review ? (
            <div
              className="profile-field-card profile-invitation-review"
              data-testid="profile-invitation-review"
              aria-live="polite"
            >
              <h3>Review before sending</h3>
              <ul>
                <li>Person: {personName}</li>
                <li>Destination: {review.masked_contact}</li>
                <li>Access: Member</li>
              </ul>
              <p className="meta">
                Sending emails a single-use activation link. The link expires
                automatically and can be revoked from this panel.
              </p>
              <div className="review-actions">
                <button
                  type="button"
                  onClick={() => void send()}
                  disabled={busy}
                  data-testid="profile-invitation-send"
                >
                  Send invitation
                </button>
                <button
                  type="button"
                  className="button-secondary"
                  onClick={() => setReview(null)}
                  disabled={busy}
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : null}
        </>
      )}

      {notice ? (
        <p
          className="hint"
          role="status"
          data-testid="profile-invitation-notice"
        >
          {notice}
        </p>
      ) : null}
    </section>
  );
}
