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
//! appended. Evidence rows that disappeared from the source are deleted, so a
//! removed citation reopens the assertion instead of leaving stale provenance
//! behind. A second run with identical input makes no status or event change.

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
    /// The resolved target person key, when the relationship was unambiguous.
    /// `None` keeps the statement unresolved and visible for review.
    pub object_key: Option<String>,
    /// The evidence rows (document_ref, page).
    pub evidence: Vec<(String, i32)>,
}

/// One resolved relationship edge. Only statements whose target names exactly one
/// catalog person reach this list; the planner never guesses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLink {
    /// `parent_child` or `spouse`.
    pub kind: String,
    /// The parent (`parent_child`) or the key-ordered first spouse.
    pub from_key: String,
    /// The child (`parent_child`) or the key-ordered second spouse.
    pub to_key: String,
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
    /// The source-document kind, one of `original`, `transcription`,
    /// `translation`, `catalog`, or `derivative`.
    pub source_kind: String,
    /// The persons referenced by the assertions.
    pub persons: Vec<ImportPerson>,
    /// The assertions to import.
    pub assertions: Vec<ImportAssertion>,
    /// The resolved relationship edges to import. Absent edges stay unresolved.
    pub links: Vec<ImportLink>,
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
    /// Relationship edges inserted for the first time.
    pub links_inserted: usize,
    /// Relationship edges that already existed.
    pub links_unchanged: usize,
}

/// Imports one document and its assertions idempotently inside one transaction.
pub fn reimport_document(
    conn: &mut postgres::Client,
    document: &ImportDocument,
) -> Result<ReimportReport, GenealogyError> {
    let mut tx = conn.transaction().map_err(GenealogyError::Query)?;
    let report = reimport_in_transaction(&mut tx, document)?;
    tx.commit().map_err(GenealogyError::Query)?;
    Ok(report)
}

