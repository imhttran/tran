"use client";

import { useCallback, useEffect, useRef, useState } from "react";

import { FormStatus } from "@/components/FormStatus";
import type { FormStatusState } from "@/components/FormStatus";
import { NominationPersonSelector } from "@/components/NominationPersonSelector";
import type { NominationPerson } from "@/components/NominationPersonSelector";
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

// The admin person-search endpoint extends the shared person shape with a
// `linked` flag so already-linked people can be shown but not chosen.
type AdminPerson = NominationPerson & { linked?: boolean };

type Pending =
  | { kind: "reset"; member: FamilyMember }
  | { kind: "deactivate"; member: FamilyMember }
  | { kind: "role"; member: FamilyMember; role: Role; revoke: boolean }
  | {
      kind: "link";
      member: FamilyMember;
      personKey: string;
      personLabel: string;
      branches: string[];
    }
  | { kind: "unlink"; member: FamilyMember; linkId: number };

const ROLES: Role[] = ["viewer", "editor", "admin"];
const EMPTY_FILTER: MemberFilter = { q: "", status: "", role: "" };
const ADMIN_PERSON_ENDPOINT = "/api/admin/family-members/people";

// Safe, human messages for the API's stable error codes.
const ERROR_MESSAGES: Record<string, string> = {
  reset_not_configured:
    "Password reset is not configured for this identity provider. No email was sent. The member can use the provider's own \"Forgot password?\" link.",
  reset_failed: "The identity provider did not accept the reset request. No email was sent. Try again later.",
  reset_throttled: "A reset was already requested for this account recently. Wait before trying again.",
  no_verified_email: "This account has no verified email address, so no reset can be requested.",
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
      return `Confirm you reviewed the identity proof and link ${who} to ${pending.personLabel}${
        pending.branches.length ? ` (${pending.branches.join(", ")})` : ""
      }?`;
    case "unlink":
      return `Remove the link between ${who} and their genealogy person?`;
  }
}

function confirmLabel(pending: Pending): string {
  switch (pending.kind) {
    case "reset":
      return "Confirm reset";
    case "deactivate":
      return "Confirm deactivate";
    case "role":
      return pending.revoke ? "Remove role" : "Grant role";
    case "link":
      return "Confirm link";
    case "unlink":
      return "Remove link";
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
      return `${who} is linked to ${action.personLabel}.`;
    }
    case "unlink":
      await revokeMemberLink(action.linkId);
      return `The link for ${who} was removed.`;
  }
}

// Focus returns to the control that opened the confirmation. The opener records
// itself here so a cancel or confirm can restore keyboard position.
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

export function AccountsAdmin() {
  const [filter, setFilter] = useState<MemberFilter>(EMPTY_FILTER);
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
      if (action.kind !== "reset") await load(filter, null);
    } catch (error) {
      setStatus({ kind: "error", text: messageFor(error, "The request failed. Nothing was changed.") });
    } finally {
      setBusy(false);
      setPending(null);
      restoreOpener();
    }
  }

  function cancelConfirm() {
    setPending(null);
    restoreOpener();
  }

  function ask(action: Pending) {
    rememberOpener();
    setStatus(null);
    setPending(action);
  }

  return (
    <section className="status-panel fm-accounts" data-testid="accounts">
      <div className="fm-section-head">
        <h2>Accounts</h2>
        <p className="hint">
          Every member, role, and link is served by the Rust API. Actions below
          ask for confirmation before they change anything.
        </p>
      </div>
      <form
        className="family-toolbar fm-filter-toolbar"
        role="search"
        data-testid="fm-filters"
        onSubmit={(event) => {
          event.preventDefault();
          void load(filter, null);
        }}
      >
        <label className="fm-filter-field fm-filter-search">
          <span>Search</span>
          <input
            type="search"
            placeholder="Name, email, or subject"
            value={filter.q}
            onChange={(event) => setFilter({ ...filter, q: event.target.value })}
            data-testid="fm-search"
          />
        </label>
        <label className="fm-filter-field">
          <span>Status</span>
          <select
            value={filter.status}
            onChange={(event) => setFilter({ ...filter, status: event.target.value as MemberFilter["status"] })}
            data-testid="fm-status-filter"
          >
            <option value="">Any</option>
            <option value="active">Active</option>
            <option value="deactivated">Deactivated</option>
          </select>
        </label>
        <label className="fm-filter-field">
          <span>Role</span>
          <select
            value={filter.role}
            onChange={(event) => setFilter({ ...filter, role: event.target.value as MemberFilter["role"] })}
            data-testid="fm-role-filter"
          >
            <option value="">Any</option>
            {ROLES.map((role) => (
              <option key={role} value={role}>{role}</option>
            ))}
          </select>
        </label>
        <button type="submit" className="fm-filter-apply" data-testid="fm-apply-filters">Apply</button>
      </form>

      <FormStatus status={status} />
      {loadError ? (
        <p className="state-panel state-panel-unavailable fm-state" role="alert" data-testid="fm-load-error">{loadError}</p>
      ) : null}
      {linkage === "unavailable" ? (
        <p className="state-panel state-panel-expired fm-state" data-testid="fm-linkage-unavailable">
          The genealogy store is unavailable, so linked-person status is not shown.
        </p>
      ) : null}

      <ul className="sources fm-members" data-testid="fm-members">
        {members.map((member) => (
          <MemberRow key={member.user_id} member={member} linkage={linkage} onAction={ask} />
        ))}
      </ul>
      {!loading && members.length === 0 && !loadError ? (
        <p className="state-panel state-panel-empty fm-state" data-testid="fm-empty">No accounts match these filters.</p>
      ) : null}
      {loading ? <p className="state-panel state-panel-loading fm-state">Loading accounts…</p> : null}
      {nextAfter && !loading ? (
        <button type="button" className="fm-more" onClick={() => void load(filter, nextAfter)} data-testid="fm-more">
          Load more
        </button>
      ) : null}

      {pending ? (
        <ConfirmDialog
          text={confirmText(pending)}
          confirmLabel={confirmLabel(pending)}
          busy={busy}
          onConfirm={() => void confirm()}
          onCancel={cancelConfirm}
        />
      ) : null}
    </section>
  );
}

