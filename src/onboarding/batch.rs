//! Batch preparation (BULK-003): resumable, idempotent preparation of a
//! reviewed set of living candidates.
//!
//! Preparation reserves one username per person and records one durable batch
//! item per `(batch, person)` pair. It creates no OIDC credential, grants no role
//! or corpus access, sends no email, and never spans an external provider call:
//! every database mutation commits per person inside this module.
//!
//! Idempotency comes from two stable keys plus unique constraints in migration
//! `0018_onboarding_batch.sql`:
//!
//!  * `batch_idempotency_key` = `batch:<actor>:<confirmed_count>:<sha256(sorted person ids[, overrides])>`
//!  * `person_idempotency_key` = `<batch_idempotency_key>:<person_id>`
//!
//! A duplicate submission collapses to the existing batch row. A restart skips
//! terminal items and re-checks the first non-terminal item. A partial unique
//! index allows one `prepared` item per person across every batch, so a later
//! batch reselecting the same person records `skipped_already_prepared` instead
//! of creating a second logical preparation.

use std::collections::HashMap;

use postgres::Client;
use thiserror::Error;

use crate::onboarding::candidate::{
    load_name_parts, reclassify_person, CandidateError, Classification, Reason,
};
use crate::onboarding::reservation::{reserve, ReservationError};

/// The bounded chunk size used by [`run_batch`].
pub const CHUNK_SIZE: usize = 25;

/// The status of one batch item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    /// Recorded but not yet processed.
    Pending,
    /// Prepared: username reserved and no blockers.
    Prepared,
    /// Skipped because the person is already linked.
    SkippedAlreadyLinked,
    /// Skipped because an invitation is already pending.
    SkippedAlreadyInvited,
    /// Skipped because another batch already prepared the person.
    SkippedAlreadyPrepared,
    /// Skipped because the person is not eligible.
    SkippedNotEligible,
    /// Skipped because the same person appeared twice in the selection.
    SkippedDuplicate,
    /// Needs human review before it can be prepared.
    NeedsReview,
    /// The per-person step failed; the row records a safe reason.
    Failed,
}

impl ItemStatus {
    /// The stable database string.
    pub fn as_str(self) -> &'static str {
        match self {
            ItemStatus::Pending => "pending",
            ItemStatus::Prepared => "prepared",
            ItemStatus::SkippedAlreadyLinked => "skipped_already_linked",
            ItemStatus::SkippedAlreadyInvited => "skipped_already_invited",
            ItemStatus::SkippedAlreadyPrepared => "skipped_already_prepared",
            ItemStatus::SkippedNotEligible => "skipped_not_eligible",
            ItemStatus::SkippedDuplicate => "skipped_duplicate",
            ItemStatus::NeedsReview => "needs_review",
            ItemStatus::Failed => "failed",
        }
    }

    /// True when the item is terminal and a restart must skip it.
    pub fn is_terminal(self) -> bool {
        !matches!(self, ItemStatus::Pending)
    }

    /// Maps a status string from the database.
    pub fn from_db(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => ItemStatus::Pending,
            "prepared" => ItemStatus::Prepared,
            "skipped_already_linked" => ItemStatus::SkippedAlreadyLinked,
            "skipped_already_invited" => ItemStatus::SkippedAlreadyInvited,
            "skipped_already_prepared" => ItemStatus::SkippedAlreadyPrepared,
            "skipped_not_eligible" => ItemStatus::SkippedNotEligible,
            "skipped_duplicate" => ItemStatus::SkippedDuplicate,
            "needs_review" => ItemStatus::NeedsReview,
            "failed" => ItemStatus::Failed,
            _ => return None,
        })
    }
}

