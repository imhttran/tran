//! Explicit branch-scoped reviewer grants (USR-005A).
//!
//! A grant is an administrator-issued, audited authorization for one reviewed
//! OIDC account to review one family branch. It is never inferred from a
//! genealogy title, Head of Family appointment, branch membership, or role.
//!
//! The grant row is bound to the exact `identity_fingerprint` of the reviewed
//! account-person link. Replacing the account or revoking the link therefore
//! makes the old grant unusable; a new account never inherits it. Every grant
//! and revocation appends an immutable event.

use postgres::{Client, IsolationLevel, Transaction};
use thiserror::Error;

use crate::onboarding::account_map::IdentityAccount;

/// Maximum length of a grant or revocation reason.
pub const GRANT_REASON_MAX_CHARS: usize = 1_000;

/// One family branch available for scoping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSummary {
    pub family_branch_id: i64,
    pub branch_key: String,
    pub branch_label: Option<String>,
}

/// One current or historical grant row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchReviewerGrant {
    pub grant_id: i64,
    pub account_id: i64,
    pub identity_fingerprint: Option<String>,
    pub family_branch_id: i64,
    pub branch_key: String,
    pub branch_label: Option<String>,
    pub granted_by: String,
    pub granted_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub revoked_at: Option<String>,
    pub revoked_by: Option<String>,
    pub revoke_reason: Option<String>,
    pub active: bool,
    pub usable: bool,
}

/// One append-only grant or revocation event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchGrantEvent {
    pub event_id: i64,
    pub grant_id: i64,
    pub account_id: i64,
    pub family_branch_id: i64,
    pub branch_key: String,
    pub event: String,
    pub actor: String,
    pub reason: Option<String>,
    pub created_at: String,
}

/// Errors raised by grant administration or authorization.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum BranchGrantError {
    #[error("at least one branch is required")]
    EmptySelection,
    #[error("branch keys must not be blank")]
    BlankBranch,
    #[error("unknown family branch: {0}")]
    UnknownBranch(String),
    #[error("the account has no reviewed person link")]
    Unlinked,
    #[error("an active grant already exists for branch {0}")]
    AlreadyActive(String),
    #[error("no active grant exists for branch {0}")]
    NotActive(String),
    #[error("unknown branch reviewer grant")]
    UnknownGrant,
    #[error("a grant actor is required")]
    InvalidActor,
    #[error("a non-empty reason is required")]
    InvalidReason,
    #[error("branch grant database operation failed: {0}")]
    Database(String),
}

impl From<postgres::Error> for BranchGrantError {
    fn from(err: postgres::Error) -> Self {
        BranchGrantError::Database(err.to_string())
    }
}

const GRANT_COLUMNS: &str = "g.grant_id, g.account_id, g.identity_fingerprint, \
     g.family_branch_id, b.branch_key, b.branch_label, g.granted_by, g.granted_reason, \
     g.created_at::text, g.updated_at::text, g.revoked_at::text, g.revoked_by, g.revoke_reason, \
     EXISTS (SELECT 1 FROM onboarding_person_account_link l \
              WHERE l.account_id = g.account_id \
                AND l.identity_fingerprint = g.identity_fingerprint)";

/// Lists the family branches an administrator may grant.
pub fn list_branches(client: &mut Client) -> Result<Vec<BranchSummary>, BranchGrantError> {
    let rows = client.query(
        "SELECT family_branch_id, branch_key, branch_label \
           FROM family_branch ORDER BY branch_key, family_branch_id",
        &[],
    )?;
    Ok(rows
        .into_iter()
        .map(|row| BranchSummary {
            family_branch_id: row.get(0),
            branch_key: row.get(1),
            branch_label: row.get(2),
        })
        .collect())
}

/// One bounded, repeatable-read snapshot of grants and their audit events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantSnapshot {
    pub grants: Vec<BranchReviewerGrant>,
    pub events: Vec<BranchGrantEvent>,
    pub grants_next_cursor: Option<i64>,
    pub events_next_cursor: Option<i64>,
    pub limit: i64,
}

