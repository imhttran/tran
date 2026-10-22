"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { FormStatus } from "@/components/FormStatus";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  cancelProfileDraft,
  createProfileDraft,
  getProfile,
  profileMediaUrl,
  restrictProfileField,
  submitProfileDraft,
  updateProfileDraft,
  uploadProfileMedia,
  withdrawProfileMedia,
} from "@/lib/api";
import type {
  ApprovedProfileValue,
  ProfileMediaAsset,
  ProfileResponse,
  ProfileRevision,
  ProfileStatus,
  ProfileVisibility,
} from "@/lib/api";
import type { Role, SessionResult } from "@/lib/experience-contract";
import {
  PROFILE_FIELDS,
  PROFILE_SECTION_COPY,
  defaultVisibility,
  profileField,
} from "@/lib/profile-fields";
import type {
  ProfileFieldDefinition,
  ProfileSection,
} from "@/lib/profile-fields";

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; data: ProfileResponse }
  | { kind: "expired" }
  | { kind: "denied"; message: string }
  | { kind: "error"; message: string };

const PHOTO_MAX_BYTES = 5 * 1024 * 1024;
const DOCUMENT_MAX_BYTES = 10 * 1024 * 1024;

export function MyProfile({
  role,
  session,
}: {
  role: Role | null;
  session: SessionResult | null;
}) {
  const [state, setState] = useState<LoadState>({ kind: "loading" });

  const load = useCallback(async () => {
    setState({ kind: "loading" });
    try {
      const data = await getProfile();
      setState({ kind: "ready", data });
    } catch (error) {
      setState(loadFailure(error));
    }
  }, []);

  useEffect(() => {
    if (!session) return;
    if (session.kind === "expired") {
      setState({ kind: "expired" });
      return;
    }
    if (session.kind === "denied" || session.kind === "no_access") {
      setState({ kind: "denied", message: session.reason });
      return;
    }
    if (session.data.state !== "signed_in") return;
    void load();
  }, [load, session]);

  if (!session) {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader
          title="My profile"
          subtitle="Loading your private profile…"
        />
        <p className="hint" data-testid="profile-loading" role="status">
          Loading your profile…
        </p>
      </>
    );
  }

  // An anonymous session (including a deactivated or unprovisioned identity)
  // must render the safe sign-in invitation, never an endless loading state.
  if (session.kind === "allowed" && session.data.state === "anonymous") {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader
          title="My profile"
          subtitle="Sign in to contribute your story, photographs, and family memories."
        />
        <section className="card profile-empty" data-testid="profile-signin">
          <p>
            Sign in with your family account to view and edit your private
            profile.
          </p>
          <a
            className="button-link"
            href="/signin"
            data-testid="profile-signin-button"
          >
            Sign in
          </a>
        </section>
      </>
    );
  }

  if (state.kind === "loading") {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader
          title="My profile"
          subtitle="Loading your private profile…"
        />
        <p className="hint" data-testid="profile-loading" role="status">
          Loading your profile…
        </p>
      </>
    );
  }

  if (state.kind === "expired") {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader title="My profile" subtitle="Your session has expired." />
        <section className="card profile-empty" data-testid="profile-expired">
          <p>Your session expired. Sign in again to continue.</p>
          <a
            className="button-link"
            href="/signin"
            data-testid="profile-signin-button"
          >
            Sign in
          </a>
        </section>
      </>
    );
  }

  if (state.kind === "denied") {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader title="My profile" />
        <DenialView kind="denied" reason={state.message} />
      </>
    );
  }

  if (state.kind === "error") {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader
          title="My profile"
          subtitle="Your profile could not be loaded."
        />
        <FormStatus status={{ kind: "error", text: state.message }} />
        <button
          type="button"
          onClick={() => void load()}
          data-testid="profile-retry"
        >
          Try again
        </button>
      </>
    );
  }

  if (!state.data.linked || !state.data.profile) {
    return (
      <>
        <PageTitle title="My profile" />
        <PageHeader
          title="My profile"
          subtitle="Your account is signed in but not yet connected to a genealogy person."
        />
        <section className="card profile-empty" data-testid="profile-unlinked">
          <h2>Your profile is waiting for a verified family link</h2>
          <p>
            An administrator reviews and connects accounts to the correct
            genealogy person. Until that happens, this page does not show or
            accept private profile information.
          </p>
          <p className="hint">
            You can still explore the family tree and the archive. If you
            believe your account should be linked, contact the family
            administrator.
          </p>
        </section>
      </>
    );
  }

  return (
    <ProfileReady role={role} data={state.data} onChanged={() => void load()} />
  );
}

