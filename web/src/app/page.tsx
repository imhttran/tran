"use client";

import Link from "next/link";
import { useEffect, useState } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { FormStatus, type FormStatusState } from "@/components/FormStatus";
import { PageFooter } from "@/components/PageFooter";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import { SystemStatus, type Availability } from "@/components/SystemStatus";
import {
  ApiRequestError,
  getHealth,
  submitDocument,
  type HealthResponse,
} from "@/lib/api";
import { canUpload, type Role } from "@/lib/experience-contract";

function serviceAvailability(ok: boolean | null): Availability {
  if (ok === null) return "not-checked";
  return ok ? "up" : "down";
}

export default function Page() {
  return (
    <ExperienceShell>
      {(role) => <HomeContent role={role} />}
    </ExperienceShell>
  );
}

function HomeContent({ role }: { role: Role | null }) {
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [file, setFile] = useState<File | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<FormStatusState>(null);

  useEffect(() => {
    let cancelled = false;
    getHealth()
      .then((result) => {
        if (!cancelled) {
          setHealth(result);
          setHealthError(null);
        }
      })
      .catch((error: unknown) => {
        if (!cancelled) {
          setHealth(null);
          setHealthError(
            error instanceof Error ? error.message : "The health probe failed.",
          );
        }
      });
    return () => {
      cancelled = true;
    };
  }, []);

  async function submit() {
    if (!file || busy) return;
    setBusy(true);
    setStatus({ kind: "info", text: "Uploading the submission…" });
    try {
      const form = new FormData();
      form.append("file", file);
      form.append("title", file.name);
      form.append("category", "other");
      form.append("scope", "family");
      const result = await submitDocument(form);
      setStatus({
        kind: "ok",
        text: `Submission ${result.version_id} recorded as ${result.review_status}. It is outside retrieval until an administrator approves and processes it.`,
      });
      setFile(null);
    } catch (error) {
      if (error instanceof ApiRequestError && error.body?.incomplete_pages) {
        setStatus({
          kind: "error",
          text: `${error.message} Pages: ${error.body.incomplete_pages.join(", ")}`,
        });
      } else {
        setStatus({ kind: "error", text: error instanceof Error ? error.message : "The submission failed." });
      }
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <PageTitle title="Family archive" />
      <PageHeader
        title="Family archive"
        subtitle="Submit documents for review, ask the archive, and explore the family tree."
      />
      {healthError ? (
        <p className="form-status error" role="alert" data-testid="health-error">
          {healthError}
        </p>
      ) : null}
      <SystemStatus
        api={serviceAvailability(health ? health.status === "ok" : null)}
        database={serviceAvailability(health ? health.database === "ok" : null)}
        model={serviceAvailability(health ? health.model === "ok" : null)}
      />
      {canUpload(role) ? (
        <section className="card" data-testid="upload-controls">
          <h2>Submit a document</h2>
          <p className="hint" data-testid="upload-direct-note">
            Browser uploads are submissions. They remain outside retrieval until
            an administrator reviews, approves, and processes them. The trusted
            local operator CLI remains a separate publication boundary.
          </p>
          <input
            data-testid="file-input"
            type="file"
            accept=".md,.markdown,.txt,.pdf"
            onChange={(event) => setFile(event.target.files?.[0] ?? null)}
          />
          <div className="form-actions">
            <button
              type="button"
              data-testid="ingest-button"
              onClick={() => void submit()}
              disabled={!file || busy}
            >
              Submit for review
            </button>
            <Link href="/documents">Open the review queue</Link>
          </div>
          <div data-testid="ingest-status">
            <FormStatus status={status} />
          </div>
        </section>
      ) : (
        <p className="hint" data-testid="upload-unavailable">
          Sign in as an editor or administrator to submit documents.
        </p>
      )}
      <PageFooter />
    </>
  );
}