fn reimport_in_transaction(
    tx: &mut Transaction<'_>,
    document: &ImportDocument,
) -> Result<ReimportReport, GenealogyError> {
    let document_id: i64 = tx
        .query_one(
            "INSERT INTO source_document (relative_path, sha256, source_kind)
             VALUES ($1, $2, $3)
             ON CONFLICT (relative_path) DO UPDATE
                SET sha256 = EXCLUDED.sha256, source_kind = EXCLUDED.source_kind
             RETURNING source_document_id",
            &[
                &document.relative_path,
                &document.sha256,
                &document.source_kind,
            ],
        )
        .map_err(GenealogyError::Query)?
        .get(0);

    // Persons are upserted inside this transaction. If any later assertion
    // fails, the transaction rollback also removes these persons.
    for person in &document.persons {
        tx.execute(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)
             ON CONFLICT (person_key) DO UPDATE SET display_name = EXCLUDED.display_name",
            &[&person.person_key, &person.display_name],
        )
        .map_err(GenealogyError::Query)?;
    }

    let mut report = ReimportReport::default();
    for assertion in &document.assertions {
        let person_id: i64 = tx
            .query_one(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&assertion.person_key],
            )
            .map_err(GenealogyError::Query)?
            .get(0);

        let existing: Option<(i64, String)> = tx
            .query_opt(
                "SELECT assertion_id, review_status FROM assertion
                  WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3",
                &[&person_id, &assertion.kind, &assertion.key],
            )
            .map_err(GenealogyError::Query)?
            .map(|r| (r.get(0), r.get(1)));

        let assertion_id = match existing {
            Some((id, status)) => {
                // Capture the baseline before syncing: an evidence INSERT or
                // DELETE fires the reopen trigger, which removes the baseline.
                let was_verified = status == ReviewStatus::Verified.as_str();
                let baseline = if was_verified {
                    review_baseline(tx, id)?
                } else {
                    None
                };

                // Re-import never rewrites reviewer history here; it only
                // reconciles evidence and lets a change reopen the assertion.
                sync_evidence(tx, id, document_id, assertion)?;
                if reopen_if_evidence_changed(tx, id, was_verified, baseline.as_deref())? {
                    report.reopened += 1;
                } else {
                    report.unchanged += 1;
                }
                // Fill a missing object binding for a non-verified assertion. A
                // verified assertion is left exactly as the reviewer decided.
                if let Some(object_key) = assertion.object_key.as_deref() {
                    tx.execute(
                        "UPDATE assertion
                            SET object_person_id = (
                                SELECT person_id FROM person WHERE person_key = $2
                            )
                          WHERE assertion_id = $1
                            AND object_person_id IS NULL
                            AND review_status <> 'verified'",
                        &[&id, &object_key],
                    )
                    .map_err(GenealogyError::Query)?;
                }
                id
            }
            None => {
                // New candidates are always unverified and have no events.
                let object_person_id: Option<i64> = match assertion.object_key.as_deref() {
                    Some(key) => tx
                        .query_opt(
                            "SELECT person_id FROM person WHERE person_key = $1",
                            &[&key],
                        )
                        .map_err(GenealogyError::Query)?
                        .map(|row| row.get(0)),
                    None => None,
                };
                let id: i64 = tx
                    .query_one(
                        "INSERT INTO assertion
                            (person_id, assertion_kind, assertion_key, value_text,
                             object_person_id, review_status)
                         VALUES ($1, $2, $3, $4, $5, 'unverified')
                         RETURNING assertion_id",
                        &[
                            &person_id,
                            &assertion.kind,
                            &assertion.key,
                            &assertion.value_text,
                            &object_person_id,
                        ],
                    )
                    .map_err(GenealogyError::Query)?
                    .get(0);
                sync_evidence(tx, id, document_id, assertion)?;
                report.inserted += 1;
                id
            }
        };
        let _ = assertion_id;
    }

    // Resolved edges are inserted last, after every person exists. Both inserts
    // are idempotent, so a re-import of unchanged input never duplicates an edge
    // and never discards a reviewed assertion. A spouse edge is matched in
    // either orientation, so an existing reversed row is reused rather than
    // duplicated.
    for link in &document.links {
        let inserted = match link.kind.as_str() {
            "parent_child" => tx
                .execute(
                    "INSERT INTO parent_child_link (parent_id, child_id)
                     SELECT parent.person_id, child.person_id
                       FROM person parent, person child
                      WHERE parent.person_key = $1 AND child.person_key = $2
                     ON CONFLICT (parent_id, child_id) DO NOTHING",
                    &[&link.from_key, &link.to_key],
                )
                .map_err(GenealogyError::Query)?,
            "spouse" => tx
                .execute(
                    "INSERT INTO spouse_link (person_id, spouse_id, source_order)
                     SELECT first.person_id, second.person_id, 1
                       FROM person first, person second
                      WHERE first.person_key = $1 AND second.person_key = $2
                        AND NOT EXISTS (
                          SELECT 1 FROM spouse_link existing
                           WHERE (existing.person_id = first.person_id
                                  AND existing.spouse_id = second.person_id)
                              OR (existing.person_id = second.person_id
                                  AND existing.spouse_id = first.person_id)
                        )
                     ON CONFLICT (person_id, spouse_id, source_order) DO NOTHING",
                    &[&link.from_key, &link.to_key],
                )
                .map_err(GenealogyError::Query)?,
            // `plan_links` only emits the two kinds above; an unknown kind is a
            // programming error, so it is not counted as an unchanged edge.
            _ => continue,
        };
        if inserted > 0 {
            report.links_inserted += 1;
        } else {
            report.links_unchanged += 1;
        }
    }
    Ok(report)
}

fn sync_evidence(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
    document_id: i64,
    assertion: &ImportAssertion,
) -> Result<(), GenealogyError> {
    let document_refs: Vec<String> = assertion
        .evidence
        .iter()
        .map(|(document_ref, _)| document_ref.clone())
        .collect();
    let pages: Vec<i32> = assertion.evidence.iter().map(|(_, page)| *page).collect();

    // Reconcile only the rows that point at this source document, so evidence
    // supplied by another document is preserved. A removed citation must not
    // leave the assertion permanently verified against stale provenance.
    tx.execute(
        "DELETE FROM assertion_evidence ae
          WHERE ae.assertion_id = $1
            AND ae.source_evidence_id IN (
                SELECT source_evidence_id
                  FROM source_evidence
                 WHERE source_document_id = $2
            )
            AND NOT EXISTS (
                SELECT 1
                  FROM unnest($3::text[], $4::int[]) AS keep(document_ref, page)
                 WHERE keep.document_ref = ae.document_ref
                   AND keep.page = ae.page
            )",
        &[&assertion_id, &document_id, &document_refs, &pages],
    )
    .map_err(GenealogyError::Query)?;

    for (document_ref, page) in &assertion.evidence {
        let source_evidence_id = source_evidence_id(tx, document_id, *page)?;
        tx.execute(
            "INSERT INTO assertion_evidence
                (assertion_id, source_evidence_id, document_ref, page)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (assertion_id, document_ref, page) DO UPDATE
                SET source_evidence_id = EXCLUDED.source_evidence_id",
            &[&assertion_id, &source_evidence_id, document_ref, page],
        )
        .map_err(GenealogyError::Query)?;
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
        .map_err(GenealogyError::Query)?
    {
        let id: i64 = row.get(0);
        tx.execute(
            "UPDATE source_evidence SET locator = 'page' WHERE source_evidence_id = $1",
            &[&id],
        )
        .map_err(GenealogyError::Query)?;
        return Ok(id);
    }

    Ok(tx
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page, locator)
             VALUES ($1, $2, 'page')
             RETURNING source_evidence_id",
            &[&document_id, &page],
        )
        .map_err(GenealogyError::Query)?
        .get(0))
}

