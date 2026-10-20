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

/// One imported person identity. Upserted in the same transaction as the
/// assertions so a failed import leaves no partial graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPerson {
    /// The stable catalog person key.
    pub person_key: String,
    /// The display name to store.
    pub display_name: String,
}

/// One imported document with its assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDocument {
    /// The relative path (the document key).
    pub relative_path: String,
    /// The document sha256.
    pub sha256: String,
    /// The persons referenced by the assertions.
    pub persons: Vec<ImportPerson>,
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
    let document_id: i64 = tx
        .query_one(
            "INSERT INTO source_document (relative_path, sha256)
             VALUES ($1, $2)
             ON CONFLICT (relative_path) DO UPDATE SET sha256 = EXCLUDED.sha256
             RETURNING source_document_id",
            &[&document.relative_path, &document.sha256],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?
        .get(0);

    // Persons are upserted inside this transaction. If any later assertion
    // fails, the transaction rollback also removes these persons.
    for person in &document.persons {
        tx.execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)
             ON CONFLICT (person_key) DO UPDATE SET display_name = EXCLUDED.display_name",
            &[&person.person_key, &person.display_name],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    }

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
                sync_evidence(tx, id, document_id, document, assertion)?;
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
                sync_evidence(tx, id, document_id, document, assertion)?;
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
    document_id: i64,
    document: &ImportDocument,
    assertion: &ImportAssertion,
) -> Result<(), GenealogyError> {
    if assertion.evidence.is_empty() {
        return Ok(());
    }
    for (document_ref, page) in &assertion.evidence {
        let _ = document;
        let source_evidence_id = source_evidence_id(tx, document_id, *page)?;
        tx.execute(
            "INSERT INTO assertion_evidence
                (assertion_id, source_evidence_id, document_ref, page)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (assertion_id, document_ref, page) DO UPDATE
                SET source_evidence_id = EXCLUDED.source_evidence_id",
            &[&assertion_id, &source_evidence_id, document_ref, page],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
    }
    Ok(())
}

fn source_evidence_id(
    tx: &mut Transaction<'_>,
    document_id: i64,
    page: i32,
) -> Result<i64, GenealogyError> {
    if let Some(row) = tx
        .query_opt(
            "SELECT source_evidence_id FROM source_evidence
              WHERE source_document_id = $1 AND page = $2
                AND (locator = 'page' OR locator IS NULL)
              ORDER BY source_evidence_id LIMIT 1",
            &[&document_id, &page],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?
    {
        let id: i64 = row.get(0);
        tx.execute(
            "UPDATE source_evidence SET locator = 'page' WHERE source_evidence_id = $1",
            &[&id],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?;
        return Ok(id);
    }

    Ok(tx
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page, locator)
             VALUES ($1, $2, 'page')
             RETURNING source_evidence_id",
            &[&document_id, &page],
        )
        .map_err(|err| GenealogyError::Query(err.to_string()))?
        .get(0))
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