/// Errors raised by batch preparation.
///
/// The messages carry rule names and opaque ids only. They never carry a display
/// name, a date, a source excerpt, a credential, or a token.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BatchError {
    /// The confirmation count does not match the distinct selected count.
    #[error("confirmed count {confirmed} does not match selected count {selected}")]
    CountMismatch { confirmed: usize, selected: usize },
    /// The selection is empty.
    #[error("selection is empty")]
    EmptySelection,
    /// A living-evidence re-evaluation failed.
    #[error("evidence re-evaluation failed for person {person_id}: {reason}")]
    Evidence { person_id: i64, reason: String },
    /// The database operation failed.
    #[error("batch database operation failed: {0}")]
    Database(String),
    /// A reservation step failed.
    #[error("reservation step failed: {0}")]
    Reservation(String),
}

impl From<CandidateError> for BatchError {
    fn from(err: CandidateError) -> Self {
        BatchError::Database(err.to_string())
    }
}

impl From<ReservationError> for BatchError {
    fn from(err: ReservationError) -> Self {
        BatchError::Reservation(err.to_string())
    }
}

/// Stable inputs that make duplicate submissions collapse.
///
/// Both keys are derived only from public inputs. They never include a name,
/// a date, an identity fingerprint, or a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchKeys {
    /// One key per confirmed request.
    pub batch_key: String,
}

impl BatchKeys {
    /// Builds the batch key from the actor, the confirmed count, the
    /// deduplicated, sorted person ids, and any username overrides.
    ///
    /// Overrides are part of the identity so a corrected override is a new
    /// request instead of replaying the old batch's terminal failure. With no
    /// overrides the key is unchanged from before overrides existed.
    pub fn derive(
        actor: &str,
        confirmed_count: usize,
        person_ids: &[i64],
        overrides: &HashMap<i64, String>,
    ) -> Self {
        let mut sorted: Vec<i64> = person_ids.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        let mut joined = sorted
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        if !overrides.is_empty() {
            let mut pairs: Vec<(&i64, &String)> = overrides.iter().collect();
            pairs.sort_unstable();
            // JSON keeps an override containing `,` or `=` unambiguous.
            joined.push('|');
            joined.push_str(&serde_json::to_string(&pairs).unwrap_or_default());
        }
        let digest = sha256_hex(joined.as_bytes());
        BatchKeys {
            batch_key: format!("batch:{}:{}:{}", actor.trim(), confirmed_count, digest),
        }
    }

    /// Builds the stable per-person key inside this batch.
    pub fn person_key(&self, person_id: i64) -> String {
        format!("{}:{}", self.batch_key, person_id)
    }
}

/// SHA-256 of opaque ids only, never personal data.
fn sha256_hex(input: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, input);
    digest
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// One recorded batch item, returned to the caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    /// The item primary key.
    pub batch_item_id: i64,
    /// The person the item belongs to.
    pub person_id: i64,
    /// The terminal status after preparation.
    pub status: ItemStatus,
    /// The privacy-safe reason code, when one applies.
    pub reason_code: Option<String>,
    /// The username reservation id, when one was created.
    pub reservation_id: Option<i64>,
}

/// One batch item as the admin status view shows it. Field names are the
/// wire names of `GET /api/admin/family-members/bulk/batches/{id}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct BatchStatusItem {
    pub person_key: String,
    pub display_name: String,
    pub status: String,
    pub reason: Option<String>,
    pub delivery_status: String,
    pub provider: Option<String>,
    pub attempts: i32,
    pub username: Option<String>,
    pub invitation_status: Option<String>,
    pub has_verified_contact: bool,
}

