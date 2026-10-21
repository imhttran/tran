"use client";

import { ExperienceShell } from "@/components/ExperienceShell";
import { ReviewConsole } from "@/components/ReviewConsole";

// Administrator-only review console. The Rust API remains the authorization
// authority; this route is only reachable through the authenticated shell.
export default function ReviewsPage() {
  return (
    <ExperienceShell>
      {(role) => <ReviewConsole role={role} />}
    </ExperienceShell>
  );
}