/// Reads bounded grant and event pages from one database snapshot.
///
/// Both lists use descending id cursors and a hard limit of 100. The
/// repeatable-read transaction prevents a concurrent grant or revocation from
/// making the grants and events in one response contradict each other.
pub fn read_grant_snapshot(
    client: &mut Client,
    account_id: Option<i64>,
    grant_cursor: Option<i64>,
    event_cursor: Option<i64>,
    limit: i64,
) -> Result<GrantSnapshot, BranchGrantError> {
    let limit = limit.clamp(1, 100);
    let mut tx = client
        .build_transaction()
        .isolation_level(IsolationLevel::RepeatableRead)
        .start()?;
    let grant_sql = format!(
        "SELECT {GRANT_COLUMNS} \
           FROM onboarding_branch_reviewer_grant g \
           JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
          WHERE ($1::bigint IS NULL OR g.account_id = $1) \
            AND ($2::bigint IS NULL OR g.grant_id < $2) \
          ORDER BY g.grant_id DESC \
          LIMIT $3"
    );
    let rows = tx.query(&grant_sql, &[&account_id, &grant_cursor, &(limit + 1)])?;
    let mut grants = rows
        .into_iter()
        .map(row_to_grant)
        .collect::<Result<Vec<_>, _>>()?;
    let grants_next_cursor = if grants.len() as i64 > limit {
        grants.truncate(limit as usize);
        grants.last().map(|grant| grant.grant_id)
    } else {
        None
    };
    let event_rows = tx.query(
        "SELECT e.event_id, e.grant_id, e.account_id, e.family_branch_id, \
                COALESCE(e.branch_key, b.branch_key, 'deleted-branch'), \
                e.event, e.actor, e.reason, e.created_at::text \
           FROM onboarding_branch_reviewer_grant_event e \
           LEFT JOIN family_branch b ON b.family_branch_id = e.family_branch_id \
          WHERE ($1::bigint IS NULL OR e.account_id = $1) \
            AND ($2::bigint IS NULL OR e.event_id < $2) \
          ORDER BY e.event_id DESC \
          LIMIT $3",
        &[&account_id, &event_cursor, &(limit + 1)],
    )?;
    let mut events = event_rows
        .into_iter()
        .map(|row| BranchGrantEvent {
            event_id: row.get(0),
            grant_id: row.get(1),
            account_id: row.get(2),
            family_branch_id: row.get(3),
            branch_key: row.get(4),
            event: row.get(5),
            actor: row.get(6),
            reason: row.get(7),
            created_at: row.get(8),
        })
        .collect::<Vec<_>>();
    let events_next_cursor = if events.len() as i64 > limit {
        events.truncate(limit as usize);
        events.last().map(|event| event.event_id)
    } else {
        None
    };
    tx.commit()?;
    Ok(GrantSnapshot {
        grants,
        events,
        grants_next_cursor,
        events_next_cursor,
        limit,
    })
}

/// Grants one or more branches to a verified, active, reviewed-linked account.
///
/// The account must already be resolved through the identity store by the API
/// caller; this function rechecks the reviewed link in the genealogy database
/// inside the transaction. All requested branches must succeed or none is
/// written.
pub fn grant_branches(
    client: &mut Client,
    account: &IdentityAccount,
    branch_keys: &[String],
    actor: &str,
    reason: &str,
) -> Result<Vec<BranchReviewerGrant>, BranchGrantError> {
    let actor = clean_actor(actor)?;
    let reason = clean_reason(reason)?;
    let mut keys: Vec<String> = Vec::with_capacity(branch_keys.len());
    for key in branch_keys {
        let key = key.trim();
        if key.is_empty() {
            return Err(BranchGrantError::BlankBranch);
        }
        keys.push(key.to_string());
    }
    keys.sort();
    keys.dedup();
    if keys.is_empty() {
        return Err(BranchGrantError::EmptySelection);
    }

    let mut tx = client.transaction()?;
    lock_reviewed_link(&mut tx, account)?;
    let mut grants = Vec::with_capacity(keys.len());
    for key in &keys {
        let branch = branch_by_key(&mut tx, key)?;
        grant_one(&mut tx, account, &branch, &actor, &reason)?;
        grants.push(load_grant(
            &mut tx,
            account.account_id,
            branch.family_branch_id,
        )?);
    }
    tx.commit()?;
    Ok(grants)
}