function loadFailure(error: unknown): LoadState {
  if (error instanceof ApiRequestError) {
    if (error.status === 401) return { kind: "expired" };
    if (error.status === 403) {
      return {
        kind: "denied",
        message: "Your account is not permitted to view this profile.",
      };
    }
  }
  return {
    kind: "error",
    message:
      error instanceof Error
        ? error.message
        : "The profile service is unavailable. Try again shortly.",
  };
}

function ProfileReady({
  role,
  data,
  onChanged,
}: {
  role: Role | null;
  data: ProfileResponse;
  onChanged: () => void;
}) {
  const profile = data.profile!;
  const media = data.media ?? [];
  const approved = useMemo(
    () => new Map(profile.approved.map((value) => [value.field_key, value])),
    [profile.approved],
  );
  const openRevisions = useMemo(
    () =>
      new Map(
        profile.pending.map((revision) => [revision.field_key, revision]),
      ),
    [profile.pending],
  );
  const feedback = useMemo(
    () => latestFeedback(profile.history),
    [profile.history],
  );

  return (
    <div className="profile-workspace">
      <PageTitle title="My profile" />
      <ProfileHero
        role={role}
        name={profile.display_name}
        approved={profile.approved}
        pending={profile.pending}
        feedback={profile.history}
        media={media}
      />

      <ProfileSection
        section="about"
        approved={approved}
        openRevisions={openRevisions}
        feedback={feedback}
        onChanged={onChanged}
      />
      <ProfileSection
        section="story"
        approved={approved}
        openRevisions={openRevisions}
        feedback={feedback}
        onChanged={onChanged}
      />
      <ProfileSection
        section="memories"
        approved={approved}
        openRevisions={openRevisions}
        feedback={feedback}
        onChanged={onChanged}
      />

      <ProfileMediaSection
        kind="photo"
        media={media}
        history={profile.history}
        onChanged={onChanged}
      />
      <ProfileMediaSection
        kind="document"
        media={media}
        history={profile.history}
        onChanged={onChanged}
      />
      <ReviewHistory history={profile.history} />
    </div>
  );
}

function ProfileHero({
  role,
  name,
  approved,
  pending,
  feedback,
  media,
}: {
  role: Role | null;
  name: string;
  approved: ApprovedProfileValue[];
  pending: ProfileRevision[];
  feedback: ProfileRevision[];
  media: ProfileMediaAsset[];
}) {
  const photo = currentMedia(media, "profile_photo");
  const approvedFields = approved.filter(
    (value) => value.value.trim() !== "",
  ).length;
  const approvedMedia = media.filter(
    (asset) => asset.status === "approved",
  ).length;
  const total = PROFILE_FIELDS.length + 2;
  const completed = approvedFields + approvedMedia;
  const pendingCount = pending.length;
  const feedbackCount = feedback.filter(
    (revision) =>
      revision.status === "rejected" || revision.status === "needs_changes",
  ).length;
  const missing = PROFILE_FIELDS.filter(
    (field) =>
      !approved.some(
        (value) => value.field_key === field.key && value.value.trim() !== "",
      ),
  ).slice(0, 3);

  return (
    <section className="profile-hero" data-testid="profile-header">
      <div className="profile-photo-panel">
        {photo ? (
          <img
            className="profile-photo"
            src={profileMediaUrl(photo.media_id)}
            alt="Your profile photograph"
            data-testid="profile-photo-preview"
          />
        ) : (
          <div
            className="profile-photo profile-photo-placeholder"
            aria-hidden="true"
          >
            <span>☺</span>
          </div>
        )}
        {photo ? <StatusChip status={photo.status} /> : null}
      </div>
      <div className="profile-hero-copy">
        <p className="eyebrow">My profile</p>
        <h1 data-testid="profile-name">{name}</h1>
        <p className="hint">
          {role
            ? `Signed in with the ${role} role.`
            : "Signed in as a family member."}{" "}
          Your family branch appears in the family tree only when authorized
          verified evidence supports it.
        </p>
        <div className="profile-progress" data-testid="profile-completion">
          <div className="profile-progress-label">
            <span>Profile completion</span>
            <strong>
              {completed} of {total}
            </strong>
          </div>
          <progress
            value={completed}
            max={total}
            aria-label="Profile completion"
          />
          {missing.length > 0 ? (
            <p className="hint">
              Next: {missing.map((field) => field.label).join(", ")}.
            </p>
          ) : (
            <p className="hint">
              Your profile has the core pieces future generations will look for.
            </p>
          )}
        </div>
        <div className="profile-status-summary">
          <span className="badge" data-testid="profile-pending-count">
            {pendingCount} open submission{pendingCount === 1 ? "" : "s"}
          </span>
          {feedbackCount > 0 ? (
            <span
              className="badge badge-feedback"
              data-testid="profile-feedback-count"
            >
              {feedbackCount} review decision{feedbackCount === 1 ? "" : "s"}
            </span>
          ) : null}
        </div>
      </div>
    </section>
  );
}

