//! PostgreSQL implementation of the identity client contract.
//!
//! [`PgIdentityClient`] wraps a synchronous `postgres::Client`. Each role
//! change and the initial-admin bootstrap run inside a single transaction that
//! writes the `role_assignments` row and the `audit_events` row together. If any
//! statement fails the transaction is rolled back, so neither row survives.
//!
//! There is no other code path that creates a role assignment.
//!
//! Identity is immutable on the pair `(issuer, subject)` (see migration 008).
//! Every state-changing lookup below filters on `subject = $1 AND issuer = $2`.
//! There is no subject-only mutation path. A subject string that arrives from a
//! different issuer resolves to no row and fails with
//! [`IdentityStoreError::UnknownUser`]; it can never touch an existing account.

use std::cell::RefCell;
use std::ops::DerefMut;

use postgres::{Client, NoTls, Transaction};

use crate::identity::Role;
use crate::identity_store::identity_client::Client as IdentityClient;
use crate::identity_store::{IdentityStore, IdentityStoreError, UserRecord};

/// A direct connection or one borrowed from a pool; both deref to `Client`.
type Connection = Box<dyn DerefMut<Target = Client> + Send>;

/// A transaction-capable identity client backed by PostgreSQL.
pub struct PgIdentityClient {
    client: RefCell<Connection>,
}

impl PgIdentityClient {
    /// Connects to `dsn`.
    pub fn connect(dsn: &str) -> Result<Self, IdentityStoreError> {
        let client = Client::connect(dsn, NoTls)
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(Self::from_client(client))
    }

    /// Wraps an existing `postgres::Client`.
    pub fn from_client(client: Client) -> Self {
        Self::from_pooled(Box::new(client))
    }

    /// Wraps a pooled connection, which returns to its pool when dropped.
    pub fn from_pooled(connection: impl DerefMut<Target = Client> + Send + 'static) -> Self {
        Self {
            client: RefCell::new(Box::new(connection)),
        }
    }

    /// Reads the immutable identity pair for one opaque account id.
    ///
    /// The reviewed-link approval stores only the account id and a one-way
    /// fingerprint in the genealogy database, so the identity pair is read back
    /// here before an audited role or corpus write. A deactivated or unknown
    /// account yields `None`.
    pub fn active_identity_by_id(
        &mut self,
        user_id: i64,
    ) -> Result<Option<(String, String)>, IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let row = guard
            .query_opt(
                "SELECT subject, issuer FROM users \
                  WHERE id = $1 AND deactivated_at IS NULL",
                &[&user_id],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(row.map(|row| (row.get(0), row.get(1))))
    }

