"use client";

import Link from "next/link";
import { useEffect, useState } from "react";

import { ExperienceShell } from "@/components/ExperienceShell";
import { PageTitle } from "@/components/PageTitle";
import { PageHeader } from "@/components/PageHeader";
import { PageFooter } from "@/components/PageFooter";
import { FormStatus, type FormStatusState } from "@/components/FormStatus";
import { SystemStatus, type Availability } from "@/components/SystemStatus";
import {
  ApiRequestError,
  getHealth,
  getStatus,
  ingestDocument,
} from "@/lib/api";
import { canUpload } from "@/lib/experience-contract";
import type {
  HealthResponse,
  IngestResponse,
  StatusResponse,
} from "@/lib/api";

function serviceAvailability(ok: boolean | null): Availability {
  if (ok === null) return "not-checked";
  return ok ? "up" : "down";
}

export default function Page() {
  const [health, setHealth] = useState<HealthResponse | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);

  const [file, setFile] = useState<File | null>(null);
  const [fileName, setFileName] = useState<string | null>(null);
  const [ingesting, setIngesting] = useState(false);
  const [ingest, setIngest] = useState<IngestResponse | null>(null);
  const [ingestError, setIngestError] = useState<string | null>(null);
  const [incompletePages, setIncompletePages] = useState<number[]>([]);
  const [status, setStatus] = useState<StatusResponse | null>(null);

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

  const ingestStatus: FormStatusState = ingesting
    ? { kind: "info", text: "Uploading and ingesting…" }
    : ingestError
      ? { kind: "error", text: ingestError }
      : ingest
        ? {
            kind: "ok",
            text: `${ingest.unchanged ? "Already ingested" : "Ingested"}: ${ingest.chunks} chunk(s) for ${ingest.source}.`,
          }
        : null;

  async function onIngest() {
    if (!file || ingesting) return;
    setIngesting(true);
    setIngest(null);
    setIngestError(null);
    setIncompletePages([]);
    setStatus(null);
    try {
      const result = await ingestDocument(file);
      setIngest(result);
      setStatus(await getStatus(result.source));
    } catch (error) {
      if (error instanceof ApiRequestError && error.body?.incomplete_pages) {
        setIncompletePages(error.body.incomplete_pages);
      }
      setIngestError(
        error instanceof Error ? error.message : "Ingestion failed.",
      );
    } finally {
      setIngesting(false);
    }
  }

  const connectionDown = healthError !== null;

  return (
    <ExperienceShell>
      {(role) => (
        <>
      <PageTitle title="Add documents" />
      <PageHeader
        title="Add documents"
        subtitle={<>Add a document to the archive, then <Link href="/ask">ask the archive</Link> about it.</>}
      />

      <SystemStatus
        api={connectionDown ? "down" : serviceAvailability(health !== null)}
        database={
          connectionDown
            ? "down"
            : serviceAvailability(health ? health.database === "ok" : null)
        }
        model={
          connectionDown
            ? "down"
            : serviceAvailability(health ? health.model === "ok" : null)
        }
      />

      {healthError ? (
        <p className="form-status error" role="alert" data-testid="health-error">
          {healthError}
        </p>
      ) : null}

      {canUpload(role) ? (
      <section className="card" data-testid="upload-controls">
        <h2>Document upload</h2>
        <p className="hint" data-testid="upload-direct-note">
          Uploading ingests the file immediately; there is no review step. It
          becomes searchable as soon as ingestion finishes.
        </p>
        <label htmlFor="doc">
          Choose one supported file (.md, .markdown, .txt, .pdf)
        </label>
        <input
          id="doc"
          type="file"
          accept=".md,.markdown,.txt,.pdf"
          aria-describedby="doc-meta"
          data-testid="file-input"
          onChange={(event) => {
            const selected = event.target.files && event.target.files[0];
            setFile(selected ?? null);
            setFileName(selected ? selected.name : null);
            setIngest(null);
            setIngestError(null);
            setIncompletePages([]);
            setStatus(null);
          }}
        />
        <p id="doc-meta" className="meta">
          {fileName ? `Selected: ${fileName}` : "No file selected."}
        </p>
        <button
          type="button"
          onClick={onIngest}
          disabled={!file || ingesting}
          data-testid="ingest-button"
        >
          {ingesting ? "Ingesting…" : "Start ingestion"}
        </button>
        <FormStatus status={ingestStatus} />
        {incompletePages.length > 0 ? (
          <p className="insufficient" data-testid="incomplete-pdf">
            Incomplete PDF extraction — pages requiring OCR:{" "}
            <strong>{incompletePages.join(", ")}</strong>. No data was written.
          </p>
        ) : null}
        {status ? (
          <div className="meta" data-testid="ingest-status">
            <p>
              Stored <strong>{status.chunks}</strong> chunk(s) for{" "}
              <code>{status.source}</code> in corpus <code>{status.corpus}</code>.
            </p>
            <ul className="sources">
              {status.sections.map((section) => (
                <li key={`${section.section}-${section.chunk_index}`}>
                  <span className="cite">
                    {section.section} — chunk {section.chunk_index}
                  </span>
                  <span className="page">
                    {section.page === null ? " (no page metadata)" : ` (page ${section.page})`}
                  </span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </section>

        ) : (
      <p className="hint" data-testid="upload-unavailable">
        Adding documents requires an editor or administrator account. You can
        still <Link href="/ask">ask the archive</Link> or browse the{" "}
        <Link href="/genealogy">family tree</Link>.
      </p>
        )}

      <PageFooter meta={<span>Rust Axum API • local PostgreSQL + Ollama</span>} />
        </>
      )}
    </ExperienceShell>
  );
}
