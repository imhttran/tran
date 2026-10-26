"use client";

import { ExperienceShell } from "@/components/ExperienceShell";
import { NarrativeReviewConsole } from "@/components/NarrativeReviewConsole";

// Administrator-only review of source narrative passages. The Rust API is the
// authorization authority; this route is only reachable through the
// authenticated shell, and every read/write re-checks the admin review role.
export default function NarrativeReviewPage() {
  return (
    <ExperienceShell>
      {(role) => <NarrativeReviewConsole role={role} />}
    </ExperienceShell>
  );
}
