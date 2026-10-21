"use client";

import { useCallback, useEffect, useState } from "react";

import { FormStatus } from "@/components/FormStatus";
import type { FormStatusState } from "@/components/FormStatus";
import {
  ApiRequestError,
  changeMemberRole,
  deactivateMember,
  linkMember,
  listFamilyMembers,
  requestPasswordReset,
  revokeMemberLink,
} from "@/lib/api";
import type { FamilyMember, FamilyMembersPage, MemberFilter } from "@/lib/api";
import type { Role } from "@/lib/experience-contract";

// Accounts section of Admin > Family Members. Every list and every outcome
// comes from the Rust API; nothing here decides authorization, and no success
// is shown unless the API confirmed it. Invitations live in the bulk onboarding
// section above. Branch reviewer grants are not offered: there is no grant API.
type Linkage = FamilyMembersPage["linkage"];

type Pending =
  | { kind: "reset"; member: FamilyMember }
  | { kind: "deactivate"; member: FamilyMember }
  | { kind: "role"; member: FamilyMember; role: Role; revoke: boolean }
  | { kind: "link"; member: FamilyMember; personKey: string }
  | { kind: "unlink"; member: FamilyMember; linkId: number };

const ROLES: Role[] = ["viewer", "editor", "admin"];
const EMPTY_FILTER: MemberFilter = { q: "", status: "", role: "" };

// Safe, human messages for the API's stable error codes.
const ERROR_MESSAGES: Record<string, string> = {
  reset_not_configured:
    "Password reset is not configured for this identity provider. No email was sent. The member can use the provider's own \"Forgot password?\" link.",
  reset_failed: "The identity provider did not accept the reset request. No email was sent. Try again later.",
  reset_throttled: "A reset was already requested for this account recently. Wait before trying again.",
  account_deactivated: "This account is deactivated.",
  last_admin: "Refusing to remove the last active administrator.",
  csrf_failed: "Your session's security token is missing or stale. Reload the page and try again.",
  unknown_person: "No genealogy person has that key.",
  identity_not_registered: "That account is not registered with this issuer.",
  identity_deactivated: "That account is deactivated and cannot be linked.",
  identity_ambiguous: "That subject exists under more than one issuer; it cannot be linked.",
  already_linked: "That person or account already has an active link.",
};

function messageFor(error: unknown, fallback: string): string {
  if (error instanceof ApiRequestError) {
    const code = error.body?.error;
    return (code && ERROR_MESSAGES[code]) || error.message || fallback;
  }
  return error instanceof Error ? error.message : fallback;
}

function label(member: FamilyMember): string {
  return member.display_name || member.email || member.subject;
}

function confirmText(pending: Pending): string {
  const who = label(pending.member);
  switch (pending.kind) {
    case "reset":
      return `Ask the identity provider to email a password reset link to ${who}? You will not see the link or the password.`;
    case "deactivate":
      return `Deactivate ${who}? They lose access immediately; their identity and audit history are kept.`;
    case "role":
      return pending.revoke ? `Remove the ${pending.role} role from ${who}?` : `Grant the ${pending.role} role to ${who}?`;
    case "link":
      return `Confirm you reviewed the identity proof and link ${who} to genealogy person ${pending.personKey}?`;
    case "unlink":
      return `Remove the link between ${who} and their genealogy person?`;
  }
}

// Runs a confirmed action and returns the success message.
async function run(action: Pending): Promise<string> {
  const who = label(action.member);
  switch (action.kind) {
    case "reset":
      await requestPasswordReset(action.member);
      return `The identity provider accepted the reset request for ${who}. The provider sends the email; delivery is not confirmed here.`;
    case "deactivate":
      await deactivateMember(action.member);
      return `${who} was deactivated.`;
    case "role":
      await changeMemberRole(action.member, action.role, action.revoke);
      return action.revoke ? `Removed ${action.role} from ${who}.` : `Granted ${action.role} to ${who}.`;
    case "link": {
      const result = await linkMember(action.member, action.personKey);
      if (!result.linked) throw new Error("The link was not created.");
      return `${who} is linked to ${action.personKey}.`;
    }
    case "unlink":
      await revokeMemberLink(action.linkId);
      return `The link for ${who} was removed.`;
  }
}