function ProfileSection({
  section,
  approved,
  openRevisions,
  feedback,
  onChanged,
}: {
  section: ProfileSection;
  approved: Map<string, ApprovedProfileValue>;
  openRevisions: Map<string, ProfileRevision>;
  feedback: Map<string, ProfileRevision>;
  onChanged: () => void;
}) {
  const copy = PROFILE_SECTION_COPY[section];
  const fields = PROFILE_FIELDS.filter((field) => field.section === section);
  return (
    <section
      className="profile-section"
      data-testid={`profile-section-${section}`}
    >
      <div className="profile-section-heading">
        <h2>{copy.title}</h2>
        <p>{copy.intro}</p>
      </div>
      <div className="profile-field-grid">
        {fields.map((field) => (
          <ProfileFieldCard
            key={field.key}
            field={field}
            approved={approved.get(field.key)}
            openRevision={openRevisions.get(field.key)}
            feedback={feedback.get(field.key)}
            onChanged={onChanged}
          />
        ))}
      </div>
    </section>
  );
}

function ProfileFieldCard({
  field,
  approved,
  openRevision,
  feedback,
  onChanged,
}: {
  field: ProfileFieldDefinition;
  approved?: ApprovedProfileValue;
  openRevision?: ProfileRevision;
  feedback?: ProfileRevision;
  onChanged: () => void;
}) {
  const baseline =
    openRevision?.value ?? approved?.value ?? feedback?.value ?? "";
  const [editing, setEditing] = useState(false);
  const [value, setValue] = useState(baseline);
  const [visibility, setVisibility] = useState<ProfileVisibility>(
    openRevision?.visibility ??
      feedback?.visibility ??
      approved?.visibility ??
      defaultVisibility(field),
  );
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const dirty = editing && value !== baseline;

  useEffect(() => {
    if (!editing) {
      setValue(baseline);
      setVisibility(
        openRevision?.visibility ??
          feedback?.visibility ??
          approved?.visibility ??
          defaultVisibility(field),
      );
    }
  }, [
    approved?.visibility,
    baseline,
    editing,
    feedback?.visibility,
    field,
    openRevision?.visibility,
  ]);

  useEffect(() => {
    if (!dirty) return;
    const handler = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", handler);
    return () => window.removeEventListener("beforeunload", handler);
  }, [dirty]);

  function beginEditing(seed?: string) {
    setValue(seed ?? baseline);
    setVisibility(
      openRevision?.visibility ??
        feedback?.visibility ??
        approved?.visibility ??
        defaultVisibility(field),
    );
    setError(null);
    setEditing(true);
  }

  function cancelEditing() {
    if (dirty && !window.confirm("Discard your unsaved changes?")) return;
    setEditing(false);
    setError(null);
  }

  async function saveDraft() {
    setBusy("save");
    setError(null);
    try {
      if (openRevision?.status === "draft") {
        await updateProfileDraft(openRevision.revision_id, {
          value,
          visibility,
        });
      } else {
        await createProfileDraft({ field_key: field.key, value, visibility });
      }
      setEditing(false);
      onChanged();
      setBusy(null);
    } catch (err) {
      setError(messageOf(err));
      setBusy(null);
    }
  }

  async function submit() {
    if (!openRevision || openRevision.status !== "draft") return;
    setBusy("submit");
    setError(null);
    try {
      await submitProfileDraft(openRevision.revision_id);
      setEditing(false);
      onChanged();
      setBusy(null);
    } catch (err) {
      setError(messageOf(err));
      setBusy(null);
    }
  }

  async function makePrivate() {
    if (
      !approved ||
      approved.visibility !== "family" ||
      approved.restricted_from_family
    ) {
      return;
    }
    if (
      !window.confirm(
        "Make this private? It will immediately stop appearing on family profile pages.",
      )
    ) {
      return;
    }
    setBusy("restrict");
    setError(null);
    try {
      await restrictProfileField(field.key);
      setEditing(false);
      onChanged();
      setBusy(null);
    } catch (err) {
      setError(messageOf(err));
      setBusy(null);
    }
  }

  async function withdraw() {
    if (!openRevision || openRevision.status !== "draft") return;
    if (!window.confirm("Withdraw this draft? It will not be sent for review."))
      return;
    setBusy("withdraw");
    setError(null);
    try {
      await cancelProfileDraft(openRevision.revision_id);
      setEditing(false);
      onChanged();
      setBusy(null);
    } catch (err) {
      setError(messageOf(err));
      setBusy(null);
    }
  }

  return (
    <article
      className="profile-field-card"
      data-testid={`profile-field-${field.key}`}
    >
      <header className="profile-field-heading">
        <div>
          <h3>{field.label}</h3>
          <p className="hint">{field.helper}</p>
        </div>
        {openRevision ? <StatusChip status={openRevision.status} /> : null}
      </header>

      {editing ? (
        <div className="profile-editor">
          <label htmlFor={`profile-${field.key}`}>{field.label}</label>
          {field.multiline ? (
            <textarea
              id={`profile-${field.key}`}
              value={value}
              rows={field.rows}
              maxLength={field.maxLength}
              placeholder={field.placeholder}
              onChange={(event) => setValue(event.target.value)}
              data-testid={`profile-input-${field.key}`}
            />
          ) : (
            <input
              id={`profile-${field.key}`}
              type="text"
              value={value}
              maxLength={field.maxLength}
              placeholder={field.placeholder}
              onChange={(event) => setValue(event.target.value)}
              data-testid={`profile-input-${field.key}`}
            />
          )}
          <div className="profile-editor-meta">
            <label htmlFor={`profile-visibility-${field.key}`}>
              Who can see this after approval?
            </label>
            <select
              id={`profile-visibility-${field.key}`}
              value={visibility}
              onChange={(event) =>
                setVisibility(event.target.value as ProfileVisibility)
              }
              data-testid={`profile-visibility-${field.key}`}
            >
              <option value="owner_private">Owner-private</option>
              <option value="family">Authorized family</option>
            </select>
            <span className="hint">
              {value.length}/{field.maxLength}
            </span>
          </div>
          {error ? (
            <FormStatus status={{ kind: "error", text: error }} />
          ) : null}
          <div className="profile-actions">
            <button
              type="button"
              onClick={() => void saveDraft()}
              disabled={busy !== null || value.trim() === ""}
              data-testid={`profile-save-${field.key}`}
            >
              {busy === "save"
                ? "Saving…"
                : openRevision?.status === "draft"
                  ? "Save changes"
                  : "Save draft"}
            </button>
            {openRevision?.status === "draft" ? (
              <>
                <button
                  type="button"
                  onClick={() => void submit()}
                  disabled={busy !== null || value.trim() === "" || dirty}
                  data-testid={`profile-submit-${field.key}`}
                >
                  {busy === "submit" ? "Submitting…" : "Submit for review"}
                </button>
                <button
                  type="button"
                  className="button-secondary"
                  onClick={() => void withdraw()}
                  disabled={busy !== null}
                  data-testid={`profile-withdraw-${field.key}`}
                >
                  Withdraw draft
                </button>
              </>
            ) : null}
            <button
              type="button"
              className="button-secondary"
              onClick={cancelEditing}
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <>
          <div className="profile-value">
            {approved?.value ? (
              <p>{approved.value}</p>
            ) : (
              <p className="hint">Not added yet.</p>
            )}
            {approved ? (
              <span className="hint" data-testid={`profile-visibility-${field.key}`}>
                Approved ·{" "}
                {approved.restricted_from_family
                  ? "private (you restricted it from family pages)"
                  : approved.visibility === "family"
                    ? "Authorized family"
                    : "Owner-private"}
              </span>
            ) : null}
            {approved?.visibility === "family" &&
            !approved.restricted_from_family ? (
              <button
                type="button"
                className="button-secondary"
                onClick={() => void makePrivate()}
                disabled={busy !== null}
                data-testid={`profile-make-private-${field.key}`}
              >
                {busy === "restrict" ? "Making private…" : "Make private"}
              </button>
            ) : null}
            {approved?.restricted_from_family ? (
              <p className="hint" data-testid={`profile-restricted-${field.key}`}>
                Hidden from family pages. To share it again, submit a new
                family-visible revision for independent review.
              </p>
            ) : null}
          </div>

          {openRevision ? (
            <div
              className="profile-open-revision"
              data-testid={`profile-open-${field.key}`}
            >
              <div className="profile-open-heading">
                <StatusChip status={openRevision.status} />
                <span className="hint">
                  Revision {openRevision.revision_number}
                  {openRevision.status === "pending_review"
                    ? " · submitted for review"
                    : " · saved draft"}
                </span>
              </div>
              <p className="profile-draft-value">{openRevision.value}</p>
              <div className="profile-actions">
                {openRevision.status === "draft" ? (
                  <>
                    <button
                      type="button"
                      onClick={() => beginEditing()}
                      data-testid={`profile-edit-${field.key}`}
                    >
                      Edit draft
                    </button>
                    <button
                      type="button"
                      onClick={() => void submit()}
                      disabled={busy !== null}
                      data-testid={`profile-submit-${field.key}`}
                    >
                      {busy === "submit" ? "Submitting…" : "Submit for review"}
                    </button>
                    <button
                      type="button"
                      className="button-secondary"
                      onClick={() => void withdraw()}
                      disabled={busy !== null}
                      data-testid={`profile-withdraw-${field.key}`}
                    >
                      Withdraw draft
                    </button>
                  </>
                ) : (
                  <p className="hint">
                    Your approved information remains visible while this
                    submission is reviewed.
                  </p>
                )}
              </div>
            </div>
          ) : null}

          {!openRevision && feedback ? (
            <div
              className="profile-feedback"
              data-testid={`profile-feedback-${field.key}`}
            >
              <StatusChip status={feedback.status} />
              <p className="profile-draft-value">{feedback.value}</p>
              {feedback.review_note ? <p>{feedback.review_note}</p> : null}
              <div className="profile-actions">
                <button
                  type="button"
                  onClick={() => beginEditing(feedback.value)}
                  data-testid={`profile-revise-${field.key}`}
                >
                  Revise and resubmit
                </button>
              </div>
            </div>
          ) : null}

          {!openRevision && !editing ? (
            <div className="profile-actions">
              <button
                type="button"
                onClick={() => beginEditing()}
                data-testid={`profile-add-${field.key}`}
              >
                {approved ? "Edit" : "Add"}
              </button>
            </div>
          ) : null}
          {error ? (
            <FormStatus status={{ kind: "error", text: error }} />
          ) : null}
        </>
      )}
    </article>
  );
}

