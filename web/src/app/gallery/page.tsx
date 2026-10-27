"use client";

import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  familyPhotographOriginalUrl,
  familyPhotographThumbnailUrl,
  getPhotographGallery,
  type GalleryPhotographView,
} from "@/lib/api";

// Family gallery of published archive photographs.
//
// This view is presentation only. Rust decides which photographs the caller may
// see and returns a published-only projection; the browser never receives a
// storage key, a filesystem path, or a content hash. Each image fetch goes back
// through the authorization-checked photograph route.

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; photographs: GalleryPhotographView[] }
  | { kind: "expired" }
  | { kind: "denied" }
  | { kind: "unavailable"; message: string };

function messageOf(error: unknown): string {
  if (error instanceof ApiRequestError && error.message) {
    return error.message;
  }
  return "The photograph gallery is unavailable.";
}

export default function GalleryPage() {
  return (
    <ExperienceShell>
      {() => <GalleryContent />}
    </ExperienceShell>
  );
}

function GalleryContent() {
  const [state, setState] = useState<LoadState>({ kind: "loading" });
  const sequence = useRef(0);

  const load = useCallback(async () => {
    const current = ++sequence.current;
    setState({ kind: "loading" });
    try {
      const body = await getPhotographGallery();
      if (sequence.current !== current) return;
      setState({ kind: "ready", photographs: body.photographs ?? [] });
    } catch (error) {
      if (sequence.current !== current) return;
      if (error instanceof ApiRequestError) {
        if (error.status === 401) {
          setState({ kind: "expired" });
          return;
        }
        if (error.status === 403) {
          setState({ kind: "denied" });
          return;
        }
      }
      setState({ kind: "unavailable", message: messageOf(error) });
    }
  }, []);

  useEffect(() => {
    void load();
    return () => {
      sequence.current += 1;
    };
  }, [load]);

  if (state.kind === "loading") {
    return (
      <>
        <PageTitle title="Photograph gallery" />
        <PageHeader title="Photograph gallery" />
        <section
          className="state-panel state-panel-loading"
          role="status"
          aria-busy="true"
          data-testid="photograph-gallery-loading"
        >
          <h2>Loading approved photographs…</h2>
          <p className="hint">
            Only photographs reviewed for the family archive appear here.
          </p>
        </section>
      </>
    );
  }

  if (state.kind === "expired") {
    return (
      <>
        <PageTitle title="Photograph gallery" />
        <PageHeader title="Photograph gallery" subtitle="Your session has ended." />
        <section
          className="state-panel state-panel-expired"
          data-testid="photograph-gallery-signin"
        >
          <h2>Sign in again</h2>
          <p>Sign in to view the family photograph archive.</p>
          <div className="state-panel-actions">
            <Link href="/signin" className="button-secondary">
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
        <PageTitle title="Photograph gallery" />
        <PageHeader title="Photograph gallery" />
        <DenialView
          kind="denied"
          reason="Your account is not linked to a verified family member, so the photograph gallery is not available to you."
        />
      </>
    );
  }

  if (state.kind === "unavailable") {
    return (
      <>
        <PageTitle title="Photograph gallery" />
        <PageHeader title="Photograph gallery" />
        <section
          className="state-panel state-panel-unavailable"
          data-testid="photograph-gallery-unavailable"
          role="alert"
        >
          <h2>Photograph gallery unavailable</h2>
          <p>{state.message}</p>
          <div className="state-panel-actions review-actions">
            <button
              type="button"
              onClick={() => void load()}
              data-testid="photograph-gallery-retry"
            >
              Try again
            </button>
          </div>
        </section>
      </>
    );
  }

  const photographs = state.photographs;

  return (
    <>
      <PageTitle title="Photograph gallery" />
      <PageHeader
        title="Photograph gallery"
        subtitle="Approved photographs from the family PDFs. Each keeps its PDF page citation."
      />
      <div data-testid="photograph-gallery">
        {photographs.length === 0 ? (
          <section
            className="state-panel state-panel-empty"
            data-testid="photograph-gallery-empty"
          >
            <h2>No approved photographs yet</h2>
            <p>
              Photographs appear here after a reviewer publishes them. Pending,
              restricted, and unpublished photographs stay hidden.
            </p>
          </section>
        ) : (
          <ul className="family-profile-media">
            {photographs.map((photo) => (
              <li
                key={photo.asset_id}
                data-testid={`gallery-photo-${photo.asset_id}`}
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
                <p className="meta" data-testid={`gallery-citation-${photo.asset_id}`}>
                  {photo.citation}
                </p>
                <p className="meta">
                  {photo.people.length > 0
                    ? `People: ${photo.people
                        .map((person) => person.display_name)
                        .join(", ")}`
                    : "No confirmed person link"}
                </p>
                <p className="meta">
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
        )}
      </div>
    </>
  );
}
