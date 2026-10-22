//! Durable document processing jobs (FAM-004).
//!
//! A queued job is claimed with `FOR UPDATE SKIP LOCKED`, revalidated against
//! the approved version, processed through the existing ingestion service, and
//! published only in a final transaction that links the chunks and marks the
//! version ready. Failed jobs are retryable; stale approval, scope, fingerprint,
//! or removal checks fail closed.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, GenericClient};
use thiserror::Error;

use crate::config::Config;
use crate::corpus::Corpus;
use crate::document_lifecycle::{self, DocumentStorage};
use crate::embedding::Embedder;
use crate::fingerprint::ChunkerConfig;
use crate::pipeline::PipelineConfig;
use crate::service::IngestService;
use crate::storage_pg::PgStore;

/// One claimed processing job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimedJob {
    pub job_id: i64,
    pub version_id: i64,
    pub attempts: i32,
    pub max_attempts: i32,
    pub claimed_by: String,
}

/// The outcome of one processing attempt.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct JobOutcome {
    pub job_id: i64,
    pub version_id: i64,
    pub status: &'static str,
    pub error: Option<String>,
}

/// A safe processing error. No variant carries private document content.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProcessingError {
    #[error("the job or version no longer exists")]
    NotFound,
    #[error("the version is no longer approved for publication")]
    StaleApproval,
    #[error("the processing claim was lost to another worker")]
    ClaimLost,
    #[error("the document original is unavailable")]
    MissingOriginal,
    #[error("the document storage failed")]
    Storage,
    #[error("document processing failed: {0}")]
    Ingest(String),
    #[error("document processing database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for ProcessingError {
    fn from(err: postgres::Error) -> Self {
        ProcessingError::Database(err.to_string())
    }
}

impl From<document_lifecycle::LifecycleError> for ProcessingError {
    fn from(err: document_lifecycle::LifecycleError) -> Self {
        match err {
            document_lifecycle::LifecycleError::MissingFile => ProcessingError::MissingOriginal,
            document_lifecycle::LifecycleError::Storage => ProcessingError::Storage,
            other => ProcessingError::Database(other.to_string()),
        }
    }
}

fn audit<C: GenericClient>(
    client: &mut C,
    version_id: i64,
    job_id: i64,
    action: &str,
    actor: &str,
    detail: serde_json::Value,
) -> Result<(), ProcessingError> {
    let detail = detail.to_string();
    let action = action.to_string();
    let actor = actor.to_string();
    client.execute(
        "INSERT INTO document_audit_event (version_id, job_id, action, actor, detail)
         VALUES ($1, $2, $3, $4, $5::text::jsonb)",
        &[&version_id, &job_id, &action, &actor, &detail],
    )?;
    Ok(())
}

/// Claims the oldest runnable job. A queued job, or a `processing` job whose
/// lease expired, may be claimed. Concurrent workers cannot claim the same row.
pub fn claim_next_job(
    client: &mut Client,
    worker: &str,
    corpus: Option<&str>,
    lease_seconds: i64,
) -> Result<Option<ClaimedJob>, ProcessingError> {
    let mut tx = client.transaction()?;
    let worker_owned = worker.to_string();
    let corpus_owned = corpus.map(str::to_string);
    let lease = lease_seconds as f64;
    let row = tx.query_opt(
        "WITH candidate AS (
             SELECT job.job_id
               FROM document_processing_job job
               JOIN document_version version ON version.version_id = job.version_id
              WHERE (job.status = 'queued'
                     OR (job.status = 'processing'
                         AND (job.lease_expires_at IS NULL
                              OR job.lease_expires_at <= now())))
                AND job.attempts < job.max_attempts
                AND ($3::text IS NULL OR version.corpus = $3)
              ORDER BY job.job_id
              FOR UPDATE OF job SKIP LOCKED
              LIMIT 1
         )
         UPDATE document_processing_job job
            SET status = 'processing', claimed_by = $1, claimed_at = now(),
                lease_expires_at = now() + make_interval(secs => $2),
                attempts = job.attempts + 1, updated_at = now()
           FROM candidate
          WHERE job.job_id = candidate.job_id
         RETURNING job.job_id, job.version_id, job.attempts, job.max_attempts",
        &[&worker_owned, &lease, &corpus_owned],
    )?;
    let Some(row) = row else {
        tx.commit()?;
        return Ok(None);
    };
    let job = ClaimedJob {
        job_id: row.get(0),
        version_id: row.get(1),
        attempts: row.get(2),
        max_attempts: row.get(3),
        claimed_by: worker.to_string(),
    };
    tx.execute(
        "UPDATE document_version SET processing_status = 'processing', updated_at = now()
          WHERE version_id = $1",
        &[&job.version_id],
    )?;
    audit(
        &mut tx,
        job.version_id,
        job.job_id,
        "processing_started",
        worker,
        serde_json::json!({ "attempt": job.attempts }),
    )?;
    tx.commit()?;
    Ok(Some(job))
}

