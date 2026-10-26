"use client";

import { useEffect, useRef, useState } from "react";

import type { OpenSource } from "@/components/SourceDetails";
import {
  ApiRequestError,
  getFamilyHistory,
  type FamilyProfilePassageView,
} from "@/lib/api";

// Personal history opened from a family-tree node.
//
// This is a presentation-only dialog. The Rust API decides whether the caller
// may read the passages and returns only approved, family-visible, linked
// history; no client state can widen that answer. The dialog fetches only when
// it opens, and every failure replaces the content so a previous person's
// history can never remain on screen.

export type HistoryPerson = {
  person_key: string;
  person_label: string;
};

type LoadState =
  | { kind: "loading" }
  | { kind: "ready"; displayName: string; passages: FamilyProfilePassageView[] }
  | { kind: "empty"; displayName: string }
  | { kind: "unavailable"; message: string };

export function PersonalHistoryDialog({
  person,
  onClose,
  onOpenSource,
}: {
  /** The person whose history is open, or null when the dialog is closed. */
  person: HistoryPerson | null;
  onClose: () => void;
  onOpenSource: (source: OpenSource) => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const closeRef = useRef<HTMLButtonElement>(null);
  const sequence = useRef(0);
  const [state, setState] = useState<LoadState>({ kind: "loading" });

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;

    if (!person) {
      if (dialog.open) dialog.close();
      return;
    }

    // Clear first: a failure for this person must not leave the previous
    // person's passages visible while the new request is in flight.
    setState({ kind: "loading" });
    const current = ++sequence.current;
    if (!dialog.open) dialog.showModal();
    closeRef.current?.focus({ preventScroll: true });

    const controller = new AbortController();
    getFamilyHistory(person.person_key, controller.signal)
      .then((body) => {
        if (sequence.current !== current || controller.signal.aborted) return;
        if (body.person.person_key !== person.person_key) return;
        setState(body.history.length === 0
          ? { kind: "empty", displayName: body.person.display_name }
          : {
            kind: "ready",
            displayName: body.person.display_name,
            passages: body.history,
          });
      })
      .catch((error: unknown) => {
        if (sequence.current !== current || controller.signal.aborted) return;
        // An expired session, a denial, and a missing person all replace the
        // content. The server's body is never rendered.
        if (error instanceof ApiRequestError &&
            (error.status === 401 || error.status === 403 || error.status === 404)) {
          setState({
            kind: "unavailable",
            message: "Personal history is not available to this account.",
          });
          return;
        }
        setState({
          kind: "unavailable",
          message: "Personal history could not be loaded. Try again in a moment.",
        });
      });

    return () => controller.abort();
  }, [person]);

  return (
    <dialog
      ref={dialogRef}
      className="personal-history-dialog"
      aria-labelledby="personal-history-title"
      data-testid="personal-history-dialog"
      onCancel={(event) => {
        // Escape closes the dialog through this path. Preventing the native
        // close keeps the parent as the single owner of the open state, so a
        // queued close event can never clear a newer open (close/reopen race).
        event.preventDefault();
        onClose();
      }}
    >
      <div className="personal-history-head">
        <div>
          <h2 id="personal-history-title">Personal history</h2>
          <p className="meta" data-testid="personal-history-person">
            {person ? person.person_label : ""}
          </p>
        </div>
        <button
          type="button"
          ref={closeRef}
          className="personal-history-close"
          onClick={onClose}
          data-testid="personal-history-close"
        >
          Close
        </button>
      </div>
      <div className="personal-history-body">
        {state.kind === "loading" && (
          <p role="status" className="hint" data-testid="personal-history-loading">
            Loading personal history…
          </p>
        )}
        {state.kind === "unavailable" && (
          <p role="alert" className="hint" data-testid="personal-history-error">
            {state.message}
          </p>
        )}
        {state.kind === "empty" && (
          <p className="hint" data-testid="personal-history-empty">
            No published personal history is available for {state.displayName}.
          </p>
        )}
        {state.kind === "ready" && (
          <>
            <p className="hint">
              Reviewed passages from the family archive, kept separate from the
              member&apos;s own profile writing. The wording is reproduced exactly as
              recorded in the source.
            </p>
            {state.passages.map((passage) => (
              <HistoryPassage
                key={passage.passage_id}
                passage={passage}
                personLabel={state.displayName}
                onOpenSource={onOpenSource}
              />
            ))}
          </>
        )}
      </div>
    </dialog>
  );
}

function HistoryPassage({
  passage,
  personLabel,
  onOpenSource,
}: {
  passage: FamilyProfilePassageView;
  personLabel: string;
  onOpenSource: (source: OpenSource) => void;
}) {
  const language = passage.language === "vi" ? "Vietnamese" : "English";
  const pages = passage.page_start === passage.page_end
    ? `page ${passage.page_start}`
    : `pages ${passage.page_start}–${passage.page_end}`;
  const documentRef = passage.document_ref?.trim() || "Family archive";
  const section = passage.section_title?.trim() || "Untitled section";
  const paragraphs = passage.body.split(/\n{2,}/);

  return (
    <article
      className="family-history-passage personal-history-passage"
      data-testid="personal-history-passage"
    >
      <h3>{section}</h3>
      <dl className="personal-history-source">
        <dt>Language</dt>
        <dd>{language}</dd>
        <dt>Source document</dt>
        <dd>{documentRef}</dd>
        <dt>PDF pages</dt>
        <dd>{pages}</dd>
      </dl>
      {paragraphs.map((paragraph, index) => (
        <p key={index} className="family-history-text">{paragraph}</p>
      ))}
      <button
        type="button"
        className="family-text-button personal-history-source-link"
        data-testid="personal-history-source-link"
        onClick={() => onOpenSource({
          // Citation metadata only: the dialog never invents an excerpt, and
          // the existing source control states that no original file is served.
          evidence: {
            evidence_id: `narrative-${passage.passage_id}`,
            citation: `${section} — ${documentRef}, PDF ${pages}`,
            document_ref: passage.document_ref ?? undefined,
            page: passage.page_start,
          },
          citedFor: [{
            label: `Personal history for ${personLabel}`,
            status: "approved",
          }],
        })}
      >
        View source details
      </button>
    </article>
  );
}
