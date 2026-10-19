"use client";

import { useEffect, useState } from "react";
import type { FormEvent } from "react";

import { AppShell } from "@/components/AppShell";
import { PageTitle } from "@/components/PageTitle";
import { PageHeader } from "@/components/PageHeader";
import { PageFooter } from "@/components/PageFooter";
import { FormStatus, type FormStatusState } from "@/components/FormStatus";
import { SystemStatus, type Availability } from "@/components/SystemStatus";
import {
  ApiRequestError,
  askQuestion,
  getHealth,
  getStatus,
  ingestDocument,
} from "@/lib/api";
import type {
  HealthResponse,
  IngestResponse,
  QueryResponse,
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

  const [question, setQuestion] = useState("");
  const [asking, setAsking] = useState(false);
  const [answer, setAnswer] = useState<QueryResponse | null>(null);
  const [queryError, setQueryError] = useState<string | null>(null);

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

  const askStatus: FormStatusState = asking
    ? { kind: "info", text: "Retrieving and answering…" }
    : queryError
      ? { kind: "error", text: queryError }
      : null;

  async function onIngest() {
    if (!file || ingesting) return;
    setIngesting(true);
    setIngest(null);
    setIngestError(null);
    setIncompletePages([]);
    setStatus(null);
    setAnswer(null);
    setQueryError(null);
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

  async function onAsk(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = question.trim();
    if (asking || trimmed === "") return;
    setAsking(true);
    setAnswer(null);
    setQueryError(null);
    try {
      setAnswer(await askQuestion(trimmed));
    } catch (error) {
      setQueryError(
        error instanceof Error ? error.message : "The question failed.",
      );
    } finally {
      setAsking(false);
    }
  }

  const hasDocument = status !== null && status.chunks > 0;
  const connectionDown = healthError !== null;

  return (
    <AppShell>
      <PageTitle title="RAG document Q&A" />
      <PageHeader
        title="RAG document Q&A"
        subtitle="Local Next.js UI over the Rust Axum API. Ingestion, retrieval, and generation run in the Rust services against PostgreSQL and Ollama."
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

      <section className="card">
        <h2>Document upload</h2>
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

      <section className="card">
        <h2>Ask a question</h2>
        {!hasDocument ? (
          <p className="hint" data-testid="empty-state">
            No documents ingested yet. Upload a document to answer questions.
          </p>
        ) : null}
        <form onSubmit={onAsk}>
          <label htmlFor="q">Question</label>
          <input
            id="q"
            type="text"
            value={question}
            onChange={(event) => setQuestion(event.target.value)}
            placeholder="What happens if someone misses a payment?"
            aria-describedby="q-hint"
            data-testid="question-input"
          />
          <p id="q-hint" className="hint">
            Answers come only from retrieved context; the Rust answer service
            refuses when the evidence cannot answer.
          </p>
          <button
            type="submit"
            disabled={asking || question.trim() === ""}
            data-testid="ask-button"
          >
            {asking ? "Asking…" : "Ask"}
          </button>
        </form>
        <FormStatus status={askStatus} />

        <div aria-live="polite">
          {answer && answer.insufficient ? (
            <p className="insufficient" data-testid="insufficient">
              The retrieved evidence does not answer the question.
            </p>
          ) : null}

          {answer && !answer.insufficient ? (
            <>
              <h3>Answer</h3>
              <p data-testid="answer">{answer.answer}</p>
              {answer.sources.length > 0 ? (
                <>
                  <h3>Sources</h3>
                  <ul className="sources" data-testid="sources">
                    {answer.sources.map((source, index) => (
                      <li key={`${source.source}-${source.section}-${index}`}>
                        <span className="cite">
                          {source.source} — {source.section}
                        </span>
                        <span className="page">
                          {source.page === null
                            ? " (no page metadata)"
                            : ` (page ${source.page})`}
                        </span>
                        <p>{source.text}</p>
                      </li>
                    ))}
                  </ul>
                </>
              ) : null}
            </>
          ) : null}
        </div>
      </section>

      <PageFooter meta={<span>Rust Axum API • local PostgreSQL + Ollama</span>} />
    </AppShell>
  );
}
