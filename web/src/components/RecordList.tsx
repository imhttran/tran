"use client";

import { EvidenceBadge } from "@/components/EvidenceBadge";
import type { GenealogyRecord, SearchHit } from "@/lib/experience-contract";

// Renders only records that Rust returned as allowed. Unauthorized records never
// reach this component, so they are absent from the DOM rather than hidden with
// styling.
export function SearchResults({ hits }: { hits: SearchHit[] }) {
  if (hits.length === 0) {
    return (
      <p className="hint" data-testid="search-empty">
        No records are available to your role.
      </p>
    );
  }
  return (
    <ul className="results" data-testid="search-results">
      {hits.map((hit) => (
        <li key={hit.record_id} data-testid="search-hit">
          <p>{hit.title}</p>
          <span className="meta">corpus: {hit.corpus}</span>
          <EvidenceBadge status={hit.evidence_status} />
        </li>
      ))}
    </ul>
  );
}

export function GenealogyRecords({ records }: { records: GenealogyRecord[] }) {
  if (records.length === 0) {
    return (
      <p className="hint" data-testid="genealogy-empty">
        No genealogy records are available to your role.
      </p>
    );
  }
  return (
    <ul className="results" data-testid="genealogy-records">
      {records.map((record) => (
        <li key={record.record_id} data-testid="genealogy-record">
          <p>{record.person_label}</p>
          <span className="meta">{record.relationship}</span>
          <EvidenceBadge status={record.evidence_status} />
        </li>
      ))}
    </ul>
  );
}