struct ProcessableVersion {
    version_id: i64,
    corpus: String,
    source: String,
    document_key: String,
    version_number: i32,
    origin: String,
    content_sha256: String,
    storage_key: Option<String>,
    original_filename: String,
}

fn load_processable(
    client: &mut Client,
    version_id: i64,
) -> Result<ProcessableVersion, ProcessingError> {
    let row = client
        .query_opt(
            "SELECT version_id, corpus, source, document_key, version_number, origin,
                    content_sha256, storage_key, original_filename, review_status,
                    (removed_at IS NOT NULL), approved_sha256, approved_scope,
                    access_scope, approved_version_number, processing_status
               FROM document_version WHERE version_id = $1 FOR UPDATE",
            &[&version_id],
        )?
        .ok_or(ProcessingError::NotFound)?;
    let review_status: String = row.get(9);
    let removed: bool = row.get(10);
    let content_sha256: String = row.get(6);
    let approved_sha256: Option<String> = row.get(11);
    let approved_scope: Option<String> = row.get(12);
    let access_scope: Option<String> = row.get(13);
    let approved_version_number: Option<i32> = row.get(14);
    let version_number: i32 = row.get(4);
    if review_status != "approved"
        || removed
        || approved_sha256.as_deref() != Some(content_sha256.as_str())
        || approved_scope != access_scope
        || approved_version_number != Some(version_number)
        || access_scope.as_deref() != Some("family")
    {
        return Err(ProcessingError::StaleApproval);
    }
    Ok(ProcessableVersion {
        version_id: row.get(0),
        corpus: row.get(1),
        source: row.get(2),
        document_key: row.get(3),
        version_number,
        origin: row.get(5),
        content_sha256,
        storage_key: row.get(7),
        original_filename: row.get(8),
    })
}

fn extension_for(filename: &str) -> &'static str {
    match filename
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .as_deref()
    {
        Some("pdf") => "pdf",
        Some("markdown") => "markdown",
        Some("txt") => "txt",
        _ => "md",
    }
}

fn temp_path(extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "tran-document-{}-{nanos}.{}",
        std::process::id(),
        extension
    ))
}

fn fail_job(
    client: &mut Client,
    job: &ClaimedJob,
    error: &str,
) -> Result<JobOutcome, ProcessingError> {
    let mut tx = client.transaction()?;
    let updated = tx.execute(
        "UPDATE document_processing_job
            SET status = 'failed', last_error = $2, lease_expires_at = NULL,
                updated_at = now(), finished_at = now()
          WHERE job_id = $1 AND status = 'processing' AND claimed_by = $3",
        &[&job.job_id, &error, &job.claimed_by],
    )?;
    if updated == 0 {
        tx.rollback()?;
        return Ok(JobOutcome {
            job_id: job.job_id,
            version_id: job.version_id,
            status: "stale",
            error: Some("claim lost".to_string()),
        });
    }
    tx.execute(
        "UPDATE document_version
            SET processing_status = 'failed', updated_at = now()
          WHERE version_id = $1 AND processing_status = 'processing'",
        &[&job.version_id],
    )?;
    audit(
        &mut tx,
        job.version_id,
        job.job_id,
        "processing_failed",
        "document-worker",
        serde_json::json!({ "error": error }),
    )?;
    tx.commit()?;
    Ok(JobOutcome {
        job_id: job.job_id,
        version_id: job.version_id,
        status: "failed",
        error: Some(error.to_string()),
    })
}

