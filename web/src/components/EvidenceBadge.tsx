"use client";

import { evidenceStatusLabel } from "@/lib/experience-contract";
import type { EvidenceRef, EvidenceStatus } from "@/lib/experience-contract";

// Renders an evidence status and the cited evidence identifiers. Missing evidence
// shows Unverified; conflicting evidence shows Needs review. The component never
// invents evidence and never infers a fact from a model answer.
export function EvidenceBadge({ status }: { status: EvidenceStatus }) {
  return (
    <span className={`badge evidence-${status}`} data-testid={`evidence-${status}`}>
      {evidenceStatusLabel(status)}
    </span>
  );
}

export function EvidenceList({ evidence }: { evidence: EvidenceRef[] }) {
  if (evidence.length === 0) {
    return null;
  }
  return (
    <ul className="evidence-links" data-testid="evidence-links">
      {evidence.map((ref) => (
        <li key={ref.evidence_id}>
          <a href={`/evidence/${encodeURIComponent(ref.evidence_id)}`}>
            {ref.citation} ({ref.evidence_id})
          </a>
        </li>
      ))}
    </ul>
  );
}
