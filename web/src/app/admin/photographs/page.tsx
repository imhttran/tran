"use client";

import Link from "next/link";
import { useCallback, useEffect, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  decidePhotographLink,
  getAdminPhotographs,
  proposePhotographLinks,
  setPhotographVisibility,
  updatePhotographCaption,
  type AdminArchivePhotographView,
  type PhotographProposalResponse,
} from "@/lib/api";
import { canAdmin, type Role } from "@/lib/experience-contract";

// Administrator review screen for archive photographs.
//
// The Rust API is the authorization authority. This screen can edit the
// recorded caption, propose and decide person links, and publish or withdraw a
// photograph. A link is proposed from the caption, section heading, nearby
// text, or a biography page, and the reviewer's explicit choice resolves a
// namesake. Page position and appearance never create a link.

const STATE_LABEL: Record<
  AdminArchivePhotographView["publication_state"],
  string
> = {
  published: "Published",
  unpublished: "Unpublished",
};

const LINK_LABEL: Record<string, string> = {
  proposed: "Proposed",
  confirmed: "Confirmed",
  rejected: "Rejected",
};

function errorMessage(error: unknown): string {
  if (error instanceof ApiRequestError) return error.message;
  return error instanceof Error ? error.message : "The request failed.";
}

export default function AdminPhotographsPage() {
  return (
    <ExperienceShell>
      {(role) => <PhotographReviewContent role={role} />}
    </ExperienceShell>
  );
}

