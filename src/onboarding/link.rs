//! Reviewed account-to-person linking.
//!
//! A link is written only after an explicit review decision. The genealogy
//! database stores the opaque identity account id and the identity fingerprint,
//! never a raw subject or issuer. Every decision appends an audit row in the same
//! transaction as the change it describes.
//!
//! There is one active link per person and one per account (enforced by the
//! schema). A conflict returns [`LinkError::AlreadyLinked`]; the code never
//! silently replaces or merges two records.

use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;

/// The reviewer's decision for one link request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkDecision {
    /// Create the link.
    Approve,
    /// Record a rejection without creating a link.
    Reject,
}

impl LinkDecision {
    /// The audit value for the decision.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkDecision::Approve => "linked",
            LinkDecision::Reject => "rejected",
        }
    }
}

/// One active person-account link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRecord {
    /// The genealogy link id.
    pub link_id: i64,
    /// The genealogy person id.
    pub person_id: i64,
    /// The opaque identity account id.
    pub account_id: i64,
    /// The identity fingerprint stored with the link.
    pub identity_fingerprint: String,
    /// The reviewer that approved the link.
    pub linked_by: String,
}

/// Errors raised by linking operations.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum LinkError {
    /// No person row exists for the id.
    #[error("unknown person")]
    UnknownPerson,
    /// The person or account already has an active link.
    #[error("person or account is already linked")]
    AlreadyLinked,
    /// The reviewer is blank.
    #[error("a reviewer is required")]
    InvalidReviewer,
    /// The review rejected the link, so no link was created.
    #[error("link review rejected the request")]
    Rejected,
    /// No link exists for the id.
    #[error("unknown link")]
    UnknownLink,
    /// The genealogy database operation failed.
    #[error("link database operation failed: {0}")]
    Database(String),
}

/// Columns returned for one link row.
const LINK_COLUMNS: &str = "link_id, person_id, account_id, identity_fingerprint, linked_by";

fn row_to_link(row: postgres::Row) -> LinkRecord {
    LinkRecord {
        link_id: row.get(0),
        person_id: row.get(1),
        account_id: row.get(2),
        identity_fingerprint: row.get(3),
        linked_by: row.get(4),
    }
}

/// Applies one reviewed link decision.
///
/// `Approve` inserts one link and one `linked` audit row atomically. `Reject`
/// inserts only a `rejected` audit row and returns `Ok(None)`. A blank reviewer
/// is rejected before any write.
pub fn link_reviewed(
    client: &mut postgres::Client,
    person_id: i64,
    account: &IdentityAccount,
    decision: LinkDecision,
    reviewer: &str,
) -> Result<Option<LinkRecord>, LinkError> {
    let reviewer = reviewer.trim();
    if reviewer.is_empty() {
        return Err(LinkError::InvalidReviewer);
    }

    let mut transaction = client
        .transaction()
        .map_err(|err| LinkError::Database(err.to_string()))?;

    let exists: bool = transaction
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM person WHERE person_id = $1)",
            &[&person_id],
        )
        .map_err(|err| LinkError::Database(err.to_string()))?
        .get(0);
    if !exists {
        return Err(LinkError::UnknownPerson);
    }

    match decision {
        LinkDecision::Reject => {
            transaction
                .execute(
                    "INSERT INTO onboarding_link_audit \
                        (person_id, account_id, identity_fingerprint, decision, reviewer) \
                     VALUES ($1, $2, $3, 'rejected', $4)",
                    &[
                        &person_id,
                        &account.account_id,
                        &account.identity_fingerprint,
                        &reviewer,
                    ],
                )
                .map_err(|err| LinkError::Database(err.to_string()))?;
            transaction
                .commit()
                .map_err(|err| LinkError::Database(err.to_string()))?;
            Ok(None)
        }
        LinkDecision::Approve => {
            let insert = format!(
                "INSERT INTO onboarding_person_account_link \
                     (person_id, account_id, identity_fingerprint, linked_by) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT DO NOTHING \
                 RETURNING {LINK_COLUMNS}"
            );
            let row = transaction
                .query_opt(
                    &insert,
                    &[
                        &person_id,
                        &account.account_id,
                        &account.identity_fingerprint,
                        &reviewer,
                    ],
                )
                .map_err(|err| LinkError::Database(err.to_string()))?;
            let Some(row) = row else {
                // The transaction is dropped without a commit, so neither the
                // audit row nor a partial link survives.
                return Err(LinkError::AlreadyLinked);
            };
            transaction
                .execute(
                    "INSERT INTO onboarding_link_audit \
                        (person_id, account_id, identity_fingerprint, decision, reviewer) \
                     VALUES ($1, $2, $3, 'linked', $4)",
                    &[
                        &person_id,
                        &account.account_id,
                        &account.identity_fingerprint,
                        &reviewer,
                    ],
                )
                .map_err(|err| LinkError::Database(err.to_string()))?;
            let record = row_to_link(row);
            transaction
                .commit()
                .map_err(|err| LinkError::Database(err.to_string()))?;
            Ok(Some(record))
        }
    }
}

