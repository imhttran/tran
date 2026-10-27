"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  familyPhotographOriginalUrl,
  familyPhotographThumbnailUrl,
  familyProfileMediaUrl,
  getFamilyProfile,
  type FamilyProfileDateView,
  type FamilyProfileFieldView,
  type FamilyProfilePassageView,
  type FamilyProfileResponse,
} from "@/lib/api";
import { formatStructuredDate, precisionLabel } from "@/lib/dates";
import { initialLetter } from "@/lib/person-display";
import { profileField } from "@/lib/profile-fields";

// USR-006-B: the family-facing profile and biography page.
//
// This page is presentation only. Rust decides whether the caller may read the
// person and returns only approved, family-visible values; nothing here can
// widen that answer. The component keeps its own field allow-list so contact
// and location fields stay out of a family history page even if an owner had
// marked such a value family-visible (publication contract §4/§5).
//
// Every failure fails closed: an expired session, a denial, a missing person,
// and an unavailable store each replace the content instead of leaving a
// previous person's biography on screen.

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; profile: FamilyProfileResponse }
  | { kind: "expired" }
  | { kind: "denied" }
  | { kind: "not_found" }
  | { kind: "unavailable"; message: string };

// Field order for a family history page. Narrative and identity fields only:
// `current_location`, `contact_preferences`, `facebook_url`, `linkedin_url`,
// `snapchat`, and `whatsapp` are deliberately excluded until the owner-visible
// policy question (contract decision A/D) is settled. The goal is a family
// history page, not a contact directory.
const PRESENTED_FIELDS: { key: string; section: FieldSection }[] = [
  { key: "display_name", section: "about" },
  { key: "preferred_name", section: "about" },
  { key: "biography", section: "about" },
  { key: "my_story", section: "story" },
  { key: "family_memories", section: "memories" },
];

type FieldSection = "about" | "story" | "memories";

const SECTION_COPY: Record<FieldSection, { title: string; intro: string }> = {
  about: {
    title: "About",
    intro: "Approved family-visible information about this person.",
  },
  story: {
    title: "Personal story",
    intro: "A personal narrative shared for the family archive.",
  },
  memories: {
    title: "Family memories",
    intro: "Recollections about parents, grandparents, and family traditions.",
  },
};

