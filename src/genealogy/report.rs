//! Local unresolved-assertion report (TRAN-002D).
//!
//! [`write_unresolved_report`] writes a JSON report of every assertion whose
//! status is `unverified`, `needs_review`, `disputed`, or `rejected` to a
//! caller-supplied path. Each entry lists the assertion ID, person ID, kind,
//! status, last reviewer, last timestamp, and the blocking violations.
//!
//! The report has no bulk approval field or command. Use
//! [`unresolved_assertion_ids`] to list concrete assertion IDs, so a reviewer
//! can choose one ID for the safe single-ID review operation.
//!
//! Report path convention: the caller passes an absolute or relative path. The
//! writer writes to a sibling temp file and renames it over the target, so a
//! second write replaces the file atomically and always leaves valid JSON.

use std::io::Write;
use std::path::Path;

use postgres::{Client, Transaction};
use serde::Serialize;

use super::validation;
use super::GenealogyError;

/// One unresolved assertion entry.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UnresolvedEntry {
    /// The assertion ID the reviewer must choose.
    pub assertion_id: i64,
    /// The owning person.
    pub person_id: i64,
    /// The assertion kind.
    pub kind: String,
    /// The assertion key.
    pub key: String,
    /// The current review status.
    pub status: String,
    /// The reviewer from the latest event, if any.
    pub last_reviewer: Option<String>,
    /// The timestamp from the latest event, if any (as text).
    pub last_timestamp: Option<String>,
    /// The blocking violations, as text.
    pub violations: Vec<String>,
}

/// The unresolved-assertion report.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct UnresolvedReport {
    /// Every unresolved assertion.
    pub entries: Vec<UnresolvedEntry>,
}

impl UnresolvedReport {
    /// The concrete assertion IDs a reviewer can pick from.
    pub fn assertion_ids(&self) -> Vec<i64> {
        self.entries.iter().map(|e| e.assertion_id).collect()
    }
}

/// Lists the concrete unresolved assertion IDs, in ascending order.
pub fn unresolved_assertion_ids(conn: &mut Client) -> Result<Vec<i64>, GenealogyError> {
    let rows = conn
        .query(
            "SELECT assertion_id FROM assertion
              WHERE review_status IN ('unverified', 'needs_review', 'disputed', 'rejected')
              ORDER BY assertion_id",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows.iter().map(|r| r.get(0)).collect())
}

/// Builds the unresolved report from the database.
pub fn build_unresolved_report(conn: &mut Client) -> Result<UnresolvedReport, GenealogyError> {
    let rows = conn
        .query(
            "SELECT a.assertion_id, a.person_id, a.assertion_kind, a.assertion_key, a.review_status,
                    e.reviewer, e.event_at::text
               FROM assertion a
               LEFT JOIN LATERAL (
                   SELECT reviewer, event_at FROM assertion_review_event
                    WHERE assertion_id = a.assertion_id
                    ORDER BY assertion_review_event_id DESC LIMIT 1
               ) e ON true
              WHERE a.review_status IN ('unverified', 'needs_review', 'disputed', 'rejected')
              ORDER BY a.assertion_id",
            &[],
        )
        .map_err(GenealogyError::Query)?;

    let mut tx: Transaction<'_> = conn.transaction().map_err(GenealogyError::Query)?;

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let assertion_id: i64 = row.try_get(0)?;
        let person_id: i64 = row.try_get(1)?;
        let kind: String = row.try_get(2)?;
        let key: String = row.try_get(3)?;
        let status: String = row.try_get(4)?;
        let last_reviewer: Option<String> = row.try_get(5)?;
        let last_timestamp: Option<String> = row.try_get(6)?;
        // One transaction for the whole report: validating each assertion in
        // its own transaction made a report O(N) transactions.
        let violations: Vec<String> = validation::validate_candidate(&mut tx, assertion_id)?
            .iter()
            .map(|violation| violation.to_string())
            .collect();
        entries.push(UnresolvedEntry {
            assertion_id,
            person_id,
            kind,
            key,
            status,
            last_reviewer,
            last_timestamp,
            violations,
        });
    }
    tx.commit().map_err(GenealogyError::Query)?;
    Ok(UnresolvedReport { entries })
}

/// Serializes and writes the report to `path` atomically.
pub fn write_unresolved_report(
    conn: &mut Client,
    path: &Path,
) -> Result<UnresolvedReport, GenealogyError> {
    let report = build_unresolved_report(conn)?;
    let json = serde_json::to_string_pretty(&report)?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let temp = path.with_extension("json.tmp");
    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(json.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path)?;
    Ok(report)
}
