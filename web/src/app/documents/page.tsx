"use client";

import { useCallback, useEffect, useState } from "react";

import { DenialView } from "@/components/DenialView";
import { ExperienceShell } from "@/components/ExperienceShell";
import { PageHeader } from "@/components/PageHeader";
import { PageTitle } from "@/components/PageTitle";
import {
  ApiRequestError,
  changeDocumentScope,
  listDocumentSubmissions,
  listDocumentJobs,
  processDocument,
  removeDocument,
  retryDocumentJob,
  runDocumentJobs,
  reviewDocument,
  submitDocument,
  type DocumentJob,
  type DocumentSubmission,
} from "@/lib/api";
import { canAdmin, canUpload, type Role } from "@/lib/experience-contract";

const CATEGORIES = [
  "genealogy",
  "photograph",
  "letter",
  "certificate",
  "story",
  "other",
] as const;

function errorMessage(error: unknown): string {
  if (error instanceof ApiRequestError) return error.message;
  return error instanceof Error ? error.message : "The request failed.";
}

export default function DocumentsPage() {
  return (
    <ExperienceShell>
      {(role) => <DocumentsContent role={role} />}
    </ExperienceShell>
  );
}

function DocumentsContent({ role }: { role: Role | null }) {
  const [submissions, setSubmissions] = useState<DocumentSubmission[]>([]);
  const [jobs, setJobs] = useState<DocumentJob[]>([]);
  const [file, setFile] = useState<File | null>(null);
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [category, setCategory] = useState<(typeof CATEGORIES)[number]>("other");
  const [scope, setScope] = useState<"family" | "admin_only">("family");
  const [reviewNote, setReviewNote] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    if (!canUpload(role)) return;
    try {
      const body = await listDocumentSubmissions();
      setSubmissions(body.submissions);
      if (canAdmin(role)) {
        const jobBody = await listDocumentJobs();
        setJobs(jobBody.jobs);
      }
    } catch (err) {
      setError(errorMessage(err));
    }
  }, [role]);

  useEffect(() => {
    void load();
  }, [load]);

  async function upload() {
    if (!file || busy) return;
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      const form = new FormData();
      form.append("file", file);
      form.append("title", title.trim() || file.name);
      if (description.trim()) form.append("description", description.trim());
      form.append("category", category);
      form.append("scope", scope);
      const result = await submitDocument(form);
      setMessage(
        `Submission ${result.version_id} recorded as ${result.review_status}. It is outside retrieval until an administrator approves and processes it.`,
      );
      setFile(null);
      setTitle("");
      setDescription("");
      await load();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  async function act(action: () => Promise<unknown>, success: string) {
    setBusy(true);
    setError(null);
    setMessage(null);
    try {
      await action();
      setMessage(success);
      await load();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  }

  if (!canUpload(role)) {
    return (
      <DenialView
        kind="denied"
        reason="Family document submissions and review are restricted to editors and administrators."
      />
    );
  }

  const admin = canAdmin(role);
  return (
    <>
      <PageTitle title="Family documents" />
      <PageHeader
        title="Family documents"
        subtitle="Uploads are submissions. They stay outside retrieval until an administrator approves them and processing succeeds."
      />
      {error ? <p className="insufficient" role="alert" data-testid="documents-error">{error}</p> : null}
      {message ? <p className="form-status ok" data-testid="documents-message">{message}</p> : null}

      <section className="status-panel" data-testid="document-upload">
        <h2>Submit a document</h2>
        <div className="documents-form">
          <label>
            File
            <input
              data-testid="document-file"
              type="file"
              accept=".md,.markdown,.txt,.pdf"
              onChange={(event) => setFile(event.target.files?.[0] ?? null)}
            />
          </label>
          <label>
            Title
            <input
              data-testid="document-title"
              value={title}
              onChange={(event) => setTitle(event.target.value)}
              placeholder="Family history notes"
            />
          </label>
          <label>
            Description
            <textarea
              value={description}
              onChange={(event) => setDescription(event.target.value)}
              rows={2}
            />
          </label>
          <label>
            Category
            <select
              value={category}
              onChange={(event) => setCategory(event.target.value as (typeof CATEGORIES)[number])}
            >
              {CATEGORIES.map((value) => (
                <option key={value} value={value}>{value}</option>
              ))}
            </select>
          </label>
          <label>
            Visibility
            <select
              value={scope}
              onChange={(event) => setScope(event.target.value as "family" | "admin_only")}
            >
              <option value="family">family</option>
              <option value="admin_only">admin only</option>
            </select>
          </label>
          <button type="button" onClick={() => void upload()} disabled={!file || busy} data-testid="submit-document">
            Submit for review
          </button>
        </div>
      </section>

      {admin ? (
        <section className="status-panel" data-testid="document-jobs">
          <h2>Processing queue</h2>
          <div className="documents-actions">
            <button
              type="button"
              disabled={busy}
              data-testid="run-document-jobs"
              onClick={() => void act(async () => {
                const result = await runDocumentJobs();
                setMessage(`Processed ${result.processed.length} queued job(s).`);
              }, "Processing queue run complete.")}
            >
              Run processing queue
            </button>
          </div>
          {jobs.length === 0 ? (
            <p className="hint">No processing jobs yet.</p>
          ) : (
            <table className="bulk-table">
              <thead>
                <tr><th scope="col">Job</th><th scope="col">Version</th><th scope="col">Status</th><th scope="col">Attempts</th><th scope="col">Error</th><th scope="col">Action</th></tr>
              </thead>
              <tbody>
                {jobs.map((job) => (
                  <tr key={job.job_id}>
                    <td>{job.job_id}</td>
                    <td>{job.version_id}</td>
                    <td><span className="badge">{job.status}</span></td>
                    <td>{job.attempts}/{job.max_attempts}</td>
                    <td className="hint">{job.last_error ?? "—"}</td>
                    <td>
                      {job.status === "failed" ? (
                        <button type="button" disabled={busy} onClick={() => void act(
                          () => retryDocumentJob(job.job_id),
                          `Retried job ${job.job_id}.`,
                        )}>Retry</button>
                      ) : "—"}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </section>
      ) : null}

      <section className="status-panel" data-testid="document-list">
        <h2>Submissions</h2>
        {submissions.length === 0 ? (
          <p className="hint">No submissions are visible to this account.</p>
        ) : (
          <table className="bulk-table">
            <thead>
              <tr>
                <th scope="col">Document</th>
                <th scope="col">Review</th>
                <th scope="col">Processing</th>
                <th scope="col">Visibility</th>
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {submissions.map((item) => (
                <tr key={item.version_id} data-testid={`document-row-${item.version_id}`}>
                  <td>
                    <strong>{item.title}</strong>
                    <div className="hint">
                      v{item.version_number} · {item.category} · {item.original_filename}
                    </div>
                  </td>
                  <td><span className="badge">{item.review_status}</span></td>
                  <td><span className="badge">{item.processing_status}</span></td>
                  <td>{item.access_scope ?? "—"}</td>
                  <td>
                    <div className="documents-actions">
                      <a href={`/api/documents/submissions/${item.version_id}/content`} target="_blank" rel="noreferrer">
                        Preview
                      </a>
                      {admin ? (
                        <>
                          <button
                            type="button"
                            disabled={busy || item.review_status === "approved"}
                            onClick={() => void act(
                              () => reviewDocument(item.version_id, {
                                approve: true,
                                // Keep the stored scope. Omitting it when the row
                                // has none lets the server decide instead of
                                // silently broadening a legacy row to family.
                                ...(item.access_scope ? { scope: item.access_scope } : {}),
                              }),
                              `Approved submission ${item.version_id}.`,
                            )}
                          >
                            Approve
                          </button>
                          <button
                            type="button"
                            disabled={busy || item.review_status === "rejected"}
                            onClick={() => void act(
                              () => reviewDocument(item.version_id, {
                                approve: false,
                                note: reviewNote.trim() || "Not suitable for publication",
                              }),
                              `Rejected submission ${item.version_id}.`,
                            )}
                          >
                            Reject
                          </button>
                          <select
                            value={item.access_scope ?? "family"}
                            onChange={(event) => void act(
                              () => changeDocumentScope(item.version_id, event.target.value as "family" | "admin_only"),
                              `Changed visibility for submission ${item.version_id}.`,
                            )}
                            disabled={busy}
                          >
                            <option value="family">family</option>
                            <option value="admin_only">admin only</option>
                          </select>
                          <button
                            type="button"
                            disabled={busy || item.processing_status === "ready" || item.review_status !== "approved"}
                            onClick={() => void act(
                              () => processDocument(item.version_id),
                              item.origin === "legacy_import"
                                ? `Promoted legacy submission ${item.version_id}.`
                                : `Queued submission ${item.version_id} for processing.`,
                            )}
                          >
                            {item.origin === "legacy_import" ? "Promote" : "Process"}
                          </button>
                        </>
                      ) : null}
                      <button
                        type="button"
                        disabled={busy || item.removed_at !== null}
                        onClick={() => void act(
                          () => removeDocument(item.version_id),
                          `Removed submission ${item.version_id} from effective access.`,
                        )}
                      >
                        Remove
                      </button>
                    </div>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
        {admin ? (
          <label className="documents-note">
            Rejection reason
            <input
              value={reviewNote}
              onChange={(event) => setReviewNote(event.target.value)}
              placeholder="Required when rejecting"
            />
          </label>
        ) : null}
      </section>
    </>
  );
}
