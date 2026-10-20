"use client";

import { useEffect, useRef } from "react";

import type { EvidenceRef } from "@/lib/experience-contract";

// Source drill-down built only from citation metadata that an authorized Rust
// read (tree, person, or Head of Family) already returned. It issues no request
// of its own, so it cannot widen access, and it never links to an original file:
// no authorized original-file read exists yet.
export type CitedFact = { label: string; status: string };
export type OpenSource = { evidence: EvidenceRef; citedFor: CitedFact[] };

export function CitationButton({ evidence, onOpen }: {
  evidence: EvidenceRef;
  onOpen: (evidence: EvidenceRef) => void;
}) {
  return (
    <button type="button" className="citation-button" data-testid="citation-button"
      onClick={() => onOpen(evidence)}>
      {evidence.citation}
    </button>
  );
}

export function SourceDialog({ source, onClose }: {
  source: OpenSource | null;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (source && !dialog.open) dialog.showModal();
    if (!source && dialog.open) dialog.close();
  }, [source]);

  return (
    <dialog ref={ref} className="source-dialog" onClose={onClose}
      aria-labelledby="source-dialog-title" data-testid="source-details">
      {source && (
        <>
          <h2 id="source-dialog-title">Source details</h2>
          <dl>
            <dt>Citation</dt><dd>{source.evidence.citation}</dd>
            {source.evidence.document_ref && <><dt>Document</dt><dd>{source.evidence.document_ref}</dd></>}
            {source.evidence.page != null && <><dt>Page</dt><dd>{source.evidence.page}</dd></>}
          </dl>
          {source.citedFor.length > 0 && (
            <>
              <h3>Cited for</h3>
              <ul className="family-detail-list">
                {source.citedFor.map((fact, i) => (
                  <li key={i}>{fact.label} <span className="meta">· {fact.status.replace("_", " ")}</span></li>
                ))}
              </ul>
            </>
          )}
          <p className="hint" data-testid="source-metadata-only">
            Only the citation is recorded here. The original document is not
            available to view in the archive, and a citation alone does not
            verify a relationship.
          </p>
          <form method="dialog"><button type="submit">Close</button></form>
        </>
      )}
    </dialog>
  );
}