fn publish_ready<C: GenericClient>(
    client: &mut C,
    version: &ProcessableVersion,
    job: &ClaimedJob,
    actor: &str,
) -> Result<(), ProcessingError> {
    // Recheck the approval binding and removal state immediately before the
    // publication transaction. A stale job never publishes.
    let row = client
        .query_opt(
            "SELECT review_status, (removed_at IS NOT NULL), content_sha256,
                    approved_sha256, approved_scope, access_scope,
                    approved_version_number, version_number
               FROM document_version WHERE version_id = $1 FOR UPDATE",
            &[&version.version_id],
        )?
        .ok_or(ProcessingError::NotFound)?;
    let review: String = row.get(0);
    let removed: bool = row.get(1);
    let content: String = row.get(2);
    let approved: Option<String> = row.get(3);
    let approved_scope: Option<String> = row.get(4);
    let access_scope: Option<String> = row.get(5);
    let approved_version: Option<i32> = row.get(6);
    let version_number: i32 = row.get(7);
    if review != "approved"
        || removed
        || approved.as_deref() != Some(content.as_str())
        || approved_scope != access_scope
        || approved_version != Some(version_number)
        || access_scope.as_deref() != Some("family")
    {
        return Err(ProcessingError::StaleApproval);
    }
    client.execute(
        "UPDATE documents SET document_version_id = $1
          WHERE corpus = $2 AND source = $3 AND document_version_id IS NULL",
        &[&version.version_id, &version.corpus, &version.source],
    )?;
    // A newer ready version supersedes older versions for retrieval while
    // preserving their rows and chunks.
    client.execute(
        "UPDATE document_version
            SET removed_at = now(), removed_by = 'system:superseded', updated_at = now()
          WHERE corpus = $1 AND document_key = $2 AND version_number < $3
            AND removed_at IS NULL",
        &[
            &version.corpus,
            &version.document_key,
            &version.version_number,
        ],
    )?;
    client.execute(
        "UPDATE document_version
            SET processing_status = 'ready', updated_at = now()
          WHERE version_id = $1",
        &[&version.version_id],
    )?;
    // Publication is authorized by claim ownership, not by the lease clock: a
    // lease can expire while a long embedding run is still healthy, and
    // reclaiming the job changes `claimed_by`, so a stolen job is already
    // rejected here.
    let updated = client.execute(
        "UPDATE document_processing_job
            SET status = 'ready', updated_at = now(), finished_at = now(),
                last_error = NULL, lease_expires_at = NULL
          WHERE job_id = $1 AND status = 'processing' AND claimed_by = $2",
        &[&job.job_id, &job.claimed_by],
    )?;
    if updated == 0 {
        return Err(ProcessingError::ClaimLost);
    }
    audit(
        client,
        version.version_id,
        job.job_id,
        "processing_ready",
        actor,
        serde_json::json!({ "chunks_source": version.source }),
    )?;
    Ok(())
}

/// Processes one claimed job. The ingestion service writes chunks under the
/// version-specific source; publication is a separate final transaction.
pub fn process_job<E: Embedder>(
    client: &mut Client,
    store: &PgStore,
    embedder: &E,
    storage: &DocumentStorage,
    config: &Config,
    job: &ClaimedJob,
    worker: &str,
) -> Result<JobOutcome, ProcessingError> {
    let version = match load_processable(client, job.version_id) {
        Ok(version) => version,
        Err(err) => {
            let reason = match err {
                ProcessingError::StaleApproval => "stale approval",
                ProcessingError::NotFound => "version not found",
                _ => "version unavailable",
            };
            return fail_job(client, job, reason);
        }
    };

    if version.origin == "legacy_import" {
        let mut tx = client.transaction()?;
        tx.execute(
            "UPDATE document_version SET processing_status = 'ready', updated_at = now()
              WHERE version_id = $1",
            &[&version.version_id],
        )?;
        tx.execute(
            "UPDATE document_processing_job
                SET status = 'ready', updated_at = now(), finished_at = now()
              WHERE job_id = $1",
            &[&job.job_id],
        )?;
        audit(
            &mut tx,
            version.version_id,
            job.job_id,
            "processing_ready",
            worker,
            serde_json::json!({ "origin": "legacy_import" }),
        )?;
        tx.commit()?;
        return Ok(JobOutcome {
            job_id: job.job_id,
            version_id: version.version_id,
            status: "ready",
            error: None,
        });
    }

    let Some(storage_key) = version.storage_key.as_deref() else {
        return fail_job(client, job, "original unavailable");
    };
    let bytes = match storage.get(storage_key) {
        Ok(bytes) => bytes,
        Err(_) => return fail_job(client, job, "original unavailable"),
    };
    if document_lifecycle::sha256_hex(&bytes) != version.content_sha256 {
        return fail_job(client, job, "original fingerprint mismatch");
    }
    let path = temp_path(extension_for(&version.original_filename));
    if std::fs::write(&path, &bytes).is_err() {
        return fail_job(client, job, "staging failed");
    }
    let corpus = match Corpus::new(version.corpus.clone()) {
        Ok(corpus) => corpus,
        Err(_) => {
            let _ = std::fs::remove_file(&path);
            return fail_job(client, job, "invalid corpus");
        }
    };
    let pipeline_config = PipelineConfig::new(
        config.language.clone(),
        ChunkerConfig::new(config.chunk_size as usize, config.chunk_overlap as usize),
        config.embed_model.clone(),
        corpus,
    );
    let service = IngestService::new(embedder, store, store, pipeline_config);
    let outcome = service.ingest_source(&path, &version.source, &version.content_sha256);
    let _ = std::fs::remove_file(&path);
    match outcome {
        Ok(crate::pipeline::IngestOutcome::Replaced { .. })
        | Ok(crate::pipeline::IngestOutcome::Unchanged { .. }) => {
            let mut tx = client.transaction()?;
            match publish_ready(&mut tx, &version, job, worker) {
                Ok(()) => {
                    tx.commit()?;
                    Ok(JobOutcome {
                        job_id: job.job_id,
                        version_id: version.version_id,
                        status: "ready",
                        error: None,
                    })
                }
                Err(ProcessingError::ClaimLost) => {
                    tx.rollback()?;
                    Ok(JobOutcome {
                        job_id: job.job_id,
                        version_id: version.version_id,
                        status: "stale",
                        error: Some("claim lost".to_string()),
                    })
                }
                Err(ProcessingError::StaleApproval) => {
                    tx.rollback()?;
                    fail_job(client, job, "stale approval")
                }
                Err(_) => {
                    tx.rollback()?;
                    fail_job(client, job, "publication failed")
                }
            }
        }
        Ok(crate::pipeline::IngestOutcome::Rejected(_)) => {
            fail_job(client, job, "extraction rejected")
        }
        Err(err) => {
            eprintln!(
                "document processing failed for version {}: {err}",
                version.version_id
            );
            fail_job(client, job, "ingestion failed")
        }
    }
}