/// Reads every item of one batch for the admin status view, in item order.
/// Read-only.
pub fn batch_status_items(
    client: &mut Client,
    batch_id: i64,
) -> Result<Vec<BatchStatusItem>, postgres::Error> {
    let rows = client.query(
        "SELECT person.person_key, person.display_name, item.status, item.reason_code,
                item.delivery_status, item.delivery_provider,
                item.delivery_attempts, reservation.username,
                invitation.status,
                EXISTS (SELECT 1 FROM onboarding_recipient_contact contact
                         WHERE contact.person_id = item.person_id
                           AND contact.verified_at IS NOT NULL) AS has_verified_contact
           FROM onboarding_batch_item item
           JOIN person ON person.person_id = item.person_id
           LEFT JOIN onboarding_username_reservation reservation
             ON reservation.reservation_id = item.reservation_id
           LEFT JOIN onboarding_invitation invitation
             ON invitation.invitation_id = item.invitation_id
          WHERE item.batch_id = $1
          ORDER BY item.batch_item_id",
        &[&batch_id],
    )?;
    Ok(rows
        .iter()
        .map(|row| BatchStatusItem {
            person_key: row.get(0),
            display_name: row.get(1),
            status: row.get(2),
            reason: row.get(3),
            delivery_status: row.get(4),
            provider: row.get(5),
            attempts: row.get(6),
            username: row.get(7),
            invitation_status: row.get(8),
            has_verified_contact: row.get(9),
        })
        .collect())
}

/// The outcome of one confirmation request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchReport {
    /// The batch primary key.
    pub batch_id: i64,
    /// The stable batch idempotency key.
    pub batch_key: String,
    /// Items in the batch.
    pub items: Vec<BatchItem>,
    /// True when this call inserted a new batch row.
    pub created: bool,
}

impl BatchReport {
    /// Count of items with a given status.
    pub fn count(&self, status: ItemStatus) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == status)
            .count()
    }
}

/// Maps a blocking reason to the stored reason code.
fn reason_code(classification: Classification, reasons: &[Reason]) -> &'static str {
    if classification == Classification::NeedsReview {
        for reason in reasons {
            match reason {
                Reason::Disputed => return "evidence_disputed",
                Reason::ContradictoryVital => return "contradictory_vital",
                Reason::MissingCitation => return "evidence_missing",
                Reason::UnresolvedDuplicate => return "unresolved_duplicate",
                Reason::AmbiguousSegmentation => return "ambiguous_segmentation",
                Reason::MinorGuardianReview => return "minor_guardian_review",
                _ => {}
            }
        }
        return "not_eligible";
    }
    match classification {
        Classification::EligibleForInvite => "confirmed",
        Classification::AlreadyLinked => "already_linked",
        Classification::Invited => "already_invited",
        Classification::AlreadyPrepared => "already_prepared",
        Classification::NotEligible => "not_eligible",
        Classification::NeedsReview => "not_eligible",
    }
}

fn status_for(classification: Classification) -> ItemStatus {
    match classification {
        Classification::EligibleForInvite => ItemStatus::Prepared,
        Classification::AlreadyLinked => ItemStatus::SkippedAlreadyLinked,
        Classification::Invited => ItemStatus::SkippedAlreadyInvited,
        Classification::AlreadyPrepared => ItemStatus::SkippedAlreadyPrepared,
        Classification::NotEligible => ItemStatus::SkippedNotEligible,
        Classification::NeedsReview => ItemStatus::NeedsReview,
    }
}