function ConfirmDialog({
  text,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: {
  text: string;
  confirmLabel: string;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const cancelRef = useRef<HTMLButtonElement>(null);
  const dialogRef = useRef<HTMLDivElement>(null);

  // Cancel takes focus on open so Escape or Enter never runs the change by
  // accident. The confirm action stays explicit and clearly labelled.
  useEffect(() => {
    cancelRef.current?.focus();
  }, []);

  function handleKeyDown(event: React.KeyboardEvent<HTMLDivElement>) {
    if (event.key === "Escape") {
      event.preventDefault();
      onCancel();
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
      className="confirm fm-confirm"
      role="dialog"
      aria-modal="true"
      aria-labelledby="fm-confirm-title"
      onKeyDown={handleKeyDown}
      data-testid="fm-confirm"
    >
      <h3 id="fm-confirm-title">Confirm this change</h3>
      <p>{text}</p>
      <div className="fm-confirm-actions">
        <button
          ref={cancelRef}
          type="button"
          className="button-secondary"
          disabled={busy}
          onClick={onCancel}
          data-testid="fm-confirm-cancel"
        >
          Cancel
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={onConfirm}
          data-testid="fm-confirm-yes"
        >
          {busy ? "Working…" : confirmLabel}
        </button>
      </div>
    </div>
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
  const [chosen, setChosen] = useState<NominationPerson | null>(null);
  const active = member.status === "active";
  const linked = member.linked_person;
  return (
    <li className="fm-member" data-testid="fm-member" data-subject={member.subject}>
      <div className="fm-member-head">
        <strong className="fm-member-name">{label(member)}</strong>
        <div className="fm-member-chips">
          <span
            className={`status-chip status-member-${member.status}`}
            data-testid="fm-member-status"
          >
            {member.status}
          </span>
          {member.roles.map((held) => (
            <span
              key={held}
              className="status-chip status-role"
              data-testid="fm-member-role"
            >
              {held}
            </span>
          ))}
        </div>
      </div>
      <p className="meta fm-member-meta">
        <span>{member.email ?? "No email on file"}</span>
      </p>
      <p className="meta fm-member-link" data-testid="fm-member-link">
        {linked ? (
          <>
            <span>
              Linked to <strong>{linked.display_name || linked.person_key}</strong>
            </span>
            <span className="hint fm-member-link-key">
              <small>
                {linked.person_key}
                {linked.username ? ` (${linked.username})` : ""}
              </small>
            </span>
          </>
        ) : linkage === "available" ? (
          "Not linked to a genealogy person"
        ) : (
          "Link status unavailable"
        )}
      </p>
      <details className="fm-actions" data-testid="fm-actions">
        <summary>Actions</summary>
        <div className="fm-actions-body">
          <p className="meta fm-member-scope" data-testid="fm-member-scope">
            {member.subject} @ {member.issuer}
          </p>
          <button
            type="button"
            disabled={!active || !member.email}
            onClick={() => onAction({ kind: "reset", member })}
            data-testid="fm-reset"
          >
            Send password reset email
          </button>
          <div className="fm-role-row">
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
              className="fm-link-form"
              onSubmit={(event) => {
                event.preventDefault();
                if (!chosen) return;
                onAction({
                  kind: "link",
                  member,
                  personKey: chosen.record_id,
                  personLabel: chosen.person_label,
                  branches: chosen.branches,
                });
              }}
            >
              <NominationPersonSelector
                endpoint={ADMIN_PERSON_ENDPOINT}
                onSelect={setChosen}
                isDisabled={(person) => (person as AdminPerson).linked === true}
                disabledText="Already linked"
                idPrefix={`fm-link-${member.user_id}`}
              />
              <button type="submit" disabled={!chosen} data-testid="fm-link">
                Link after review
              </button>
            </form>
          ) : null}
          <button
            type="button"
            className="button-danger fm-deactivate"
            disabled={!active}
            onClick={() => onAction({ kind: "deactivate", member })}
            data-testid="fm-deactivate"
          >
            Deactivate
          </button>
        </div>
      </details>
    </li>
  );
}
