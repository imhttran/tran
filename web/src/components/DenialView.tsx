"use client";

import type { OutcomeKind } from "@/lib/experience-contract";

// Explicit denial view for denied and no-access results. It shows no record
// detail, so a denial cannot leak the existence or contents of a record.
type Props = {
  kind: Extract<OutcomeKind, "denied" | "no_access">;
  reason: string;
};

export function DenialView({ kind, reason }: Props) {
  return (
    <section
      className="state-panel state-panel-unavailable"
      data-testid={kind === "denied" ? "denied-view" : "no-access-view"}
      role="alert"
    >
      <h2>{kind === "denied" ? "Access denied" : "No access"}</h2>
      <p>{reason}</p>
    </section>
  );
}