/// Confirms a batch and records one durable item per distinct selected person.
///
/// The confirmation count must equal the count of distinct person ids. A
/// mismatch returns [`BatchError::CountMismatch`] before any write.
///
/// The insert is idempotent: the unique batch key returns the existing batch
/// row on a duplicate submission, and the unique person key returns the existing
/// item row for a duplicate person.
pub fn confirm_batch(
    client: &mut Client,
    actor: &str,
    confirmed_count: usize,
    person_ids: &[i64],
    overrides: &HashMap<i64, String>,
) -> Result<BatchReport, BatchError> {
    let mut distinct = person_ids.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    if distinct.is_empty() {
        return Err(BatchError::EmptySelection);
    }
    if distinct.len() != confirmed_count {
        return Err(BatchError::CountMismatch {
            confirmed: confirmed_count,
            selected: distinct.len(),
        });
    }

    let keys = BatchKeys::derive(actor, confirmed_count, &distinct, overrides);

    // Reserve the batch row. Concurrent duplicates race here and the unique key
    // decides; the loser reads the existing row.
    let mut tx = client
        .transaction()
        .map_err(|err| BatchError::Database(err.to_string()))?;
    let inserted = tx
        .query_opt(
            "INSERT INTO onboarding_batch
                 (batch_idempotency_key, status, confirmed_count, selected_count, created_by)
             VALUES ($1, 'confirmed', $2, $3, $4)
             ON CONFLICT (batch_idempotency_key) DO NOTHING
             RETURNING batch_id",
            &[
                &keys.batch_key,
                &(confirmed_count as i32),
                &(distinct.len() as i32),
                &actor,
            ],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;

    let (batch_id, created): (i64, bool) = match inserted {
        Some(row) => {
            let id: i64 = row.get(0);
            tx.execute(
                "INSERT INTO onboarding_batch_audit
                     (batch_id, event, to_status, actor)
                 VALUES ($1, 'batch_confirmed', 'confirmed', $2)",
                &[&id, &actor],
            )
            .map_err(|err| BatchError::Database(err.to_string()))?;
            (id, true)
        }
        None => {
            let row = tx
                .query_one(
                    "SELECT batch_id FROM onboarding_batch WHERE batch_idempotency_key = $1",
                    &[&keys.batch_key],
                )
                .map_err(|err| BatchError::Database(err.to_string()))?;
            (row.get(0), false)
        }
    };
    tx.commit()
        .map_err(|err| BatchError::Database(err.to_string()))?;

    // Record every distinct person as a pending item. ON CONFLICT keeps the
    // existing item so a restart does not overwrite a terminal outcome.
    for person_id in &distinct {
        let person_key = keys.person_key(*person_id);
        client
            .execute(
                "INSERT INTO onboarding_batch_item
                     (batch_id, person_id, person_idempotency_key, status)
                 VALUES ($1, $2, $3, 'pending')
                 ON CONFLICT DO NOTHING",
                &[&batch_id, person_id, &person_key],
            )
            .map_err(|err| BatchError::Database(err.to_string()))?;
    }

    let items = run_batch(client, batch_id, &distinct, overrides, actor)?;

    Ok(BatchReport {
        batch_id,
        batch_key: keys.batch_key,
        items,
        created,
    })
}

/// Writes one terminal item outcome and its audit event atomically.
///
/// A concurrent writer that already moved the item out of `pending` wins; this
/// function then returns the existing row without a second audit event.
fn record_item(
    client: &mut Client,
    batch_id: i64,
    person_id: i64,
    status: ItemStatus,
    reason: &str,
    reservation_id: Option<i64>,
    actor: &str,
) -> Result<BatchItem, BatchError> {
    let mut tx = client
        .transaction()
        .map_err(|err| BatchError::Database(err.to_string()))?;
    let updated = tx
        .execute(
            "UPDATE onboarding_batch_item
                SET status = $1, reason_code = $2, reservation_id = COALESCE(reservation_id, $3),
                    updated_at = now(), completed_at = now()
              WHERE batch_id = $4 AND person_id = $5 AND status = 'pending'",
            &[
                &status.as_str(),
                &reason,
                &reservation_id,
                &batch_id,
                &person_id,
            ],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;
    if updated == 1 {
        let event = match status {
            ItemStatus::Failed => "item_failed",
            ItemStatus::NeedsReview => "item_needs_review",
            ItemStatus::Prepared => "item_prepared",
            _ => "item_skipped",
        };
        tx.execute(
            "INSERT INTO onboarding_batch_audit
                 (batch_id, person_id, event, to_status, reason_code, actor)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &batch_id,
                &person_id,
                &event,
                &status.as_str(),
                &reason,
                &actor,
            ],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;
    }
    let row = tx
        .query_one(
            "SELECT batch_item_id, status, reason_code, reservation_id
               FROM onboarding_batch_item
              WHERE batch_id = $1 AND person_id = $2",
            &[&batch_id, &person_id],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;
    tx.commit()
        .map_err(|err| BatchError::Database(err.to_string()))?;
    Ok(item_from_row(row, person_id))
}

fn item_from_row(row: postgres::Row, person_id: i64) -> BatchItem {
    let status_text: String = row.get(1);
    BatchItem {
        batch_item_id: row.get(0),
        person_id,
        status: ItemStatus::from_db(&status_text).unwrap_or(ItemStatus::Failed),
        reason_code: row.get(2),
        reservation_id: row.get(3),
    }
}

/// Processes the pending items of one batch in bounded chunks with one commit
/// per person.
///
/// Terminal items are skipped. Each person is re-evaluated for living evidence
/// and eligibility before any mutation. A failing row records
/// [`ItemStatus::Failed`] and leaves earlier committed rows unchanged.
pub fn run_batch(
    client: &mut Client,
    batch_id: i64,
    person_ids: &[i64],
    overrides: &HashMap<i64, String>,
    actor: &str,
) -> Result<Vec<BatchItem>, BatchError> {
    let mut items = Vec::new();
    for chunk in person_ids.chunks(CHUNK_SIZE) {
        for person_id in chunk {
            match process_one(client, batch_id, *person_id, overrides, actor) {
                Ok(item) => items.push(item),
                Err(err) => {
                    let reason = match &err {
                        BatchError::Reservation(_) => "username_collision",
                        _ => "database_error",
                    };
                    // The failure itself may be the database link; record the
                    // safe reason and keep earlier committed rows.
                    items.push(record_item(
                        client,
                        batch_id,
                        *person_id,
                        ItemStatus::Failed,
                        reason,
                        None,
                        actor,
                    )?);
                }
            }
        }
    }

    let failed: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_batch_item
              WHERE batch_id = $1 AND status = 'failed'",
            &[&batch_id],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?
        .get(0);
    let status = if failed > 0 {
        "partially_failed"
    } else {
        "completed"
    };
    client
        .execute(
            "UPDATE onboarding_batch
                SET updated_at = now(), status = $2, completed_at = now()
              WHERE batch_id = $1",
            &[&batch_id, &status],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;
    client
        .execute(
            "INSERT INTO onboarding_batch_audit (batch_id, event, to_status, actor)
             VALUES ($1, 'batch_completed', $2, $3)",
            &[&batch_id, &status, &actor],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;

    Ok(items)
}

fn process_one(
    client: &mut Client,
    batch_id: i64,
    person_id: i64,
    overrides: &HashMap<i64, String>,
    actor: &str,
) -> Result<BatchItem, BatchError> {
    // Read the item. A terminal item is skipped on restart.
    let row = client
        .query_opt(
            "SELECT batch_item_id, status, reason_code, reservation_id
               FROM onboarding_batch_item
              WHERE batch_id = $1 AND person_id = $2",
            &[&batch_id, &person_id],
        )
        .map_err(|err| BatchError::Database(err.to_string()))?;
    let Some(row) = row else {
        return Err(BatchError::Database(format!(
            "batch item missing for person {person_id}"
        )));
    };
    let status_text: String = row.get(1);
    if ItemStatus::from_db(&status_text)
        .map(ItemStatus::is_terminal)
        .unwrap_or(false)
    {
        return Ok(item_from_row(row, person_id));
    }

    let person_key: String = client
        .query_one(
            "SELECT person_key FROM person WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| BatchError::Evidence {
            person_id,
            reason: err.to_string(),
        })?
        .get(0);

    // Re-evaluate eligibility before any mutation. This is the security
    // prerequisite for both preparation and invitation creation.
    let (classification, reasons) = reclassify_person(client, person_id, &person_key)?;

    if classification == Classification::EligibleForInvite {
        // A different batch may already hold the logical prepared entry.
        let prepared_elsewhere: bool = client
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM onboarding_batch_item
                      WHERE person_id = $1 AND status = 'prepared' AND batch_id <> $2
                 )",
                &[&person_id, &batch_id],
            )
            .map_err(|err| BatchError::Database(err.to_string()))?
            .get(0);
        if prepared_elsewhere {
            return record_item(
                client,
                batch_id,
                person_id,
                ItemStatus::SkippedAlreadyPrepared,
                "already_prepared",
                None,
                actor,
            );
        }

        let parts = load_name_parts(client, person_id)?;
        let override_name = overrides.get(&person_id).map(String::as_str);
        match reserve(client, person_id, &parts, override_name, actor) {
            Ok(reservation) => record_item(
                client,
                batch_id,
                person_id,
                ItemStatus::Prepared,
                "confirmed",
                Some(reservation.reservation_id),
                actor,
            ),
            Err(err) => {
                let reason = match &err {
                    ReservationError::Policy(_) => "username_policy",
                    ReservationError::Exhausted => "username_collision",
                    _ => "username_collision",
                };
                record_item(
                    client,
                    batch_id,
                    person_id,
                    ItemStatus::Failed,
                    reason,
                    None,
                    actor,
                )
            }
        }
    } else {
        let reason = reason_code(classification, &reasons);
        record_item(
            client,
            batch_id,
            person_id,
            status_for(classification),
            reason,
            None,
            actor,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn batch_key_is_stable_for_sorted_inputs() {
        let a = BatchKeys::derive("admin", 2, &[3, 1], &HashMap::new());
        let b = BatchKeys::derive("admin", 2, &[1, 3], &HashMap::new());
        assert_eq!(a.batch_key, b.batch_key);
    }

    #[test]
    fn batch_key_without_overrides_is_unchanged() {
        let keys = BatchKeys::derive("admin", 2, &[3, 1], &HashMap::new());
        assert_eq!(
            keys.batch_key,
            format!("batch:admin:2:{}", sha256_hex(b"1,3"))
        );
    }

    #[test]
    fn batch_key_changes_with_overrides() {
        let none = BatchKeys::derive("admin", 1, &[1], &HashMap::new());
        let bad = BatchKeys::derive("admin", 1, &[1], &HashMap::from([(1, "bad!".into())]));
        let fixed = BatchKeys::derive("admin", 1, &[1], &HashMap::from([(1, "good".into())]));
        let again = BatchKeys::derive("admin", 1, &[1], &HashMap::from([(1, "good".into())]));
        assert_ne!(none.batch_key, bad.batch_key);
        assert_ne!(bad.batch_key, fixed.batch_key);
        assert_eq!(fixed.batch_key, again.batch_key);
    }

    #[test]
    fn batch_key_changes_with_count() {
        let a = BatchKeys::derive("admin", 2, &[1, 3], &HashMap::new());
        let b = BatchKeys::derive("admin", 3, &[1, 3], &HashMap::new());
        assert_ne!(a.batch_key, b.batch_key);
    }

    #[test]
    fn person_key_uses_batch_key_and_person() {
        let keys = BatchKeys::derive("admin", 1, &[7], &HashMap::new());
        assert_eq!(keys.person_key(7), format!("{}:7", keys.batch_key));
        assert_ne!(keys.person_key(7), keys.person_key(8));
    }

    #[test]
    fn sha256_matches_known_vector() {
        // Well-known SHA-256 of the empty string.
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn item_status_round_trips() {
        for status in [
            ItemStatus::Pending,
            ItemStatus::Prepared,
            ItemStatus::SkippedAlreadyLinked,
            ItemStatus::SkippedAlreadyInvited,
            ItemStatus::SkippedAlreadyPrepared,
            ItemStatus::SkippedNotEligible,
            ItemStatus::SkippedDuplicate,
            ItemStatus::NeedsReview,
            ItemStatus::Failed,
        ] {
            assert_eq!(ItemStatus::from_db(status.as_str()), Some(status));
        }
    }

    #[test]
    fn terminal_statuses_are_not_pending() {
        assert!(!ItemStatus::Pending.is_terminal());
        assert!(ItemStatus::Prepared.is_terminal());
        assert!(ItemStatus::Failed.is_terminal());
    }
}
