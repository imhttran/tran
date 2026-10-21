"use client";

import { BranchReviewConsole } from "@/components/BranchReviewConsole";
import { ExperienceShell } from "@/components/ExperienceShell";

// Branch-scoped review. Any signed-in member may open the route, but the page
// only renders the branches the Rust API says this account may review, and a
// member without a grant sees an explicit explanation instead of a queue.
export default function BranchReviewPage() {
  return (
    <ExperienceShell>
      {() => <BranchReviewConsole />}
    </ExperienceShell>
  );
}
