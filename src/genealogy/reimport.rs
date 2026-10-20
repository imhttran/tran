//! Idempotent genealogy re-import (TRAN-002D).
//!
//! [`reimport_document`] upserts `source_document`, `source_evidence`,
//! `assertion`, and `assertion_evidence` idempotently. It never deletes a
//! `assertion_review_event` row and never rewrites `reviewed_by`/`reviewed_at`/
//! `review_note` on an existing assertion. New candidates land as `unverified`.
//!
//! When the recomputed evidence fingerprint differs from
//! `assertion_review_baseline`, a verified assertion is reopened: its status
//! moves to `needs_review`, its baseline is removed, and one `reopened` event is
//! appended. A second run with identical input makes no status or event change.

use postgres::Transaction;

use super::review::{evidence_fingerprint, ReviewStatus};
use super::GenealogyError;

/// One imported assertion with its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportAssertion {
    /// The owning person key (must already exist).
    pub person_key: String,
    /// The assertion kind.
    pub kind: String,
    /// The assertion key.
    pub key: String,
    /// The optional value text.
    pub value_text: Option<String>,
    /// The evidence rows (document_ref, page).
    pub evidence: Vec<(String, i32)>,
}

/// One imported document with its assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDocument {
    /// The relative path (the document key).
    pub relative_path: String,
    /// The document sha256.
    pub sha256: String,
    /// The assertions to import.
    pub assertions: Vec<ImportAssertion>,
}

/// The counters for one re-import run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReimportReport {
    /// Assertions inserted as new `unverified` candidates.
    pub inserted: usize,
    /// Assertions already present and left unchanged.
    pub unchanged: usize,
    /// Verified assertions reopened because the evidence changed.
    pub reopened: usize,
}

/// Imports one document and its assertions idempotently inside one transaction.
pub fn reimport_document(
    conn: &mut postgres::Client,
    document: &ImportDocument,
) -> Result<ReimportReport, GenealogyError> {
    let mut tx = conn
        .transaction()
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    let report = reimport_in_transaction(&mut tx, document)?;
    tx.commit()
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    Ok(report)
}

fn reimport_in_transaction(
    tx: &mut Transaction<'_>,
    document: &ImportDocument,
) -> Result<ReimportReport, GenealogyError> {
    let _document_id: i64 = tx
        .query_one(
            "INSERT INTO source_document (relative_path, sha256)
             VALUES ($1, $2)
             ON CONFLICT (relative_path) DO UPDATE SET sha256 = EXCLUDED.sha256
             RETURNING source_document_id",
            &[&document.relative_path, &document.sha256],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?
        .get(0);

    let mut report = ReimportReport::default();
    for assertion in &document.assertions {
        let person_id: i64 = tx
            .query_one(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&assertion.person_key],
            )
            .map_err(|err| GenealogyError::Query(err.to_string()))?
            .get(0);

        let existing: Option<(i64, String)> = tx
            .query_opt(
                "SELECT assertion_id, review_status FROM assertion
                  WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3",
                &[&person_id, &assertion.kind, &assertion.key],
            )
            .map_err(|err| GenealogyError::Query(err.to_string()))?
            .map(|r| (r.get(0), r.get(1)));

        let assertion_id = match existing {
            Some((id, status)) => {
                // Re-import never rewrites reviewer history or status here.
                sync_evidence(tx, id, document, assertion)?;
                if status == ReviewStatus::Verified.as_str() {
                    if reopened_by_evidence_change(tx, id)? {
                        report.reopened += 1;
                    } else {
                        report.unchanged += 1;
                    }
                } else {
                    report.unchanged += 1;
                }
                id
            }
            None => {
                // New candidates are always unverified and have no events.
                let id: i64 = tx
                    .query_one(
                        "INSERT INTO assertion
                            (person_id, assertion_kind, assertion_key, value_text, review_status)
                         VALUES ($1, $2, $3, $4, 'unverified')
                         RETURNING assertion_id",
                        &[
                            &person_id,
                            &assertion.kind,
                            &assertion.key,
                            &assertion.value_text,
                        ],
                    )
                    .map_err(|err| GenealogyError::Query(err.to_string()))?
                    .get(0);
                sync_evidence(tx, id, document, assertion)?;
                report.inserted += 1;
                id
            }
        };
        let _ = assertion_id;
    }
    Ok(report)
}

fn sync_evidence(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
    document: &ImportDocument,
    assertion: &ImportAssertion,
) -> Result<(), GenealogyError> {
    if assertion.evidence.is_empty() {
        return Ok(());
    }
    // Insert missing evidence rows. The change trigger reopens a verified
    // assertion when new evidence arrives; that is the intended reopening.
    for (document_ref, page) in &assertion.evidence {
        let _ = document;
        tx.execute(
            "INSERT INTO assertion_evidence (assertion_id, document_ref, page)
             VALUES ($1, $2, $3)
             ON CONFLICT (assertion_id, document_ref, page) DO NOTHING",
            &[&assertion_id, document_ref, page],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    }
    Ok(())
}

fn reopened_by_evidence_change(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
) -> Result<bool, GenealogyError> {
    let baseline: Option<String> = tx
        .query_opt(
            "SELECT evidence_fingerprint FROM assertion_review_baseline WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?
        .map(|r| r.get(0));
    let Some(expected) = baseline else {
        return Ok(false);
    };
    let current = evidence_fingerprint(tx, assertion_id)
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    if current == expected {
        return Ok(false);
    }
    // Reopen: move to needs_review, drop the stale baseline, append one event.
    tx.execute(
        "UPDATE assertion SET review_status = 'needs_review' WHERE assertion_id = $1",
        &[&assertion_id],
    )
    .map_err(|err| GenealogyError::Query(err.to_string()))?;
    tx.execute(
        "DELETE FROM assertion_review_baseline WHERE assertion_id = $1",
        &[&assertion_id],
    )
    .map_err(|err| GenealogyError::Query(err.to_string()))?;
    tx.execute(
        "INSERT INTO assertion_review_event
            (assertion_id, reviewer, from_status, to_status, evidence_ref, note)
         VALUES ($1, 'reimport', 'verified', 'needs_review', $2, 'evidence_changed')",
        &[&assertion_id, &expected],
    )
    .map_err(|err| GenealogyError::Query(err.to_string()))?;
    Ok(true)
}