/// Revokes one active grant and appends an immutable revocation event.
pub fn revoke_grant(
    client: &mut Client,
    grant_id: i64,
    actor: &str,
    reason: &str,
) -> Result<BranchReviewerGrant, BranchGrantError> {
    let actor = clean_actor(actor)?;
    let reason = clean_reason(reason)?;
    let mut tx = client.transaction()?;
    let ids = tx
        .query_opt(
            "SELECT g.account_id, g.family_branch_id, b.branch_key \
               FROM onboarding_branch_reviewer_grant g \
               JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
              WHERE g.grant_id = $1",
            &[&grant_id],
        )?
        .ok_or(BranchGrantError::UnknownGrant)?;
    let account_id: i64 = ids.get(0);
    let family_branch_id: i64 = ids.get(1);
    let branch_key: String = ids.get(2);
    // Match the grant path's person -> advisory -> grant-row order. Revoke does
    // not need the link, but taking the same locks prevents a grant/revoke
    // deadlock when the target account is still linked.
    if let Some(link) = tx.query_opt(
        "SELECT person_id FROM onboarding_person_account_link WHERE account_id = $1",
        &[&account_id],
    )? {
        let person_id: i64 = link.get(0);
        tx.query_one(
            "SELECT 1 FROM person WHERE person_id = $1 FOR UPDATE",
            &[&person_id],
        )?;
    }
    lock_grant_key(&mut tx, account_id, family_branch_id)?;
    let row = tx
        .query_opt(
            "SELECT g.revoked_at::text \
               FROM onboarding_branch_reviewer_grant g \
              WHERE g.grant_id = $1 FOR UPDATE",
            &[&grant_id],
        )?
        .ok_or(BranchGrantError::UnknownGrant)?;
    let revoked_at: Option<String> = row.get(0);
    if revoked_at.is_some() {
        return Err(BranchGrantError::NotActive(branch_key));
    }
    tx.execute(
        "UPDATE onboarding_branch_reviewer_grant \
            SET revoked_at = now(), revoked_by = $2, revoke_reason = $3, updated_at = now() \
          WHERE grant_id = $1 AND revoked_at IS NULL",
        &[&grant_id, &actor, &reason],
    )?;
    let grant = load_grant_by_id(&mut tx, grant_id)?;
    insert_event(
        &mut tx,
        grant.grant_id,
        account_id,
        family_branch_id,
        &branch_key,
        grant.identity_fingerprint.as_deref(),
        "revoked",
        &actor,
        Some(&reason),
    )?;
    tx.commit()?;
    Ok(grant)
}

/// Returns the active grants usable by this exact account identity.
///
/// A grant requires the exact `(account_id, identity_fingerprint)` reviewed link
/// to still exist. Revoked links, replaced accounts, and legacy rows without a
/// fingerprint therefore yield no authority.
pub fn active_branch_grants(
    client: &mut Client,
    account: &IdentityAccount,
) -> Result<Vec<BranchReviewerGrant>, BranchGrantError> {
    let sql = format!(
        "SELECT {GRANT_COLUMNS} \
           FROM onboarding_branch_reviewer_grant g \
           JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
          WHERE g.account_id = $1 \
            AND g.identity_fingerprint = $2 \
            AND g.revoked_at IS NULL \
            AND EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                         WHERE l.account_id = g.account_id \
                           AND l.identity_fingerprint = g.identity_fingerprint) \
          ORDER BY b.branch_key"
    );
    let rows = client.query(&sql, &[&account.account_id, &account.identity_fingerprint])?;
    rows.into_iter().map(row_to_grant).collect()
}

