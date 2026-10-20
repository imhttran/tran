"use client";

import { useEffect, useState } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { HeadOfFamilyActions } from "@/components/HeadOfFamilyActions";
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

async function postJSON(
  path: string,
  body: unknown,
): Promise<{ ok: boolean; message?: string }> {
  const headers = new Headers({ "content-type": "application/json" });
  if (typeof document !== "undefined") {
    const entry = document.cookie
      .split(";")
      .map((part) => part.trim())
      .find((part) => part.startsWith("rag_csrf="));
    if (entry) {
      headers.set("x-csrf-token", decodeURIComponent(entry.slice("rag_csrf=".length)));
    }
  }
  try {
    const response = await fetch(path, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      let message = `The API returned HTTP ${response.status}.`;
      try {
        const payload = (await response.json()) as { message?: unknown };
        if (typeof payload.message === "string") message = payload.message;
      } catch {
        // Keep the status message.
      }
      return { ok: false, message };
    }
    return { ok: true };
  } catch {
    return { ok: false, message: "The API is unreachable." };
  }
}

function HeadOfFamilyContent({ role }: { role: Role | null }) {
  const [result, setResult] = useState<HeadOfFamilyResult | null>(null);
  const [mutationError, setMutationError] = useState<string | null>(null);

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
      {mutationError ? (
        <p className="insufficient" role="alert" data-testid="hof-mutation-error">
          {mutationError}
        </p>
      ) : null}
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
            }).then((outcome) => {
              if (!outcome.ok) {
                setMutationError(outcome.message ?? "The nomination failed.");
              }
            })
          }
          onApprove={(nominationId) =>
            void postJSON(
              `/api/genealogy/nominations/${encodeURIComponent(nominationId)}`,
              { approve: true },
            ).then((outcome) => {
              if (!outcome.ok) {
                setMutationError(outcome.message ?? "The approval failed.");
              }
            })
          }
          onRevoke={(appointmentId, reason) =>
            void postJSON(
              `/api/genealogy/appointments/${encodeURIComponent(appointmentId)}`,
              { reason },
            ).then((outcome) => {
              if (outcome.ok) window.location.reload();
              else setMutationError(outcome.message ?? "The revocation failed.");
            })
          }
        />
      )}
      <HeadOfFamilyActions role={role} />
    </>
  );
}
