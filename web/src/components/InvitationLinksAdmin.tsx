"use client";

import { useCallback, useEffect, useState } from "react";

import { FormStatus } from "@/components/FormStatus";
import type { FormStatusState } from "@/components/FormStatus";
import { NominationPersonSelector } from "@/components/NominationPersonSelector";
import type { NominationPerson } from "@/components/NominationPersonSelector";
import {
  ApiRequestError,
  createInvitationLink,
  decideInvitationClaim,
  listInvitationClaims,
} from "@/lib/api";
import type { InvitationClaim } from "@/lib/api";

// Shareable invitation links. An administrator creates a single-use link for
// one family member and sends it however suits them (text, chat, in person);
// no email address is needed. When the member redeems it, their login is
// recorded as a claim that an administrator confirms or rejects here. Rust
// enforces every step; this view only asks and shows.

type AdminPerson = NominationPerson & { linked?: boolean };
type Shared = { person: string; url: string; days: number };

function messageOf(error: unknown, fallback: string): string {
  if (error instanceof ApiRequestError) {
    if (error.body?.error === "already_linked") return "That person is already linked to a login.";
    return error.body?.message ?? fallback;
  }
  return fallback;
}

// The configured activation URL may be relative (the default is /activate).
function absolute(url: string): string {
  return new URL(url, window.location.origin).toString();
}

export function InvitationLinksAdmin() {
  const [chosen, setChosen] = useState<NominationPerson | null>(null);
  const [shared, setShared] = useState<Shared | null>(null);
  const [copied, setCopied] = useState(false);
  const [claims, setClaims] = useState<InvitationClaim[]>([]);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<FormStatusState>(null);

  const loadClaims = useCallback(async () => {
    try {
      setClaims(await listInvitationClaims());
    } catch {
      setClaims([]);
      setStatus({ kind: "error", text: "Claims awaiting confirmation could not be loaded." });
    }
  }, []);

  useEffect(() => {
    void loadClaims();
  }, [loadClaims]);

  async function create() {
    if (!chosen || busy) return;
    if (!window.confirm(`Create a new invitation link for ${chosen.person_label}? Any earlier link for them stops working.`)) return;
    setBusy(true);
    setStatus(null);
    setCopied(false);
    try {
      const link = await createInvitationLink(chosen.record_id);
      setShared({
        person: chosen.person_label,
        url: absolute(link.url),
        days: Math.round(link.expires_in_secs / 86400),
      });
    } catch (error) {
      setShared(null);
      setStatus({ kind: "error", text: messageOf(error, "The invitation link could not be created.") });
    } finally {
      setBusy(false);
    }
  }

  async function copy() {
    if (!shared) return;
    try {
      await navigator.clipboard.writeText(shared.url);
      setCopied(true);
    } catch {
      setStatus({ kind: "info", text: "Copy is unavailable here; select the link and copy it manually." });
    }
  }

  async function decide(claim: InvitationClaim, decision: "approve" | "reject") {
    const question =
      decision === "approve"
        ? `Link this login to ${claim.person_label}? They will be able to view the family archive.`
        : `Reject this claim for ${claim.person_label}? No link is made; you can send a new invitation.`;
    if (busy || !window.confirm(question)) return;
    setBusy(true);
    setStatus(null);
    try {
      await decideInvitationClaim(claim.invitation_id, decision);
      setStatus({
        kind: "ok",
        text: decision === "approve" ? `Linked to ${claim.person_label}.` : "Claim rejected. No link was made.",
      });
    } catch (error) {
      setStatus({ kind: "error", text: messageOf(error, "The decision failed. Nothing was changed.") });
    } finally {
      setBusy(false);
      await loadClaims();
    }
  }

  return (
    <section className="status-panel fm-accounts" data-testid="invitation-links">
      <div className="fm-section-head">
        <h2>Invitation links</h2>
        <p className="hint">
          Create a single-use link for a family member and send it by text,
          chat, or in person. No email address is needed. Each link expires,
          works once, and replaces any earlier link for that person.
        </p>
      </div>

      <form
        className="fm-link-form"
        onSubmit={(event) => {
          event.preventDefault();
          void create();
        }}
      >
        <NominationPersonSelector
          endpoint="/api/admin/family-members/people"
          onSelect={(person) => {
            setChosen(person);
            setShared(null);
          }}
          isDisabled={(person) => (person as AdminPerson).linked === true}
          disabledText="Already linked"
          idPrefix="invite-link"
        />
        <button type="submit" disabled={!chosen || busy} data-testid="invite-link-create">
          Create invitation link
        </button>
      </form>

      {shared ? (
        <div className="invite-link-result" data-testid="invite-link-result">
          <p>
            Invitation link for <strong>{shared.person}</strong>, valid for {shared.days} day
            {shared.days === 1 ? "" : "s"}. It is shown only now; share it privately.
          </p>
          <input
            type="text"
            readOnly
            value={shared.url}
            aria-label={`Invitation link for ${shared.person}`}
            onFocus={(event) => event.currentTarget.select()}
            data-testid="invite-link-url"
          />
          <button type="button" onClick={() => void copy()} data-testid="invite-link-copy">
            {copied ? "Copied" : "Copy link"}
          </button>
        </div>
      ) : null}

      <FormStatus status={status} />

      <h3>Claims awaiting confirmation</h3>
      {claims.length === 0 ? (
        <p className="hint" data-testid="invite-claims-empty">No redeemed invitations are waiting.</p>
      ) : (
        <ul className="sources" data-testid="invite-claims">
          {claims.map((claim) => (
            <li key={claim.invitation_id} data-testid="invite-claim">
              <p>
                <strong>{claim.claimed_name}</strong>{" "}
                <span className="meta">(name they entered, not verified)</span> redeemed the
                invitation for <strong>{claim.person_label}</strong>.
              </p>
              <p className="meta">Login #{claim.account_id} · {claim.claimed_at}</p>
              <div className="profile-actions">
                <button type="button" disabled={busy} onClick={() => void decide(claim, "approve")} data-testid="invite-claim-approve">
                  Confirm and link
                </button>
                <button type="button" className="button-danger" disabled={busy} onClick={() => void decide(claim, "reject")} data-testid="invite-claim-reject">
                  Reject
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