function PhotographReviewContent({ role }: { role: Role | null }) {
  const [photographs, setPhotographs] = useState<AdminArchivePhotographView[]>(
    [],
  );
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [captionDraft, setCaptionDraft] = useState("");
  const [visibility, setVisibility] = useState<"published" | "unpublished">(
    "unpublished",
  );
  const [proposal, setProposal] = useState<PhotographProposalResponse | null>(
    null,
  );
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);

  const selected = photographs.find((photo) => photo.asset_id === selectedId);

  const load = useCallback(async () => {
    if (!canAdmin(role)) return;
    setLoading(true);
    setError(null);
    try {
      const body = await getAdminPhotographs();
      const next = body.photographs ?? [];
      setPhotographs(next);
      setSelectedId((current) => {
        if (
          current !== null &&
          next.some((photo) => photo.asset_id === current)
        ) {
          return current;
        }
        return next[0]?.asset_id ?? null;
      });
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setLoading(false);
    }
  }, [role]);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    if (!selected) return;
    setCaptionDraft(selected.caption ?? "");
    setVisibility(selected.publication_state);
    setProposal(null);
  }, [selected]);

  async function act(action: () => Promise<unknown>, success: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await action();
      setMessage(success);
      await load();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  function saveCaption() {
    if (!selected) return;
    const caption = captionDraft.trim();
    void act(
      () => updatePhotographCaption(selected.asset_id, caption),
      caption
        ? "Caption saved. Link proposals still require an explicit match."
        : "Caption cleared.",
    );
  }

  function applyVisibility() {
    if (!selected) return;
    void act(
      () => setPhotographVisibility(selected.asset_id, visibility),
      visibility === "published"
        ? "Photograph published to the family gallery."
        : "Photograph withdrawn from family view.",
    );
  }

  function propose() {
    if (!selected) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    void proposePhotographLinks(selected.asset_id)
      .then((body) => {
        setProposal(body);
        setMessage(
          body.candidates.length === 0
            ? (body.unlinked_reason ??
                "No recorded person is supported by the caption, heading, or nearby text.")
            : "Candidates are ordered strongest first. Confirm or reject each link explicitly.",
        );
        return load();
      })
      .catch((err) => setError(errorMessage(err)))
      .finally(() => setBusy(false));
  }

  function decide(personId: number, decision: "confirm" | "reject") {
    if (!selected) return;
    void act(
      () => decidePhotographLink(selected.asset_id, personId, decision),
      decision === "confirm"
        ? "Person link confirmed."
        : "Person link rejected.",
    );
  }

  if (!canAdmin(role)) {
    return (
      <DenialView
        kind="denied"
        reason="Archive photograph review is restricted to administrators."
      />
    );
  }

  return (
    <>
      <PageTitle title="Archive photographs — review" />
      <PageHeader
        title="Archive photographs"
        subtitle="Review photographs extracted from family PDFs. Nothing is visible to the family until a reviewer publishes it."
      />

      {error ? (
        <div className="state-panel state-panel-unavailable" role="alert">
          <p>{error}</p>
          <button type="button" onClick={() => void load()}>
            Try again
          </button>
        </div>
      ) : null}
      {message ? (
        <p
          className="hint"
          role="status"
          data-testid="photograph-review-message"
        >
          {message}
        </p>
      ) : null}

      {loading ? (
        <p className="hint" role="status">
          Loading archive photographs…
        </p>
      ) : null}

      {!loading && photographs.length === 0 ? (
        <section
          className="state-panel state-panel-empty"
          data-testid="photograph-review-empty"
        >
          <h2>No archive photographs</h2>
          <p>
            Import a reviewed family PDF first. Imported photographs start
            unpublished and appear here for review.
          </p>
        </section>
      ) : null}

      {photographs.length > 0 ? (
        <div
          className="review-workspace"
          data-testid="photograph-review-console"
        >
          <aside className="review-list" aria-label="Archive photographs">
            {photographs.map((photo) => (
              <button
                type="button"
                key={photo.asset_id}
                className={`review-row${
                  photo.asset_id === selectedId ? " selected" : ""
                }`}
                aria-pressed={photo.asset_id === selectedId}
                onClick={() => setSelectedId(photo.asset_id)}
                data-testid={`photograph-row-${photo.asset_id}`}
              >
                <span className="review-row-heading">
                  {photo.caption ?? `Photograph ${photo.asset_id}`}
                </span>
                <span className="review-row-value">
                  {STATE_LABEL[photo.publication_state]} · {photo.citation}
                </span>
              </button>
            ))}
          </aside>

          {selected ? (
            <section
              className="review-detail"
              aria-labelledby="photograph-review-detail"
              data-testid="photograph-detail"
            >
              <div className="review-detail-heading">
                <p className="eyebrow">Archive photograph</p>
                <h2 id="photograph-review-detail">
                  {selected.caption ?? `Photograph ${selected.asset_id}`}
                </h2>
                <p className="meta">
                  {selected.citation} · {selected.mime_type} ·{" "}
                  {selected.width ?? "?"}×{selected.height ?? "?"} ·{" "}
                  {selected.byte_size} bytes
                </p>
              </div>

              <img
                className="family-profile-photo"
                src={selected.thumbnail_url}
                alt={selected.caption ?? "Archive photograph"}
                width={selected.width ?? undefined}
                height={selected.height ?? undefined}
              />

              <dl className="review-meta">
                <div>
                  <dt>Publication</dt>
                  <dd data-testid="photograph-publication-state">
                    {STATE_LABEL[selected.publication_state]}
                  </dd>
                </div>
                <div>
                  <dt>Classification</dt>
                  <dd>
                    {selected.classification} ({selected.classification_rule})
                  </dd>
                </div>
                <div>
                  <dt>Source object</dt>
                  <dd>{selected.object_id}</dd>
                </div>
                <div>
                  <dt>PDF pages</dt>
                  <dd>{selected.page_numbers.join(", ") || "unknown"}</dd>
                </div>
                <div>
                  <dt>Document</dt>
                  <dd>
                    version {selected.document_version_id} · {selected.corpus}
                  </dd>
                </div>
              </dl>

              <div className="review-actions">
                <label>
                  Caption
                  <textarea
                    value={captionDraft}
                    onChange={(event) => setCaptionDraft(event.target.value)}
                    rows={3}
                    data-testid="photograph-caption-input"
                  />
                </label>
                <button
                  type="button"
                  disabled={busy}
                  onClick={saveCaption}
                  data-testid="photograph-caption-save"
                >
                  Save caption
                </button>
              </div>

              <div className="review-actions">
                <label>
                  Family visibility
                  <select
                    value={visibility}
                    onChange={(event) =>
                      setVisibility(
                        event.target.value === "published"
                          ? "published"
                          : "unpublished",
                      )
                    }
                    data-testid="photograph-visibility-select"
                  >
                    <option value="unpublished">Unpublished</option>
                    <option value="published">Published</option>
                  </select>
                </label>
                <button
                  type="button"
                  disabled={busy}
                  onClick={applyVisibility}
                  data-testid="photograph-visibility-apply"
                >
                  Apply visibility
                </button>
              </div>

              <section
                className="profile-section"
                aria-labelledby="photograph-person-links"
              >
                <div className="profile-section-heading">
                  <h3 id="photograph-person-links">Person links</h3>
                  <p>
                    Proposed links come from the caption, section heading,
                    nearby text, or a biography page. Only a confirmed link
                    appears on a person’s profile. A name near the photo is a
                    candidate, not proof.
                  </p>
                </div>
                {selected.links.length === 0 ? (
                  <p className="meta" data-testid="photograph-no-links">
                    No person links.
                  </p>
                ) : (
                  <ul className="record-list">
                    {selected.links.map((link) => (
                      <li
                        key={link.person_id}
                        data-testid={`photograph-link-${link.person_id}`}
                      >
                        <span>
                          {link.display_name}{" "}
                          <span className="status-chip">
                            {LINK_LABEL[link.status] ?? link.status}
                          </span>
                          {link.confidence ? (
                            <span className="meta">
                              {" "}
                              {link.confidence} confidence
                              {link.source_page
                                ? ` · PDF page ${link.source_page}`
                                : ""}
                            </span>
                          ) : null}
                        </span>
                        {link.supporting_text ? (
                          <blockquote className="photograph-supporting-text">
                            “{link.supporting_text}”
                          </blockquote>
                        ) : null}
                        {link.confidence_basis ? (
                          <p className="meta">{link.confidence_basis}</p>
                        ) : null}
                        {link.ambiguity_note ? (
                          <p className="meta" role="note">
                            {link.ambiguity_note}
                          </p>
                        ) : null}
                        <span className="review-actions">
                          <button
                            type="button"
                            disabled={busy || link.status === "confirmed"}
                            onClick={() => decide(link.person_id, "confirm")}
                            data-testid={`photograph-link-${link.person_id}-confirm`}
                          >
                            Confirm
                          </button>
                          <button
                            type="button"
                            className="button-danger"
                            disabled={busy || link.status === "rejected"}
                            onClick={() => decide(link.person_id, "reject")}
                            data-testid={`photograph-link-${link.person_id}-reject`}
                          >
                            Reject
                          </button>
                        </span>
                      </li>
                    ))}
                  </ul>
                )}

                <div className="review-actions">
                  <button
                    type="button"
                    disabled={busy}
                    onClick={propose}
                    data-testid="photograph-propose"
                  >
                    Propose from PDF layout and nearby text
                  </button>
                  <Link
                    className="button-secondary"
                    href={selected.original_url}
                    target="_blank"
                    rel="noreferrer"
                  >
                    Open original
                  </Link>
                </div>

                {proposal ? (
                  <div data-testid="photograph-proposal">
                    {proposal.candidates.length === 0 ? (
                      <p className="meta">
                        {proposal.unlinked_reason ??
                          "No recorded person is supported by the caption, heading, or nearby text."}
                      </p>
                    ) : null}
                    {proposal.preserved_decisions > 0 ? (
                      <p className="meta">
                        {proposal.preserved_decisions} existing link decision
                        {proposal.preserved_decisions === 1 ? "" : "s"}{" "}
                        preserved.
                      </p>
                    ) : null}
                    <ul className="record-list">
                      {proposal.candidates.map((candidate) => (
                        <li
                          key={candidate.person_id}
                          data-testid={`photograph-candidate-${candidate.person_id}`}
                        >
                          <span>
                            {candidate.display_name}{" "}
                            <span className="status-chip">
                              {candidate.confidence} confidence
                            </span>
                          </span>
                          <p className="meta">
                            {candidate.source_kind.replaceAll("_", " ")} · PDF
                            page {candidate.source_page} ·{" "}
                            {candidate.match_kind} name match
                          </p>
                          <blockquote className="photograph-supporting-text">
                            “{candidate.supporting_text}”
                          </blockquote>
                          <p className="meta">{candidate.confidence_basis}</p>
                          {candidate.ambiguity ? (
                            <p className="meta" role="note">
                              {candidate.ambiguity}
                            </p>
                          ) : null}
                          <span className="review-actions">
                            <button
                              type="button"
                              disabled={busy}
                              onClick={() =>
                                decide(candidate.person_id, "confirm")
                              }
                            >
                              Confirm
                            </button>
                            <button
                              type="button"
                              className="button-danger"
                              disabled={busy}
                              onClick={() =>
                                decide(candidate.person_id, "reject")
                              }
                            >
                              Reject
                            </button>
                          </span>
                        </li>
                      ))}
                    </ul>
                  </div>
                ) : null}
              </section>
            </section>
          ) : null}
        </div>
      ) : null}
    </>
  );
}