    /// Grants one corpus to an existing user and records the audit event.
    ///
    /// The user is resolved by immutable (issuer, subject). There is no
    /// subject-only corpus path.
    pub fn grant_corpus(
        &mut self,
        subject: &str,
        issuer: &str,
        corpus: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let row = tx
            .query_opt(
                "SELECT id FROM users WHERE subject = $1 AND issuer = $2",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let Some(row) = row else {
            return Err(IdentityStoreError::UnknownUser(subject.to_string()));
        };
        let user_id: i64 = row.get(0);
        tx.execute(
            "INSERT INTO corpus_grants (user_id, corpus, granted_by) \
             SELECT $1, $2, $3 \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM corpus_grants \
                 WHERE user_id = $1 AND corpus = $2 AND revoked_at IS NULL \
             )",
            &[&user_id, &corpus, &actor],
        )
        .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        audit(&mut tx, actor, "corpus.grant", subject, issuer)?;
        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }

    /// Revokes one corpus grant and records the audit event.
    ///
    /// The user is resolved by immutable (issuer, subject). There is no
    /// subject-only corpus path.
    pub fn revoke_corpus(
        &mut self,
        subject: &str,
        issuer: &str,
        corpus: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let row = tx
            .query_opt(
                "SELECT id FROM users WHERE subject = $1 AND issuer = $2",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let Some(row) = row else {
            return Err(IdentityStoreError::UnknownUser(subject.to_string()));
        };
        let user_id: i64 = row.get(0);
        tx.execute(
            "UPDATE corpus_grants SET revoked_at = now() \
             WHERE user_id = $1 AND corpus = $2 AND revoked_at IS NULL",
            &[&user_id, &corpus],
        )
        .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        audit(&mut tx, actor, "corpus.revoke", subject, issuer)?;
        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }

    /// Provisions or reactivates an account by immutable issuer + subject.
    ///
    /// The row is inserted only when `(issuer, subject)` is absent. When it is
    /// present, only that matching row is reactivated. The statement never
    /// assigns `issuer`, so a provision call for a new issuer cannot rewrite or
    /// reactivate an existing account.
    pub fn provision_user(
        &mut self,
        subject: &str,
        issuer: &str,
        actor: &str,
    ) -> Result<i64, IdentityStoreError> {
        if subject.trim().is_empty() || issuer.trim().is_empty() {
            return Err(IdentityStoreError::Database(
                "subject and issuer are required".into(),
            ));
        }
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let user_id: i64 = tx
            .query_one(
                "INSERT INTO users (subject, issuer) VALUES ($1, $2) \
                 ON CONFLICT (issuer, subject) DO UPDATE \
                   SET deactivated_at = NULL, updated_at = now() \
                 RETURNING id",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?
            .get(0);
        audit(&mut tx, actor, "user.provision", subject, issuer)?;
        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(user_id)
    }

    /// Deactivates an account without deleting its audit history.
    ///
    /// The user is resolved by immutable (issuer, subject), so deactivating one
    /// identity leaves any other account with the same subject string active.
    pub fn deactivate_user(
        &mut self,
        subject: &str,
        issuer: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let row = tx
            .query_opt(
                "SELECT id FROM users WHERE subject = $1 AND issuer = $2 FOR UPDATE",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let Some(row) = row else {
            return Err(IdentityStoreError::UnknownUser(subject.to_string()));
        };
        let user_id: i64 = row.get(0);
        guard_admin_removal(&mut tx, user_id)?;
        tx.execute(
            "UPDATE users SET deactivated_at = now(), updated_at = now() WHERE id = $1",
            &[&user_id],
        )
        .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        audit(&mut tx, actor, "user.deactivate", subject, issuer)?;
        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }
}

/// Whether `user_id` holds an unrevoked admin role and is not deactivated.
fn is_active_admin(tx: &mut Transaction<'_>, user_id: i64) -> Result<bool, IdentityStoreError> {
    tx.query_one(
        "SELECT EXISTS ( \
             SELECT 1 FROM role_assignments r JOIN users u ON u.id = r.user_id \
              WHERE r.user_id = $1 AND r.role = 'admin' AND r.revoked_at IS NULL \
                AND u.deactivated_at IS NULL)",
        &[&user_id],
    )
    .map(|row| row.get(0))
    .map_err(|err| IdentityStoreError::Database(err.to_string()))
}

/// Fails with [`IdentityStoreError::LastAdmin`] when removing `user_id`'s admin
/// access (by revocation or deactivation) would leave no active administrator.
///
/// The advisory lock is taken before anything is read, so the target check and
/// the count both see a state no concurrent admin change can alter. Two removals
/// therefore cannot both observe two admins and leave zero. Removing a
/// deactivated user's dormant admin role is allowed: it does not reduce the
/// active-admin count.
fn guard_admin_removal(tx: &mut Transaction<'_>, user_id: i64) -> Result<(), IdentityStoreError> {
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtext('role:admin')::bigint)",
        &[],
    )
    .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
    if !is_active_admin(tx, user_id)? {
        return Ok(());
    }
    let active: i64 = tx
        .query_one(
            "SELECT count(*) FROM role_assignments r JOIN users u ON u.id = r.user_id \
              WHERE r.role = 'admin' AND r.revoked_at IS NULL AND u.deactivated_at IS NULL",
            &[],
        )
        .map_err(|err| IdentityStoreError::Database(err.to_string()))?
        .get(0);
    if active <= 1 {
        return Err(IdentityStoreError::LastAdmin);
    }
    Ok(())
}

/// Inserts the audit row inside the caller's transaction.
///
/// `audit_events` keys on `subject` only (a pre-existing schema limit). To keep
/// two accounts with the same subject string apart, the issuer is recorded in
/// the `detail` JSONB as `detail.issuer`. The audit row always commits with the
/// change it describes.
fn audit(
    tx: &mut Transaction<'_>,
    actor: &str,
    action: &str,
    subject: &str,
    issuer: &str,
) -> Result<(), IdentityStoreError> {
    tx.execute(
        "INSERT INTO audit_events (actor, action, subject, detail) \
         VALUES ($1, $2, $3, jsonb_build_object('issuer', $4::text))",
        &[&actor, &action, &subject, &issuer],
    )
    .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
    Ok(())
}

impl IdentityClient for PgIdentityClient {
    fn user_by_subject(
        &self,
        subject: &str,
        issuer: &str,
    ) -> Result<Option<UserRecord>, IdentityStoreError> {
        let row = self
            .client
            .borrow_mut()
            .query_opt(
                "SELECT id, subject, issuer FROM users WHERE subject = $1 AND issuer = $2 AND deactivated_at IS NULL",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(row.map(|row| UserRecord {
            id: row.get(0),
            subject: row.get(1),
            issuer: row.get(2),
        }))
    }

    fn effective_roles(&self, user_id: i64) -> Result<Vec<Role>, IdentityStoreError> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT r.role FROM role_assignments r JOIN users u ON u.id = r.user_id \
                  WHERE r.user_id = $1 AND r.revoked_at IS NULL AND u.deactivated_at IS NULL",
                &[&user_id],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let name: String = row.get(0);
                Role::parse(&name)
            })
            .collect())
    }

    fn effective_grants(&self, user_id: i64) -> Result<Vec<String>, IdentityStoreError> {
        let rows = self
            .client
            .borrow_mut()
            .query(
                "SELECT g.corpus FROM corpus_grants g JOIN users u ON u.id = g.user_id \
                  WHERE g.user_id = $1 AND g.revoked_at IS NULL AND u.deactivated_at IS NULL",
                &[&user_id],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    fn change_role(
        &mut self,
        subject: &str,
        issuer: &str,
        role: Role,
        actor: &str,
        grant: bool,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;

        let row = tx
            .query_opt(
                "SELECT id FROM users WHERE subject = $1 AND issuer = $2",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let Some(row) = row else {
            return Err(IdentityStoreError::UnknownUser(subject.to_string()));
        };
        let user_id: i64 = row.get(0);

        let action = if grant { "role.grant" } else { "role.revoke" };
        // Revoking a deactivated user's dormant admin role cannot reduce the
        // active-admin count, so only an active admin target is guarded.
        if !grant && role == Role::Admin {
            guard_admin_removal(&mut tx, user_id)?;
        }
        if grant {
            tx.execute(
                "INSERT INTO role_assignments (user_id, role, granted_by) \
                 SELECT $1, $2, $3 \
                 WHERE NOT EXISTS ( \
                     SELECT 1 FROM role_assignments \
                     WHERE user_id = $1 AND role = $2 AND revoked_at IS NULL \
                 )",
                &[&user_id, &role.as_str(), &actor],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        } else {
            tx.execute(
                "UPDATE role_assignments SET revoked_at = now() \
                 WHERE user_id = $1 AND role = $2 AND revoked_at IS NULL",
                &[&user_id, &role.as_str()],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        }

        audit(&mut tx, actor, action, subject, issuer)?;

        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }

    fn bootstrap_initial_admin(
        &mut self,
        subject: &str,
        issuer: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;

        let existing = tx
            .query_opt(
                "SELECT 1 FROM role_assignments WHERE role = 'admin' AND revoked_at IS NULL LIMIT 1",
                &[],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        if existing.is_some() {
            return Err(IdentityStoreError::AlreadyBootstrapped);
        }

        let user_id: i64 = tx
            .query_one(
                "INSERT INTO users (subject, issuer) VALUES ($1, $2) \
                 ON CONFLICT (issuer, subject) DO UPDATE SET updated_at = now() \
                 RETURNING id",
                &[&subject, &issuer],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?
            .get(0);

        tx.execute(
            "INSERT INTO role_assignments (user_id, role, granted_by) \
             VALUES ($1, 'admin', $2)",
            &[&user_id, &actor],
        )
        .map_err(|err| IdentityStoreError::Database(err.to_string()))?;

        audit(&mut tx, actor, "bootstrap.initial_admin", subject, issuer)?;

        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }
}

/// Convenience: wraps a connected client in the persistence layer.
pub fn connect(dsn: &str) -> Result<IdentityStore<PgIdentityClient>, IdentityStoreError> {
    let client = PgIdentityClient::connect(dsn)?;
    Ok(IdentityStore::new(client))
}