function ProfileMediaSection({
  kind,
  media,
  history,
  onChanged,
}: {
  kind: "photo" | "document";
  media: ProfileMediaAsset[];
  history: ProfileRevision[];
  onChanged: () => void;
}) {
  const fieldKey = kind === "photo" ? "profile_photo" : "personal_document";
  const approved = latestWithStatus(media, fieldKey, ["approved"]);
  const pending = latestWithStatus(media, fieldKey, ["pending_review"]);
  const decision = latestWithStatus(media, fieldKey, [
    "rejected",
    "needs_changes",
  ]);
  const current = approved ?? pending ?? decision;
  const [visibility, setVisibility] = useState<ProfileVisibility>(
    current?.visibility ?? "owner_private",
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  const title = kind === "photo" ? "Profile photograph" : "Personal document";
  const accept =
    kind === "photo" ? "image/jpeg,image/png" : ".pdf,.txt,.md,.markdown";
  const maxBytes = kind === "photo" ? PHOTO_MAX_BYTES : DOCUMENT_MAX_BYTES;
  const uploadDisabled = pending !== undefined;

  async function upload(file: File) {
    const invalid = validateMedia(kind, file);
    if (invalid) {
      setError(invalid);
      if (inputRef.current) inputRef.current.value = "";
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await uploadProfileMedia(kind, file, visibility);
      onChanged();
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(false);
      if (inputRef.current) inputRef.current.value = "";
    }
  }

  async function withdraw(asset: ProfileMediaAsset) {
    if (
      !window.confirm(
        "Withdraw this pending submission? The private file will be removed.",
      )
    )
      return;
    setBusy(true);
    setError(null);
    try {
      await withdrawProfileMedia(asset.media_id);
      onChanged();
    } catch (err) {
      setError(messageOf(err));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section
      className="profile-media-card"
      data-testid={`profile-media-${kind}`}
    >
      <div className="profile-section-heading">
        <h2>{title}</h2>
        <p className="hint">
          {kind === "photo"
            ? "JPEG or PNG, up to 5 MiB. The server re-encodes the image and removes metadata."
            : "PDF, text, or Markdown, up to 10 MiB. Owner-private until approved for family visibility."}
        </p>
      </div>

      {current ? (
        <div
          className="profile-media-current"
          data-testid={`profile-${kind}-current`}
        >
          {kind === "photo" ? (
            <img
              className="profile-photo profile-photo-large"
              src={profileMediaUrl(current.media_id)}
              alt="Current profile photograph"
              data-testid="profile-photo-preview"
            />
          ) : (
            <div className="profile-document-summary">
              <strong>{current.file_name}</strong>
              <span className="hint">{formatBytes(current.byte_size)}</span>
              <a
                href={profileMediaUrl(current.media_id)}
                target="_blank"
                rel="noreferrer"
                data-testid="profile-document-download"
              >
                View or download
              </a>
            </div>
          )}
          <StatusChip status={current.status} />
          {current.status === "pending_review" ? (
            <button
              type="button"
              className="button-secondary"
              onClick={() => void withdraw(current)}
              disabled={busy}
              data-testid={`profile-${kind}-withdraw`}
            >
              Withdraw pending {kind === "photo" ? "photo" : "document"}
            </button>
          ) : null}
        </div>
      ) : (
        <p className="hint" data-testid={`profile-${kind}-empty`}>
          {kind === "photo"
            ? "No photograph yet. Add one to preserve your family’s history."
            : "No personal document yet. Share a document that future generations should keep."}
        </p>
      )}

      {pending && pending.media_id !== current?.media_id ? (
        <div
          className="profile-media-pending"
          data-testid={`profile-${kind}-pending`}
        >
          <div className="profile-open-heading">
            <StatusChip status={pending.status} />
            <span className="hint">A replacement is waiting for review.</span>
          </div>
          <button
            type="button"
            className="button-secondary"
            onClick={() => void withdraw(pending)}
            disabled={busy}
            data-testid={`profile-${kind}-withdraw`}
          >
            Withdraw pending {kind === "photo" ? "photo" : "document"}
          </button>
        </div>
      ) : null}

      {decision ? (
        <p
          className="profile-feedback-note"
          data-testid={`profile-${kind}-feedback`}
        >
          Latest decision: {statusLabel(decision.status)}.
          {mediaNote(history, decision.revision_id)}
        </p>
      ) : null}

      <div className="profile-media-upload">
        <label htmlFor={`profile-${kind}-file`}>
          {kind === "photo"
            ? "Upload or replace photograph"
            : "Upload or replace personal document"}
        </label>
        <input
          ref={inputRef}
          id={`profile-${kind}-file`}
          type="file"
          accept={accept}
          disabled={busy || uploadDisabled}
          onChange={(event) => {
            const file = event.target.files?.[0];
            if (file) void upload(file);
          }}
          data-testid={`profile-${kind}-input`}
        />
        <div className="profile-editor-meta">
          <label htmlFor={`profile-${kind}-visibility`}>
            Who can see this after approval?
          </label>
          <select
            id={`profile-${kind}-visibility`}
            value={visibility}
            onChange={(event) =>
              setVisibility(event.target.value as ProfileVisibility)
            }
            disabled={busy}
            data-testid={`profile-${kind}-visibility`}
          >
            <option value="owner_private">Owner-private</option>
            <option value="family">Authorized family</option>
          </select>
        </div>
        {uploadDisabled ? (
          <p className="hint">
            A submission is already pending review. Withdraw it before uploading
            a replacement.
          </p>
        ) : null}
        {error ? <FormStatus status={{ kind: "error", text: error }} /> : null}
      </div>
    </section>
  );
}

function ReviewHistory({ history }: { history: ProfileRevision[] }) {
  const decisions = history
    .filter((revision) => revision.status !== "draft")
    .slice(0, 20);
  return (
    <section className="profile-history" data-testid="profile-history">
      <div className="profile-section-heading">
        <h2>Review history</h2>
        <p className="hint">
          Every submission and decision is recorded. Reviewer feedback is shown
          here; approved values stay visible while a change is under review.
        </p>
      </div>
      {decisions.length === 0 ? (
        <p className="hint" data-testid="profile-history-empty">
          No review decisions yet. Save a draft and submit it when you are
          ready.
        </p>
      ) : (
        <ol className="profile-history-list">
          {decisions.map((revision) => (
            <li
              key={revision.revision_id}
              data-testid={`profile-history-${revision.revision_id}`}
            >
              <div className="profile-history-heading">
                <strong>
                  {profileField(revision.field_key)?.label ??
                    revision.field_key}
                </strong>
                <StatusChip status={revision.status} />
                <span className="hint">
                  {formatDate(revision.reviewed_at ?? revision.changed_at)}
                </span>
              </div>
              {revision.review_note ? <p>{revision.review_note}</p> : null}
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}

function StatusChip({ status }: { status: ProfileStatus }) {
  return (
    <span
      className={`status-chip status-${status}`}
      data-testid={`status-${status}`}
    >
      {statusLabel(status)}
    </span>
  );
}

function statusLabel(status: ProfileStatus): string {
  switch (status) {
    case "draft":
      return "Draft";
    case "pending_review":
      return "Pending review";
    case "approved":
      return "Approved";
    case "rejected":
      return "Rejected";
    case "needs_changes":
      return "Needs changes";
  }
}

function latestFeedback(
  history: ProfileRevision[],
): Map<string, ProfileRevision> {
  const result = new Map<string, ProfileRevision>();
  for (const revision of history) {
    if (revision.status !== "rejected" && revision.status !== "needs_changes")
      continue;
    const existing = result.get(revision.field_key);
    if (!existing || revision.changed_at > existing.changed_at) {
      result.set(revision.field_key, revision);
    }
  }
  return result;
}

function currentMedia(
  media: ProfileMediaAsset[],
  kind: "profile_photo" | "personal_document",
): ProfileMediaAsset | undefined {
  const approved = latestWithStatus(media, kind, ["approved"]);
  if (approved) return approved;
  return (
    latestWithStatus(media, kind, ["pending_review"]) ??
    latestWithStatus(media, kind, ["rejected", "needs_changes"])
  );
}

function latestWithStatus(
  media: ProfileMediaAsset[],
  kind: "profile_photo" | "personal_document",
  statuses: ProfileStatus[],
): ProfileMediaAsset | undefined {
  return media
    .filter((asset) => asset.kind === kind && statuses.includes(asset.status))
    .sort((a, b) => b.revision_id - a.revision_id)[0];
}

function mediaNote(history: ProfileRevision[], revisionId: number): string {
  const revision = history.find((item) => item.revision_id === revisionId);
  return revision?.review_note ? ` ${revision.review_note}` : "";
}

function validateMedia(kind: "photo" | "document", file: File): string | null {
  if (file.size === 0) return "The selected file is empty.";
  if (file.size > (kind === "photo" ? PHOTO_MAX_BYTES : DOCUMENT_MAX_BYTES)) {
    return kind === "photo"
      ? "Photographs must be 5 MiB or smaller."
      : "Personal documents must be 10 MiB or smaller.";
  }
  if (kind === "photo") {
    if (!["image/jpeg", "image/png"].includes(file.type)) {
      return "Choose a JPEG or PNG photograph.";
    }
    return null;
  }
  const extension = file.name.split(".").pop()?.toLowerCase() ?? "";
  if (!["pdf", "txt", "md", "markdown"].includes(extension)) {
    return "Choose a PDF, text, or Markdown document.";
  }
  return null;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

function formatDate(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError) {
    if (error.status === 401)
      return "Your session expired. Sign in again to continue.";
    if (error.status === 403)
      return "Your account is not permitted to make this change.";
    if (error.status === 413) return "The selected file is too large.";
    if (error.body?.message) return error.body.message;
  }
  return error instanceof Error
    ? error.message
    : "The request failed. Try again.";
}