export function AccountsAdmin() {
  // `draft` is what the controls show; `applied` is the query the loaded rows
  // and `nextAfter` cursor belong to, so "Load more" never mixes queries.
  const [draft, setDraft] = useState<MemberFilter>(EMPTY_FILTER);
  const [applied, setApplied] = useState<MemberFilter>(EMPTY_FILTER);
  const [members, setMembers] = useState<FamilyMember[]>([]);
  const [nextAfter, setNextAfter] = useState<number | null>(null);
  const [linkage, setLinkage] = useState<Linkage>("available");
  const [loadError, setLoadError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState<Pending | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<FormStatusState>(null);

  const load = useCallback(async (active: MemberFilter, after: number | null) => {
    setLoading(true);
    setLoadError(null);
    try {
      const page = await listFamilyMembers(active, after);
      setApplied(active);
      setMembers((current) => (after ? [...current, ...page.members] : page.members));
      setNextAfter(page.next_after);
      setLinkage(page.linkage);
    } catch (error) {
      setLoadError(messageFor(error, "The member directory could not be loaded."));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(EMPTY_FILTER, null);
  }, [load]);

  async function confirm() {
    if (!pending || busy) return;
    setBusy(true);
    const action = pending;
    try {
      setStatus({ kind: "ok", text: await run(action) });
      if (action.kind !== "reset") await load(applied, null);
    } catch (error) {
      setStatus({ kind: "error", text: messageFor(error, "The request failed. Nothing was changed.") });
    } finally {
      setBusy(false);
      setPending(null);
    }
  }

  function ask(action: Pending) {
    setStatus(null);
    setPending(action);
  }

  return (
    <section className="status-panel" data-testid="accounts">
      <h2>Accounts</h2>
      <form
        className="family-toolbar"
        role="search"
        data-testid="fm-filters"
        onSubmit={(event) => {
          event.preventDefault();
          void load(draft, null);
        }}
      >
        <label>
          Search
          <input
            type="search"
            placeholder="Name, email, or subject"
            value={draft.q}
            onChange={(event) => setDraft({ ...draft, q: event.target.value })}
            data-testid="fm-search"
          />
        </label>
        <label>
          Status
          <select
            value={draft.status}
            onChange={(event) => setDraft({ ...draft, status: event.target.value as MemberFilter["status"] })}
            data-testid="fm-status-filter"
          >
            <option value="">Any</option>
            <option value="active">Active</option>
            <option value="deactivated">Deactivated</option>
          </select>
        </label>
        <label>
          Role
          <select
            value={draft.role}
            onChange={(event) => setDraft({ ...draft, role: event.target.value as MemberFilter["role"] })}
            data-testid="fm-role-filter"
          >
            <option value="">Any</option>
            {ROLES.map((role) => (
              <option key={role} value={role}>{role}</option>
            ))}
          </select>
        </label>
        <button type="submit" data-testid="fm-apply-filters">Apply</button>
      </form>

      <FormStatus status={status} />
      {loadError ? (
        <p className="insufficient" role="alert" data-testid="fm-load-error">{loadError}</p>
      ) : null}
      {linkage === "unavailable" ? (
        <p className="hint" data-testid="fm-linkage-unavailable">
          The genealogy store is unavailable, so linked-person status is not shown.
        </p>
      ) : null}

      <ul className="sources" data-testid="fm-members">
        {members.map((member) => (
          <MemberRow key={member.user_id} member={member} linkage={linkage} onAction={ask} />
        ))}
      </ul>
      {!loading && members.length === 0 && !loadError ? (
        <p className="hint" data-testid="fm-empty">No accounts match these filters.</p>
      ) : null}
      {loading ? <p className="hint">Loading accounts…</p> : null}
      {nextAfter && !loading ? (
        <button type="button" onClick={() => void load(applied, nextAfter)} data-testid="fm-more">
          Load more
        </button>
      ) : null}

      {pending ? (
        <div className="confirm" role="dialog" aria-modal="true" data-testid="fm-confirm">
          <p>{confirmText(pending)}</p>
          <button type="button" disabled={busy} onClick={() => void confirm()} data-testid="fm-confirm-yes">
            {busy ? "Working…" : "Confirm"}
          </button>
          <button type="button" disabled={busy} onClick={() => setPending(null)} data-testid="fm-confirm-cancel">
            Cancel
          </button>
        </div>
      ) : null}
    </section>
  );
}

function MemberRow({
  member,
  linkage,
  onAction,
}: {
  member: FamilyMember;
  linkage: Linkage;
  onAction: (action: Pending) => void;
}) {
  const [role, setRole] = useState<Role>("viewer");
  const [personKey, setPersonKey] = useState("");
  const active = member.status === "active";
  const linked = member.linked_person;
  return (
    <li data-testid="fm-member" data-subject={member.subject}>
      <strong>{label(member)}</strong>{" "}
      <span className="badge" data-testid="fm-member-status">{member.status}</span>{" "}
      {member.roles.map((held) => (
        <span key={held} className="badge" data-testid="fm-member-role">{held}</span>
      ))}
      <p className="meta">
        {member.email ?? "No email on file"} · {member.subject} @ {member.issuer}
      </p>
      <p className="meta" data-testid="fm-member-link">
        {linked
          ? `Linked to ${linked.person_key}${linked.username ? ` (${linked.username})` : ""}`
          : linkage === "available"
            ? "Not linked to a genealogy person"
            : "Link status unavailable"}
      </p>
      <details data-testid="fm-actions">
        <summary>Actions</summary>
        <button
          type="button"
          disabled={!active}
          onClick={() => onAction({ kind: "reset", member })}
          data-testid="fm-reset"
        >
          Send password reset email
        </button>
        <div>
          <select
            aria-label="Role"
            value={role}
            onChange={(event) => setRole(event.target.value as Role)}
            data-testid="fm-role-select"
          >
            {ROLES.map((option) => (
              <option key={option} value={option}>{option}</option>
            ))}
          </select>
          <button
            type="button"
            disabled={!active || member.roles.includes(role)}
            onClick={() => onAction({ kind: "role", member, role, revoke: false })}
            data-testid="fm-grant-role"
          >
            Grant role
          </button>
          <button
            type="button"
            disabled={!active || !member.roles.includes(role)}
            onClick={() => onAction({ kind: "role", member, role, revoke: true })}
            data-testid="fm-revoke-role"
          >
            Remove role
          </button>
        </div>
        {linked ? (
          <button
            type="button"
            onClick={() => onAction({ kind: "unlink", member, linkId: linked.link_id })}
            data-testid="fm-unlink"
          >
            Remove person link
          </button>
        ) : linkage === "available" && active ? (
          <form
            onSubmit={(event) => {
              event.preventDefault();
              if (personKey.trim()) onAction({ kind: "link", member, personKey: personKey.trim() });
            }}
          >
            <input
              aria-label="Genealogy person key"
              placeholder="Person key"
              value={personKey}
              onChange={(event) => setPersonKey(event.target.value)}
              data-testid="fm-link-person"
            />
            <button type="submit" disabled={!personKey.trim()} data-testid="fm-link">
              Link after review
            </button>
          </form>
        ) : null}
        <button
          type="button"
          disabled={!active}
          onClick={() => onAction({ kind: "deactivate", member })}
          data-testid="fm-deactivate"
        >
          Deactivate
        </button>
      </details>
    </li>
  );
}
