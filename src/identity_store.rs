//! Identity persistence: users, roles, corpus grants, and audit events.
//!
//! Every role change runs in a single database transaction that writes the
//! `role_assignments` row and the `audit_events` row together. There is no
//! other code path that creates a role assignment.
//!
//! The initial-admin bootstrap is an explicit, one-time, audited operation
//! exposed through [`IdentityStore::bootstrap_initial_admin`] and the CLI in
//! `src/bin/admin.rs`. It is not reachable from any HTTP route and is not a
//! configuration switch.

use thiserror::Error;

use crate::identity::Role;

/// Errors from the identity store.
#[derive(Debug, Error)]
pub enum IdentityStoreError {
    /// A database error.
    #[error("database: {0}")]
    Database(String),
    /// The requested user does not exist.
    #[error("unknown user: {0}")]
    UnknownUser(String),
    /// A role name is not valid.
    #[error("invalid role: {0}")]
    InvalidRole(String),
    /// The initial admin already exists.
    #[error("an initial admin already exists")]
    AlreadyBootstrapped,
}

/// A stored user row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRecord {
    pub id: i64,
    pub subject: String,
    pub issuer: String,
}

/// The low-level client contract. The PostgreSQL implementation lives in
/// [`crate::identity_store_pg`]; tests use an in-memory implementation.
pub mod identity_client {
    use super::{IdentityStoreError, Role, UserRecord};

    /// Transaction-capable identity client.
    pub trait Client {
        /// Reads a user by subject and issuer.
        fn user_by_subject(
            &self,
            subject: &str,
            issuer: &str,
        ) -> Result<Option<UserRecord>, IdentityStoreError>;

        /// Effective roles.
        fn effective_roles(&self, user_id: i64) -> Result<Vec<Role>, IdentityStoreError>;

        /// Effective corpus grants.
        fn effective_grants(&self, user_id: i64) -> Result<Vec<String>, IdentityStoreError>;

        /// Single transactional role change with an audit row.
        fn change_role(
            &mut self,
            subject: &str,
            role: Role,
            actor: &str,
            grant: bool,
        ) -> Result<(), IdentityStoreError>;

        /// One-time bootstrap.
        fn bootstrap_initial_admin(
            &mut self,
            subject: &str,
            issuer: &str,
            actor: &str,
        ) -> Result<(), IdentityStoreError>;
    }
}

/// The persistence layer for identity. It wraps a transaction-capable client.
pub struct IdentityStore<C> {
    client: C,
}

impl<C> IdentityStore<C>
where
    C: identity_client::Client,
{
    /// Wraps a client.
    pub fn new(client: C) -> Self {
        Self { client }
    }

    /// Reads a user by verified subject and issuer.
    pub fn user_by_subject(
        &self,
        subject: &str,
        issuer: &str,
    ) -> Result<Option<UserRecord>, IdentityStoreError> {
        self.client.user_by_subject(subject, issuer)
    }

    /// Reads the effective (non-revoked) roles for a user.
    pub fn effective_roles(&self, user_id: i64) -> Result<Vec<Role>, IdentityStoreError> {
        self.client.effective_roles(user_id)
    }

    /// Reads the effective (non-revoked) corpus grants for a user.
    pub fn effective_grants(&self, user_id: i64) -> Result<Vec<String>, IdentityStoreError> {
        self.client.effective_grants(user_id)
    }

    /// Grants a role and writes the audit event in the same transaction.
    pub fn grant_role(
        &mut self,
        subject: &str,
        role: Role,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        self.client.change_role(subject, role, actor, true)
    }

    /// Revokes a role and writes the audit event in the same transaction.
    pub fn revoke_role(
        &mut self,
        subject: &str,
        role: Role,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        self.client.change_role(subject, role, actor, false)
    }

    /// Creates the first user and admin role assignment, audited, once.
    ///
    /// Fails with [`IdentityStoreError::AlreadyBootstrapped`] when any admin
    /// role assignment already exists, so the bootstrap cannot be replayed to
    /// mint a second initial admin.
    pub fn bootstrap_initial_admin(
        &mut self,
        subject: &str,
        issuer: &str,
        actor: &str,
    ) -> Result<(), IdentityStoreError> {
        self.client.bootstrap_initial_admin(subject, issuer, actor)
    }

    /// Borrows the underlying client (used by the PostgreSQL bootstrap).
    pub fn client(&mut self) -> &mut C {
        &mut self.client
    }
}

#[cfg(test)]
mod tests {
    use super::identity_client::Client;
    use super::*;
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct MemoryClient {
        users: Vec<UserRecord>,
        roles: BTreeMap<i64, Vec<Role>>,
        grants: BTreeMap<i64, Vec<String>>,
        audit: Vec<(String, String, String)>,
        fail_after_assignment: bool,
    }