export function FamilyProfileView({ personKey }: { personKey: string }) {
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const sequence = useRef(0);

  const load = useCallback(async () => {
    const current = ++sequence.current;
    // Clear first: a reused instance must never keep showing the previous
    // person while the new request is in flight. The route also keys this
    // component by person key, so a same-route change remounts with a fresh
    // state instead of relying on this reset alone.
    setState({ kind: "loading" });
    if (!personKey) {
      setState({ kind: "not_found" });
      return;
    }
    try {
      const profile = await getFamilyProfile(personKey);
      if (sequence.current !== current) return;
      setState({ kind: "ready", profile });
    } catch (err) {
      if (sequence.current !== current) return;
      if (err instanceof ApiRequestError) {
        if (err.status === 401) {
          setState({ kind: "expired" });
          return;
        }
        if (err.status === 403) {
          setState({ kind: "denied" });
          return;
        }
        if (err.status === 404) {
          setState({ kind: "not_found" });
          return;
        }
      }
      setState({
        kind: "unavailable",
        message: messageOf(err),
      });
    }
  }, [personKey]);

  useEffect(() => {
    void load();
    return () => {
      // Invalidate any in-flight response when the person changes.
      sequence.current += 1;
    };
  }, [load]);

  if (state.kind === "loading") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" />
        <section
          className="state-panel state-panel-loading"
          role="status"
          aria-busy="true"
          data-testid="family-profile-loading"
        >
          <h2>Loading this family profile…</h2>
          <p className="hint">Approved family information is on its way.</p>
        </section>
      </>
    );
  }

  if (state.kind === "expired") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" subtitle="Your session has ended." />
        <section
          className="state-panel state-panel-expired"
          data-testid="family-profile-signin"
        >
          <h2>Sign in again</h2>
          <p>Sign in to view approved family profiles.</p>
          <div className="state-panel-actions">
            <Link
              href="/signin"
              className="button-secondary"
              data-testid="family-profile-signin-button"
            >
              Sign in
            </Link>
          </div>
        </section>
      </>
    );
  }

  if (state.kind === "denied") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" />
        <DenialView
          kind="denied"
          reason="Your account is not linked to a verified family member, so this profile is not available to you."
        />
        <p className="hint">
          <Link href="/genealogy">Back to the family tree</Link>
        </p>
      </>
    );
  }

  if (state.kind === "not_found") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" />
        <section
          className="state-panel state-panel-not-found"
          data-testid="family-profile-not-found"
          role="status"
        >
          <h2>Profile not available</h2>
          <p>This family profile could not be found.</p>
          <div className="state-panel-actions">
            <Link href="/genealogy" className="button-secondary">
              Back to the family tree
            </Link>
          </div>
        </section>
      </>
    );
  }

  if (state.kind === "unavailable") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" />
        <section
          className="state-panel state-panel-unavailable"
          data-testid="family-profile-unavailable"
          role="alert"
        >
          <h2>Family profile unavailable</h2>
          <p>{state.message}</p>
          <div className="state-panel-actions review-actions">
            <button
              type="button"
              onClick={() => void load()}
              data-testid="family-profile-retry"
            >
              Try again
            </button>
            <Link href="/genealogy" className="button-secondary">
              Back to the family tree
            </Link>
          </div>
        </section>
      </>
    );
  }

  const { profile } = state;
  const presented = presentFields(profile.fields);
  // Verified dates are optional in the payload; an older or mocked response that
  // omits them must not break the page.
  const dates = profile.dates ?? [];
  const history = profile.history ?? [];
  const displayName = profile.person.display_name;
  // Media is presentation only as well: the API already excluded pending,
  // rejected, owner-private, restricted, and unprovenanced assets, and each
  // `<img>`/link below re-authorizes server-side when the browser fetches it.
  const photos = profile.media.filter((item) => item.kind === "profile_photo");
  const documents = profile.media.filter(
    (item) => item.kind === "personal_document",
  );
  // Archive photographs come from reviewed family PDFs. They are a separate
  // section from owner-uploaded profile media: the API already excluded
  // unpublished and restricted assets, and every image fetch re-authorizes.
  const archivePhotographs = profile.archive_photographs ?? [];
  const hasContent =
    presented.length > 0 ||
    photos.length > 0 ||
    documents.length > 0 ||
    dates.length > 0 ||
    history.length > 0 ||
    archivePhotographs.length > 0;
  // The summary photo is the newest approved photograph, if the family shared
  // one. No photo means the initials fallback renders and no `<img>` at all.
  const summaryPhoto = photos[0];
  const birth = dates.find((date) => date.kind === "birth_date");
  const death = dates.find((date) => date.kind === "death_date");
  const fieldsOf = (section: FieldSection) =>
    presented.filter((field) => field.section === section);

  return (
    <>
      <PageTitle title={`${displayName} — family profile`} />
      <p className="family-profile-back">
        <Link href="/genealogy" data-testid="family-profile-back">
          ← Back to the family tree
        </Link>
      </p>
      <PageHeader
        title={displayName}
        subtitle="Approved family information. Only values this family member chose to share with the family appear here."
      />

      <div
        className="profile-workspace family-profile-layout"
        data-testid="family-profile"
      >
        <aside
          className="family-profile-summary"
          aria-labelledby="family-profile-summary-heading"
          data-testid="family-profile-summary"
        >
          <div className="family-profile-summary-media">
            {summaryPhoto ? (
              <img
                className="family-profile-summary-photo"
                data-testid="family-profile-summary-photo"
                src={familyProfileMediaUrl(
                  profile.person.person_key,
                  summaryPhoto.media_id,
                )}
                alt={`Photograph shared by ${displayName}`}
                width={summaryPhoto.width ?? undefined}
                height={summaryPhoto.height ?? undefined}
              />
            ) : (
              <span
                className="family-profile-summary-initial"
                data-testid="family-profile-summary-initial"
                aria-hidden="true"
              >
                {initialLetter(displayName)}
              </span>
            )}
          </div>
          <h2
            id="family-profile-summary-heading"
            className="family-profile-summary-name"
          >
            {displayName}
          </h2>
          <p
            className="meta family-profile-summary-dates"
            data-testid="family-profile-summary-dates"
          >
            {birth || death ? (
              <span>{lifespan(birth, death)}</span>
            ) : (
              <span>No verified dates</span>
            )}
          </p>
          <dl
            className="family-profile-counts"
            data-testid="family-profile-counts"
          >
            <div>
              <dt>Approved facts</dt>
              <dd data-testid="family-profile-count-fields">
                {presented.length}
              </dd>
            </div>
            <div>
              <dt>Dates</dt>
              <dd data-testid="family-profile-count-dates">{dates.length}</dd>
            </div>
            <div>
              <dt>History passages</dt>
              <dd data-testid="family-profile-count-history">
                {history.length}
              </dd>
            </div>
            <div>
              <dt>Photographs</dt>
              <dd data-testid="family-profile-count-photos">{photos.length}</dd>
            </div>
            <div>
              <dt>Archive photographs</dt>
              <dd data-testid="family-profile-count-archive-photos">
                {archivePhotographs.length}
              </dd>
            </div>
            <div>
              <dt>Documents</dt>
              <dd data-testid="family-profile-count-documents">
                {documents.length}
              </dd>
            </div>
          </dl>
          <p className="hint family-profile-summary-link">
            <Link
              href="/genealogy"
              data-testid="family-profile-summary-tree-link"
            >
              See this person in the family tree
            </Link>
          </p>
        </aside>

        <div className="family-profile-main">
          {fieldsOf("about").length > 0 ? (
            <FieldSectionBlock section="about" fields={fieldsOf("about")} />
          ) : null}

          {fieldsOf("story").length > 0 ? (
            <FieldSectionBlock section="story" fields={fieldsOf("story")} />
          ) : null}

          {fieldsOf("memories").length > 0 ? (
            <FieldSectionBlock
              section="memories"
              fields={fieldsOf("memories")}
            />
          ) : null}

          {dates.length > 0 ? (
            <section
              className="profile-section"
              aria-labelledby="family-profile-dates"
              data-testid="family-profile-dates"
            >
              <div className="profile-section-heading">
                <h2 id="family-profile-dates">Dates</h2>
                <p>
                  Reviewed dates from the family archive. Each keeps the
                  precision the source states.
                </p>
              </div>
              <dl className="family-profile-dates">
                {dates.map((date) => (
                  <div
                    key={date.kind}
                    data-testid={`family-profile-date-${date.kind}`}
                  >
                    <dt>{date.kind === "birth_date" ? "Born" : "Died"}</dt>
                    <dd>
                      <span className="family-profile-date-value">
                        {formatStructuredDate(date)}
                      </span>
                      <span className="meta">
                        {" "}
                        · {precisionLabel(date.precision)}
                      </span>
                      {date.value ? (
                        <span className="meta">
                          {" "}
                          · as written: “{date.value}”
                        </span>
                      ) : null}
                    </dd>
                  </div>
                ))}
              </dl>
            </section>
          ) : null}

          {history.length > 0 ? (
            <section
              className="profile-section"
              aria-labelledby="family-profile-history"
              data-testid="family-profile-history"
            >
              <div className="profile-section-heading">
                <h2 id="family-profile-history">
                  History from the family archive
                </h2>
                <p>
                  Passages from the family history, reviewed for the family
                  archive. The wording is kept exactly as written in the source.
                </p>
              </div>
              <div className="family-profile-prose">
                {history.map((passage) => (
                  <HistoryPassage key={passage.passage_id} passage={passage} />
                ))}
              </div>
            </section>
          ) : null}

          {photos.length > 0 ? (
            <section
              className="profile-section"
              aria-labelledby="family-profile-photos"
              data-testid="family-profile-photos"
            >
              <div className="profile-section-heading">
                <h2 id="family-profile-photos">Photographs</h2>
                <p>Approved photographs shared with the family.</p>
              </div>
              <ul className="family-profile-media">
                {photos.map((photo) => (
                  <li
                    key={photo.media_id}
                    data-testid={`family-profile-photo-${photo.media_id}`}
                  >
                    <img
                      className="family-profile-photo"
                      src={familyProfileMediaUrl(
                        profile.person.person_key,
                        photo.media_id,
                      )}
                      alt={`Photograph shared by ${displayName}`}
                      width={photo.width ?? undefined}
                      height={photo.height ?? undefined}
                      loading="lazy"
                    />
                    <p className="meta">
                      Approved version {photo.approved_revision_number} ·{" "}
                      {formatDate(photo.created_at)}
                    </p>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          {documents.length > 0 ? (
            <section
              className="profile-section"
              aria-labelledby="family-profile-documents"
              data-testid="family-profile-documents"
            >
              <div className="profile-section-heading">
                <h2 id="family-profile-documents">Shared documents</h2>
                <p>Documents approved for the family archive.</p>
              </div>
              <ul className="family-profile-media">
                {documents.map((document) => (
                  <li
                    key={document.media_id}
                    className="family-profile-document"
                    data-testid={`family-profile-document-${document.media_id}`}
                  >
                    <a
                      href={familyProfileMediaUrl(
                        profile.person.person_key,
                        document.media_id,
                      )}
                      target="_blank"
                      rel="noreferrer"
                    >
                      {document.file_name}
                    </a>
                    <span className="meta">
                      Approved version {document.approved_revision_number} ·{" "}
                      {formatDate(document.created_at)}
                    </span>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          {archivePhotographs.length > 0 ? (
            <section
              className="profile-section"
              aria-labelledby="family-profile-archive-photographs"
              data-testid="family-profile-archive-photographs"
            >
              <div className="profile-section-heading">
                <h2 id="family-profile-archive-photographs">
                  Archive photographs
                </h2>
                <p>
                  Photographs from reviewed family PDFs, approved for the family
                  archive. Each keeps the PDF page it came from, separate from
                  photographs uploaded to this profile.
                </p>
              </div>
              <ul className="family-profile-media">
                {archivePhotographs.map((photo) => (
                  <li
                    key={photo.asset_id}
                    data-testid={`family-profile-archive-photo-${photo.asset_id}`}
                  >
                    <img
                      className="family-profile-photo"
                      src={familyPhotographThumbnailUrl(photo.asset_id)}
                      alt={photo.caption ?? "Archive photograph"}
                      width={photo.width ?? undefined}
                      height={photo.height ?? undefined}
                      loading="lazy"
                    />
                    {photo.caption ? (
                      <p className="family-profile-text">“{photo.caption}”</p>
                    ) : (
                      <p className="meta">No recorded caption</p>
                    )}
                    <p className="meta">
                      {photo.citation} ·{" "}
                      <a
                        href={familyPhotographOriginalUrl(photo.asset_id)}
                        target="_blank"
                        rel="noreferrer"
                      >
                        Open original
                      </a>
                    </p>
                  </li>
                ))}
              </ul>
            </section>
          ) : null}

          {hasContent ? null : (
            <section
              className="state-panel state-panel-empty"
              data-testid="family-profile-empty"
            >
              <h2>No family information yet</h2>
              <p>
                {displayName} has not published any family-visible profile
                information yet. Nothing is shown until their own approved
                contribution is available.
              </p>
            </section>
          )}
        </div>
      </div>
    </>
  );
}

function FieldSectionBlock({
  section,
  fields,
}: {
  section: FieldSection;
  fields: PresentedField[];
}) {
  const copy = SECTION_COPY[section];
  return (
    <section
      className="profile-section"
      aria-labelledby={`family-profile-${section}`}
      data-testid={`family-profile-section-${section}`}
    >
      <div className="profile-section-heading">
        <h2 id={`family-profile-${section}`}>{copy.title}</h2>
        <p>{copy.intro}</p>
      </div>
      {fields.map((field) => (
        <article
          key={field.field_key}
          data-testid={`family-profile-field-${field.field_key}`}
        >
          <h3>{labelOf(field.field_key)}</h3>
          <p className="family-profile-text">{field.value}</p>
          <p className="meta">
            Approved version {field.approved_revision_number} ·{" "}
            {formatDate(field.updated_at)}
          </p>
        </article>
      ))}
    </section>
  );
}

type PresentedField = FamilyProfileFieldView & { section: FieldSection };

/// Keeps only the fields this page presents, in the documented order.
function presentFields(values: FamilyProfileFieldView[]): PresentedField[] {
  const byKey = new Map(values.map((value) => [value.field_key, value]));
  const out: PresentedField[] = [];
  for (const { key, section } of PRESENTED_FIELDS) {
    const value = byKey.get(key);
    if (value && value.value.trim().length > 0) {
      out.push({ ...value, section });
    }
  }
  return out;
}

function labelOf(fieldKey: string): string {
  return profileField(fieldKey)?.label ?? fieldKey;
}

/// A concise verified lifespan for the summary card.
///
/// Only verified dates from the payload reach this function. A missing date is
/// omitted instead of guessed, so no relationship or status is invented.
function lifespan(
  birth: FamilyProfileDateView | undefined,
  death: FamilyProfileDateView | undefined,
): string {
  const birthText = birth ? formatStructuredDate(birth) : "";
  const deathText = death ? formatStructuredDate(death) : "";
  if (birthText && deathText) return `${birthText} – ${deathText}`;
  if (birthText) return `Born ${birthText}`;
  if (deathText) return `Died ${deathText}`;
  return "";
}

/// One approved archive passage: source wording, language, and PDF citation.
function HistoryPassage({ passage }: { passage: FamilyProfilePassageView }) {
  const pages =
    passage.page_start === passage.page_end
      ? `page ${passage.page_start}`
      : `pages ${passage.page_start}–${passage.page_end}`;
  const language = passage.language === "vi" ? "Vietnamese" : "English";
  const paragraphs = passage.body.split(/\n{2,}/);
  return (
    <article
      className="family-history-passage"
      data-testid={`family-history-passage-${passage.passage_id}`}
    >
      {passage.section_title ? <h3>{passage.section_title}</h3> : null}
      <p className="meta" data-testid="family-history-source">
        {language} source · PDF {pages}
        {passage.document_ref ? ` · ${passage.document_ref}` : ""}
      </p>
      {paragraphs.map((paragraph, index) => (
        <p key={index} className="family-history-text">
          {paragraph}
        </p>
      ))}
    </article>
  );
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

/// Fixed, user-facing text for the failure states.
///
/// The message is never derived from the response: an unexpected status (for
/// example a 500) may carry a server or proxy body, and that body must not be
/// rendered on a family page. Authorization outcomes (401/403/404) are handled
/// before this function is reached.
function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError) {
    if (error.status === 0) {
      return "The family profile could not be reached. Check your connection and try again.";
    }
    if (error.status === 503) {
      return "The family profile service is temporarily unavailable. Try again in a moment.";
    }
  }
  return "The family profile could not be loaded. Try again in a moment.";
}