/// Reports whether this exact account identity holds an active grant for a
/// branch key. The caller still decides what the branch key means.
pub fn has_active_branch_grant(
    client: &mut Client,
    account: &IdentityAccount,
    branch_key: &str,
) -> Result<bool, BranchGrantError> {
    let exists: bool = client
        .query_one(
            "SELECT EXISTS (\
                SELECT 1 \
                  FROM onboarding_branch_reviewer_grant g \
                  JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
                 WHERE g.account_id = $1 \
                   AND g.identity_fingerprint = $2 \
                   AND b.branch_key = $3 \
                   AND g.revoked_at IS NULL \
                   AND EXISTS (SELECT 1 FROM onboarding_person_account_link l \
                                WHERE l.account_id = g.account_id \
                                  AND l.identity_fingerprint = g.identity_fingerprint)\
             )",
            &[
                &account.account_id,
                &account.identity_fingerprint,
                &branch_key,
            ],
        )?
        .get(0);
    Ok(exists)
}

fn row_to_grant(row: postgres::Row) -> Result<BranchReviewerGrant, BranchGrantError> {
    let revoked_at: Option<String> = row.get(10);
    let active = revoked_at.is_none();
    let linked: bool = row.get(13);
    Ok(BranchReviewerGrant {
        grant_id: row.get(0),
        account_id: row.get(1),
        identity_fingerprint: row.get(2),
        family_branch_id: row.get(3),
        branch_key: row.get(4),
        branch_label: row.get(5),
        granted_by: row.get(6),
        granted_reason: row.get(7),
        created_at: row.get(8),
        updated_at: row.get(9),
        active,
        revoked_at,
        revoked_by: row.get(11),
        revoke_reason: row.get(12),
        usable: active && linked,
    })
}

fn clean_actor(actor: &str) -> Result<String, BranchGrantError> {
    let actor = actor.trim();
    if actor.is_empty() {
        return Err(BranchGrantError::InvalidActor);
    }
    Ok(actor.to_string())
}

fn clean_reason(reason: &str) -> Result<String, BranchGrantError> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(BranchGrantError::InvalidReason);
    }
    if reason.chars().count() > GRANT_REASON_MAX_CHARS {
        return Err(BranchGrantError::InvalidReason);
    }
    Ok(reason.to_string())
}

fn branch_by_key(
    tx: &mut Transaction<'_>,
    branch_key: &str,
) -> Result<BranchSummary, BranchGrantError> {
    let row = tx
        .query_opt(
            "SELECT family_branch_id, branch_key, branch_label \
               FROM family_branch WHERE branch_key = $1",
            &[&branch_key],
        )?
        .ok_or_else(|| BranchGrantError::UnknownBranch(branch_key.to_string()))?;
    Ok(BranchSummary {
        family_branch_id: row.get(0),
        branch_key: row.get(1),
        branch_label: row.get(2),
    })
}

fn lock_grant_key(
    tx: &mut Transaction<'_>,
    account_id: i64,
    family_branch_id: i64,
) -> Result<(), BranchGrantError> {
    let key = format!("branch-reviewer-grant:{account_id}:{family_branch_id}");
    tx.query_one(
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
        &[&key],
    )?;
    Ok(())
}

fn lock_reviewed_link(
    tx: &mut Transaction<'_>,
    account: &IdentityAccount,
) -> Result<i64, BranchGrantError> {
    let row = tx
        .query_opt(
            "SELECT person_id FROM onboarding_person_account_link \
              WHERE account_id = $1 AND identity_fingerprint = $2",
            &[&account.account_id, &account.identity_fingerprint],
        )?
        .ok_or(BranchGrantError::Unlinked)?;
    let person_id: i64 = row.get(0);
    // Link creation/revocation and profile review lock the person row. Lock it
    // before rechecking the link so a concurrent revocation cannot slip between
    // the check and the grant write.
    tx.query_one(
        "SELECT 1 FROM person WHERE person_id = $1 FOR UPDATE",
        &[&person_id],
    )?;
    let still_linked: bool = tx
        .query_one(
            "SELECT EXISTS (\
                SELECT 1 FROM onboarding_person_account_link \
                 WHERE account_id = $1 AND identity_fingerprint = $2\
             )",
            &[&account.account_id, &account.identity_fingerprint],
        )?
        .get(0);
    if !still_linked {
        return Err(BranchGrantError::Unlinked);
    }
    Ok(person_id)
}