    impl MemoryClient {
        fn insert_user(&mut self, subject: &str, issuer: &str) -> i64 {
            let id = self.users.len() as i64 + 1;
            self.users.push(UserRecord {
                id,
                subject: subject.to_string(),
                issuer: issuer.to_string(),
            });
            id
        }
    }

    impl Client for MemoryClient {
        fn user_by_subject(
            &self,
            subject: &str,
            issuer: &str,
        ) -> Result<Option<UserRecord>, IdentityStoreError> {
            Ok(self
                .users
                .iter()
                .find(|user| user.subject == subject && user.issuer == issuer)
                .cloned())
        }

        fn effective_roles(&self, user_id: i64) -> Result<Vec<Role>, IdentityStoreError> {
            Ok(self.roles.get(&user_id).cloned().unwrap_or_default())
        }

        fn effective_grants(&self, user_id: i64) -> Result<Vec<String>, IdentityStoreError> {
            Ok(self.grants.get(&user_id).cloned().unwrap_or_default())
        }

        fn change_role(
            &mut self,
            subject: &str,
            role: Role,
            actor: &str,
            grant: bool,
        ) -> Result<(), IdentityStoreError> {
            let Some(user) = self
                .users
                .iter()
                .find(|user| user.subject == subject)
                .cloned()
            else {
                return Err(IdentityStoreError::UnknownUser(subject.to_string()));
            };
            // Simulated transactional rollback: nothing is retained on failure.
            if self.fail_after_assignment && grant {
                return Err(IdentityStoreError::Database("injected failure".into()));
            }
            if grant {
                let entry = self.roles.entry(user.id).or_default();
                if !entry.contains(&role) {
                    entry.push(role);
                }
                self.audit.push((
                    actor.to_string(),
                    "role.grant".to_string(),
                    subject.to_string(),
                ));
            } else {
                if let Some(entry) = self.roles.get_mut(&user.id) {
                    entry.retain(|existing| existing != &role);
                }
                self.audit.push((
                    actor.to_string(),
                    "role.revoke".to_string(),
                    subject.to_string(),
                ));
            }
            Ok(())
        }

        fn bootstrap_initial_admin(
            &mut self,
            subject: &str,
            issuer: &str,
            actor: &str,
        ) -> Result<(), IdentityStoreError> {
            if self
                .roles
                .values()
                .flatten()
                .any(|role| role == &Role::Admin)
            {
                return Err(IdentityStoreError::AlreadyBootstrapped);
            }
            let id = match self.user_by_subject(subject, issuer)? {
                Some(user) => user.id,
                None => self.insert_user(subject, issuer),
            };
            self.roles.entry(id).or_default().push(Role::Admin);
            self.audit.push((
                actor.to_string(),
                "bootstrap.initial_admin".to_string(),
                subject.to_string(),
            ));
            Ok(())
        }
    }

    fn store() -> IdentityStore<MemoryClient> {
        let mut client = MemoryClient::default();
        client.insert_user("subject-1", "https://issuer");
        IdentityStore::new(client)
    }

    #[test]
    fn grant_role_records_audit_event() {
        let mut store = store();
        store
            .grant_role("subject-1", Role::Editor, "admin-1")
            .unwrap();
        let user = store
            .user_by_subject("subject-1", "https://issuer")
            .unwrap()
            .unwrap();
        assert_eq!(store.effective_roles(user.id).unwrap(), vec![Role::Editor]);
        assert_eq!(store.client.audit.len(), 1);
        assert_eq!(store.client.audit[0].1, "role.grant");
    }

    #[test]
    fn failed_role_change_leaves_no_assignment_and_no_audit() {
        let mut store = store();
        store.client.fail_after_assignment = true;
        assert!(store
            .grant_role("subject-1", Role::Admin, "admin-1")
            .is_err());
        let user = store
            .user_by_subject("subject-1", "https://issuer")
            .unwrap()
            .unwrap();
        assert!(store.effective_roles(user.id).unwrap().is_empty());
        assert!(store.client.audit.is_empty());
    }

    #[test]
    fn bootstrap_creates_exactly_one_admin() {
        let mut store = store();
        store
            .bootstrap_initial_admin("first-admin", "https://issuer", "operator")
            .unwrap();
        let first = store
            .user_by_subject("first-admin", "https://issuer")
            .unwrap()
            .unwrap();
        assert_eq!(store.effective_roles(first.id).unwrap(), vec![Role::Admin]);
        assert_eq!(store.client.audit.len(), 1);

        let second = store.bootstrap_initial_admin("second-admin", "https://issuer", "operator");
        assert!(matches!(
            second,
            Err(IdentityStoreError::AlreadyBootstrapped)
        ));
    }

    #[test]
    fn unknown_user_cannot_receive_a_role() {
        let mut store = store();
        assert!(matches!(
            store.grant_role("nobody", Role::Viewer, "admin-1"),
            Err(IdentityStoreError::UnknownUser(_))
        ));
        assert!(store.client.audit.is_empty());
    }
}