/// Runs up to `max_jobs` queued jobs. Returns one outcome per attempted job.
#[allow(clippy::too_many_arguments)]
pub fn run_due_jobs<E: Embedder>(
    client: &mut Client,
    store: &PgStore,
    embedder: &E,
    storage: &DocumentStorage,
    config: &Config,
    worker: &str,
    max_jobs: usize,
    corpus: Option<&str>,
    lease_seconds: i64,
) -> Result<Vec<JobOutcome>, ProcessingError> {
    let mut outcomes = Vec::new();
    for _ in 0..max_jobs {
        let Some(job) = claim_next_job(client, worker, corpus, lease_seconds)? else {
            break;
        };
        outcomes.push(process_job(
            client, store, embedder, storage, config, &job, worker,
        )?);
    }
    Ok(outcomes)
}

/// Resets a failed job to queued. The retry is explicit and audited.
pub fn retry_job(
    client: &mut Client,
    job_id: i64,
    actor: &str,
    corpus: &str,
) -> Result<ClaimedJob, ProcessingError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT job.job_id, job.version_id, job.attempts, job.max_attempts
               FROM document_processing_job job
               JOIN document_version version ON version.version_id = job.version_id
              WHERE job.job_id = $1 AND job.status = 'failed' AND version.corpus = $2
              FOR UPDATE OF job",
            &[&job_id, &corpus],
        )?
        .ok_or(ProcessingError::NotFound)?;
    let job = ClaimedJob {
        job_id: row.get(0),
        version_id: row.get(1),
        attempts: row.get(2),
        max_attempts: row.get(3),
        claimed_by: actor.to_string(),
    };
    tx.execute(
        "UPDATE document_processing_job
            SET status = 'queued', max_attempts = GREATEST(max_attempts, attempts + 1),
                last_error = NULL, claimed_by = NULL, claimed_at = NULL,
                lease_expires_at = NULL, updated_at = now(), finished_at = NULL
          WHERE job_id = $1",
        &[&job_id],
    )?;
    tx.execute(
        "UPDATE document_version
            SET processing_status = 'queued', updated_at = now()
          WHERE version_id = $1 AND removed_at IS NULL",
        &[&job.version_id],
    )?;
    audit(
        &mut tx,
        job.version_id,
        job.job_id,
        "retry",
        actor,
        serde_json::json!({ "attempts": job.attempts }),
    )?;
    tx.commit()?;
    Ok(job)
}

/// Lists recent jobs for the admin status surface.
#[derive(Debug, Clone, serde::Serialize)]
pub struct JobRecord {
    pub job_id: i64,
    pub version_id: i64,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub fn list_jobs(client: &mut Client, corpus: &str) -> Result<Vec<JobRecord>, ProcessingError> {
    let rows = client.query(
        "SELECT job.job_id, job.version_id, job.status, job.attempts, job.max_attempts,
                job.last_error, job.created_at::text, job.updated_at::text
           FROM document_processing_job job
           JOIN document_version version ON version.version_id = job.version_id
          WHERE version.corpus = $1
          ORDER BY job.job_id DESC
          LIMIT 200",
        &[&corpus],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| JobRecord {
            job_id: row.get(0),
            version_id: row.get(1),
            status: row.get(2),
            attempts: row.get(3),
            max_attempts: row.get(4),
            last_error: row.get(5),
            created_at: row.get(6),
            updated_at: row.get(7),
        })
        .collect())
}