/// Revokes one active link and appends a `revoked` audit row atomically.
pub fn revoke_link(
    client: &mut postgres::Client,
    link_id: i64,
    reviewer: &str,
) -> Result<(), LinkError> {
    let reviewer = reviewer.trim();
    if reviewer.is_empty() {
        return Err(LinkError::InvalidReviewer);
    }

    let mut transaction = client
        .transaction()
        .map_err(|err| LinkError::Database(err.to_string()))?;
    let row = transaction
        .query_opt(
            "DELETE FROM onboarding_person_account_link \
              WHERE link_id = $1 \
              RETURNING person_id, account_id, identity_fingerprint",
            &[&link_id],
        )
        .map_err(|err| LinkError::Database(err.to_string()))?;
    let Some(row) = row else {
        return Err(LinkError::UnknownLink);
    };
    let person_id: i64 = row.get(0);
    let account_id: i64 = row.get(1);
    let fingerprint: String = row.get(2);
    transaction
        .execute(
            "INSERT INTO onboarding_link_audit \
                (person_id, account_id, identity_fingerprint, decision, reviewer) \
             VALUES ($1, $2, $3, 'revoked', $4)",
            &[&person_id, &account_id, &fingerprint, &reviewer],
        )
        .map_err(|err| LinkError::Database(err.to_string()))?;
    transaction
        .commit()
        .map_err(|err| LinkError::Database(err.to_string()))?;
    Ok(())
}

/// Returns the active link for a person, if any.
pub fn link_for_person(
    client: &mut postgres::Client,
    person_id: i64,
) -> Result<Option<LinkRecord>, LinkError> {
    let sql =
        format!("SELECT {LINK_COLUMNS} FROM onboarding_person_account_link WHERE person_id = $1");
    let row = client
        .query_opt(&sql, &[&person_id])
        .map_err(|err| LinkError::Database(err.to_string()))?;
    Ok(row.map(row_to_link))
}

/// Returns the active link owned by one verified identity, if any.
///
/// Ownership requires both the opaque account id and the fingerprint of the
/// exact `(issuer, subject)` pair. The fingerprint binds the row to that pair as
/// a defense-in-depth check; it does not replace database access control. The
/// `account_id` predicate uses the existing unique
/// `onboarding_person_account_link_account_key` index; the fingerprint predicate
/// is a binding check, not a second lookup key.
pub fn link_for_account(
    client: &mut postgres::Client,
    account_id: i64,
    identity_fingerprint: &str,
) -> Result<Option<LinkRecord>, LinkError> {
    let sql = format!(
        "SELECT {LINK_COLUMNS} FROM onboarding_person_account_link \
          WHERE account_id = $1 AND identity_fingerprint = $2"
    );
    let row = client
        .query_opt(&sql, &[&account_id, &identity_fingerprint])
        .map_err(|err| LinkError::Database(err.to_string()))?;
    Ok(row.map(row_to_link))
}
