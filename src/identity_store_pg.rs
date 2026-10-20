//! PostgreSQL implementation of the identity client contract.
//!
//! [`PgIdentityClient`] wraps a synchronous `postgres::Client`. Each role
//! change and the initial-admin bootstrap run inside a single transaction that
//! writes the `role_assignments` row and the `audit_events` row together. If any
//! statement fails the transaction is rolled back, so neither row survives.
//!
//! There is no other code path that creates a role assignment.

use std::cell::RefCell;

use postgres::{Client, NoTls, Transaction};

use crate::identity::Role;
use crate::identity_store::identity_client::Client as IdentityClient;
use crate::identity_store::{IdentityStore, IdentityStoreError, UserRecord};

/// A transaction-capable identity client backed by PostgreSQL.
pub struct PgIdentityClient {
    client: RefCell<Client>,
}

impl PgIdentityClient {
    /// Connects to `dsn`.
    pub fn connect(dsn: &str) -> Result<Self, IdentityStoreError> {
        let client = Client::connect(dsn, NoTls)
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(Self {
            client: RefCell::new(client),
        })
    }

    /// Wraps an existing `postgres::Client`.
    pub fn from_client(client: Client) -> Self {
        Self {
            client: RefCell::new(client),
        }
    }

    /// Grants one corpus to an existing user and records the audit event.
    pub fn grant_corpus(
        &mut self,
        subject: &str,
        corpus: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let row = tx
            .query_opt("SELECT id FROM users WHERE subject = $1", &[&subject])
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
        audit(&mut tx, actor, "corpus.grant", subject)?;
        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }

    /// Revokes one corpus grant and records the audit event.
    pub fn revoke_corpus(
        &mut self,
        subject: &str,
        corpus: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let row = tx
            .query_opt("SELECT id FROM users WHERE subject = $1", &[&subject])
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
        audit(&mut tx, actor, "corpus.revoke", subject)?;
        tx.commit()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(())
    }
}

fn audit(
    tx: &mut Transaction<'_>,
    actor: &str,
    action: &str,
    subject: &str,
) -> Result<(), IdentityStoreError> {
    tx.execute(
        "INSERT INTO audit_events (actor, action, subject, detail) \
         VALUES ($1, $2, $3, '{}'::jsonb)",
        &[&actor, &action, &subject],
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
                "SELECT id, subject, issuer FROM users WHERE subject = $1 AND issuer = $2",
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
                "SELECT role FROM role_assignments WHERE user_id = $1 AND revoked_at IS NULL",
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
                "SELECT corpus FROM corpus_grants WHERE user_id = $1 AND revoked_at IS NULL",
                &[&user_id],
            )
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    fn change_role(
        &mut self,
        subject: &str,
        role: Role,
        actor: &str,
        grant: bool,
    ) -> Result<(), IdentityStoreError> {
        let mut guard = self.client.borrow_mut();
        let mut tx = guard
            .transaction()
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;

        let row = tx
            .query_opt("SELECT id FROM users WHERE subject = $1", &[&subject])
            .map_err(|err| IdentityStoreError::Database(err.to_string()))?;
        let Some(row) = row else {
            return Err(IdentityStoreError::UnknownUser(subject.to_string()));
        };
        let user_id: i64 = row.get(0);

        let action = if grant { "role.grant" } else { "role.revoke" };
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

        audit(&mut tx, actor, action, subject)?;

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
                 ON CONFLICT (subject) DO UPDATE SET updated_at = now() \
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

        audit(&mut tx, actor, "bootstrap.initial_admin", subject)?;

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