fn review_baseline(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
) -> Result<Option<String>, GenealogyError> {
    tx.query_opt(
        "SELECT evidence_fingerprint FROM assertion_review_baseline WHERE assertion_id = $1",
        &[&assertion_id],
    )
    .map_err(GenealogyError::Query)
    .map(|row| row.map(|r| r.get(0)))
}

fn current_status(tx: &mut Transaction<'_>, assertion_id: i64) -> Result<String, GenealogyError> {
    tx.query_one(
        "SELECT review_status FROM assertion WHERE assertion_id = $1",
        &[&assertion_id],
    )
    .map_err(GenealogyError::Query)
    .map(|row| row.get(0))
}

/// Reports whether a verified assertion must reopen after an evidence sync.
///
/// The evidence trigger is the authority: it flips the status and drops the
/// baseline on every INSERT or DELETE. A changed source hash, however, changes
/// the fingerprint without touching an evidence row, so the baseline comparison
/// covers that path as well.
fn should_reopen(
    was_verified: bool,
    baseline: Option<&str>,
    current_fingerprint: &str,
    current_status: &str,
) -> bool {
    was_verified
        && (current_status != ReviewStatus::Verified.as_str()
            || baseline.is_some_and(|expected| expected != current_fingerprint))
}

fn reopen_if_evidence_changed(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
    was_verified: bool,
    baseline: Option<&str>,
) -> Result<bool, GenealogyError> {
    if !was_verified {
        return Ok(false);
    }
    let current = evidence_fingerprint(tx, assertion_id)
        .map_err(|err| GenealogyError::Review(format!("evidence fingerprint: {err}")))?;
    let status_after = current_status(tx, assertion_id)?;
    if !should_reopen(was_verified, baseline, &current, &status_after) {
        return Ok(false);
    }

    // The trigger already moved the status and removed the baseline when an
    // evidence row changed. Re-asserting the post-state is idempotent and also
    // covers the fingerprint-only path (a changed source hash with identical
    // evidence), where the trigger never fired.
    tx.execute(
        "UPDATE assertion SET review_status = 'needs_review'
          WHERE assertion_id = $1 AND review_status = 'verified'",
        &[&assertion_id],
    )
    .map_err(GenealogyError::Query)?;
    tx.execute(
        "DELETE FROM assertion_review_baseline WHERE assertion_id = $1",
        &[&assertion_id],
    )
    .map_err(GenealogyError::Query)?;

    let evidence_ref = baseline.unwrap_or(&current);
    tx.execute(
        "INSERT INTO assertion_review_event
            (assertion_id, reviewer, from_status, to_status, evidence_ref, note)
         VALUES ($1, 'reimport', 'verified', 'needs_review', $2, 'evidence_changed')",
        &[&assertion_id, &evidence_ref],
    )
    .map_err(GenealogyError::Query)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::should_reopen;

    const VERIFIED: &str = "verified";
    const NEEDS_REVIEW: &str = "needs_review";

    #[test]
    fn unchanged_evidence_and_a_stable_hash_do_not_reopen() {
        assert!(!should_reopen(true, Some("fp"), "fp", VERIFIED));
    }

    #[test]
    fn a_changed_fingerprint_reopens_a_still_verified_assertion() {
        assert!(should_reopen(true, Some("old"), "new", VERIFIED));
    }

    #[test]
    fn a_trigger_reopen_counts_even_when_the_fingerprint_is_stable() {
        assert!(should_reopen(true, Some("fp"), "fp", NEEDS_REVIEW));
    }

    #[test]
    fn an_assertion_that_was_not_verified_is_never_counted_as_reopened() {
        assert!(!should_reopen(false, Some("old"), "new", NEEDS_REVIEW));
    }

    #[test]
    fn a_verified_assertion_without_a_baseline_does_not_reopen_on_its_own() {
        assert!(!should_reopen(true, None, "fp", VERIFIED));
    }
}
