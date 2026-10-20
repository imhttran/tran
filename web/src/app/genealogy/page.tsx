"use client";

import { useEffect, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { GenealogyRecords } from "@/components/RecordList";
import type { GenealogyRecord, Role } from "@/lib/experience-contract";

// Genealogy catalog view. It renders only records the Rust API returned as
// allowed. A 403 shows an explicit denial with no record detail; a 404 shows the
// explicit no-access state. UI visibility is never an authorization decision.
export default function GenealogyPage() {
  return (
    <ExperienceShell>
      {(role) => <GenealogyContent role={role} />}
    </ExperienceShell>
  );
}

function GenealogyContent({ role }: { role: Role | null }) {
  const [records, setRecords] = useState<GenealogyRecord[] | null>(null);
  const [denial, setDenial] = useState<{
    kind: "denied" | "no_access";
    reason: string;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetch("/api/genealogy/records", { cache: "no-store" })
      .then(async (response) => {
        if (cancelled) return;
        if (response.status === 403) {
          setDenial({ kind: "denied", reason: "Your role does not permit this request." });
          return;
        }
        if (response.status === 404) {
          setDenial({ kind: "no_access", reason: "No record is available to your role." });
          return;
        }
        if (!response.ok) {
          setDenial({ kind: "denied", reason: `The API returned HTTP ${response.status}.` });
          return;
        }
        const body = (await response.json()) as { records?: GenealogyRecord[] };
        setRecords(Array.isArray(body.records) ? body.records : []);
      })
      .catch(() => {
        if (!cancelled) {
          setDenial({ kind: "denied", reason: "The API is unreachable." });
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <>
      <PageTitle title="Genealogy" />
      <PageHeader
        title="Genealogy"
        subtitle={`Read-only catalog for ${role ?? "an unauthenticated visitor"}. Rust enforces every record grant.`}
      />
      {denial ? (
        <DenialView kind={denial.kind} reason={denial.reason} />
      ) : records === null ? (
        <p className="hint" data-testid="genealogy-loading">
          Loading genealogy records…
        </p>
      ) : (
        <section className="card">
          <h2>Catalog records</h2>
          <GenealogyRecords records={records} />
        </section>
      )}
    </>
  );
}
