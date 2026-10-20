"use client";

import { useEffect, useState } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { HeadOfFamilyView } from "@/components/HeadOfFamilyView";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { outcomeFromStatus } from "@/lib/experience-contract";
import type { HeadOfFamilyView as HeadOfFamilyData, HeadOfFamilyResult, Role } from "@/lib/experience-contract";

// Head of Family view. Every conclusion and evidence identifier comes from Rust;
// the page never derives a head from a model answer. Editor nomination and admin
// approval/revocation controls require an explicit confirmation before any
// request is sent.
export default function HeadOfFamilyPage() {
  return (
    <ExperienceShell>
      {(role) => <HeadOfFamilyContent role={role} />}
    </ExperienceShell>
  );
}

async function postJSON(path: string, body: unknown): Promise<void> {
  await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
}

function HeadOfFamilyContent({ role }: { role: Role | null }) {
  const [result, setResult] = useState<HeadOfFamilyResult | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/api/genealogy/head-of-family", { cache: "no-store" })
      .then(async (response) => {
        if (cancelled) return;
        let payload: unknown = null;
        try {
          payload = await response.json();
        } catch {
          payload = null;
        }
        		setResult(outcomeFromStatus<HeadOfFamilyData>(response.status, payload as HeadOfFamilyData | null));
      })
      .catch(() => {
        if (!cancelled) {
          setResult({ kind: "denied", reason: "The API is unreachable." });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <>
      <PageTitle title="Head of Family" />
      <PageHeader
        title="Head of Family"
        subtitle="A genealogy designation backed by cited evidence. It is separate from application administrator permissions."
      />
      {result === null ? (
        <p className="hint" data-testid="hof-loading">
          Loading the Head of Family record…
        </p>
      ) : (
        <HeadOfFamilyView
          role={role}
          result={result}
          onNominate={(recordId, candidateLabel) =>
            void postJSON("/api/genealogy/nominations", {
              record_id: recordId,
              candidate_label: candidateLabel,
            })
          }
          onApprove={(nominationId) =>
            void postJSON(
              `/api/genealogy/nominations/${encodeURIComponent(nominationId)}`,
              { nomination_id: nominationId },
            )
          }
          onRevoke={(appointmentId, reason) =>
            void postJSON(
              `/api/genealogy/appointments/${encodeURIComponent(appointmentId)}`,
              { reason },
            )
          }
        />
      )}
    </>
  );
}