fn grant_one(
    tx: &mut Transaction<'_>,
    account: &IdentityAccount,
    branch: &BranchSummary,
    actor: &str,
    reason: &str,
) -> Result<(), BranchGrantError> {
    lock_grant_key(tx, account.account_id, branch.family_branch_id)?;
    let active = tx.query_opt(
        "SELECT grant_id, identity_fingerprint \
           FROM onboarding_branch_reviewer_grant \
          WHERE account_id = $1 AND family_branch_id = $2 AND revoked_at IS NULL \
          FOR UPDATE",
        &[&account.account_id, &branch.family_branch_id],
    )?;
    let grant_id = match active {
        Some(row) => {
            let grant_id: i64 = row.get(0);
            let fingerprint: Option<String> = row.get(1);
            if fingerprint.is_some() {
                return Err(BranchGrantError::AlreadyActive(branch.branch_key.clone()));
            }
            // Legacy active row without a fingerprint: bind it to this exact
            // reviewed identity instead of creating an unusable duplicate.
            tx.execute(
                "UPDATE onboarding_branch_reviewer_grant \
                    SET identity_fingerprint = $2, granted_by = $3, granted_reason = $4, \
                        updated_at = now() \
                  WHERE grant_id = $1",
                &[&grant_id, &account.identity_fingerprint, &actor, &reason],
            )?;
            grant_id
        }
        None => tx
            .query_one(
                "INSERT INTO onboarding_branch_reviewer_grant \
                     (account_id, family_branch_id, identity_fingerprint, granted_by, \
                      granted_reason, updated_at) \
                 VALUES ($1, $2, $3, $4, $5, now()) \
                 RETURNING grant_id",
                &[
                    &account.account_id,
                    &branch.family_branch_id,
                    &account.identity_fingerprint,
                    &actor,
                    &reason,
                ],
            )?
            .get(0),
    };
    insert_event(
        tx,
        grant_id,
        account.account_id,
        branch.family_branch_id,
        &branch.branch_key,
        Some(&account.identity_fingerprint),
        "granted",
        actor,
        Some(reason),
    )
}

#[allow(clippy::too_many_arguments)]
fn insert_event(
    tx: &mut Transaction<'_>,
    grant_id: i64,
    account_id: i64,
    family_branch_id: i64,
    branch_key: &str,
    identity_fingerprint: Option<&str>,
    event: &str,
    actor: &str,
    reason: Option<&str>,
) -> Result<(), BranchGrantError> {
    tx.execute(
        "INSERT INTO onboarding_branch_reviewer_grant_event \
             (grant_id, account_id, family_branch_id, branch_key, identity_fingerprint, \
              event, actor, reason) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        &[
            &grant_id,
            &account_id,
            &family_branch_id,
            &branch_key,
            &identity_fingerprint,
            &event,
            &actor,
            &reason,
        ],
    )?;
    Ok(())
}

fn load_grant_by_id(
    tx: &mut Transaction<'_>,
    grant_id: i64,
) -> Result<BranchReviewerGrant, BranchGrantError> {
    let sql = format!(
        "SELECT {GRANT_COLUMNS} \
           FROM onboarding_branch_reviewer_grant g \
           JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
          WHERE g.grant_id = $1"
    );
    tx.query_opt(&sql, &[&grant_id])?
        .map(row_to_grant)
        .transpose()?
        .ok_or(BranchGrantError::UnknownGrant)
}

fn load_grant(
    tx: &mut Transaction<'_>,
    account_id: i64,
    family_branch_id: i64,
) -> Result<BranchReviewerGrant, BranchGrantError> {
    let sql = format!(
        "SELECT {GRANT_COLUMNS} \
           FROM onboarding_branch_reviewer_grant g \
           JOIN family_branch b ON b.family_branch_id = g.family_branch_id \
          WHERE g.account_id = $1 AND g.family_branch_id = $2 \
            AND g.revoked_at IS NULL"
    );
    tx.query_opt(&sql, &[&account_id, &family_branch_id])?
        .map(row_to_grant)
        .transpose()?
        .ok_or(BranchGrantError::UnknownGrant)
}
