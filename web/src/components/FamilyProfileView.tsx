"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  getFamilyProfile,
  type FamilyProfileFieldView,
  type FamilyProfileResponse,
} from "@/lib/api";
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

const SECTION_ORDER: FieldSection[] = ["about", "story", "memories"];

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
    // Clear first: navigating between people must never show stale content.
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
        <p className="hint" role="status" data-testid="family-profile-loading">
          Loading this family profile…
        </p>
      </>
    );
  }

  if (state.kind === "expired") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" subtitle="Your session has ended." />
        <section className="card profile-empty" data-testid="family-profile-signin">
          <p>Sign in to view approved family profiles.</p>
          <Link href="/signin" className="button-secondary" data-testid="family-profile-signin-button">
            Sign in
          </Link>
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
        <section className="card" data-testid="family-profile-not-found" role="status">
          <h2>Profile not available</h2>
          <p>This family profile could not be found.</p>
          <p className="hint">
            <Link href="/genealogy">Back to the family tree</Link>
          </p>
        </section>
      </>
    );
  }

  if (state.kind === "unavailable") {
    return (
      <>
        <PageTitle title="Family profile" />
        <PageHeader title="Family profile" />
        <section className="card" data-testid="family-profile-unavailable" role="alert">
          <h2>Family profile unavailable</h2>
          <p>{state.message}</p>
          <div className="review-actions">
            <button type="button" onClick={() => void load()} data-testid="family-profile-retry">
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
  const displayName = profile.person.display_name;

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

      <div className="profile-workspace" data-testid="family-profile">
        {presented.length === 0 ? (
          <section className="card" data-testid="family-profile-empty">
            <h2>No family information yet</h2>
            <p>
              {displayName} has not published any family-visible profile
              information yet. Nothing is shown until their own approved
              contribution is available.
            </p>
          </section>
        ) : (
          SECTION_ORDER.map((section) => {
            const fields = presented.filter((field) => field.section === section);
            if (fields.length === 0) return null;
            const copy = SECTION_COPY[section];
            return (
              <section
                key={section}
                className="profile-section"
                aria-labelledby={`family-profile-${section}`}
                data-testid={`family-profile-section-${section}`}
              >
                <div className="profile-section-heading">
                  <h2 id={`family-profile-${section}`}>{copy.title}</h2>
                  <p>{copy.intro}</p>
                </div>
                {fields.map((field) => (
                  <article key={field.field_key} data-testid={`family-profile-field-${field.field_key}`}>
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
          })
        )}
      </div>
    </>
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
    if (error.status === 503) {
      return "The family profile service is temporarily unavailable.";
    }
    if (error.status === 0) {
      return "The family profile could not be reached. Check your connection and try again.";
    }
    if (error.body?.message) return error.body.message;
  }
  return error instanceof Error
    ? error.message
    : "The family profile could not be loaded.";
}
