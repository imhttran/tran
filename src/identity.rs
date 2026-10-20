//! Verified identity, roles, and the central permission policy.
//!
//! Trust model:
//!
//! * The only source of identity is a verified OIDC token (see [`crate::oidc`]).
//! * Client-supplied headers (`X-Role`, `X-User`, ...) and client-supplied body
//!   fields are never read by this module.
//! * Every route is denied by default. A route reaches its handler only after
//!   the policy allows the verified identity.
//! * There is no environment flag or configuration switch that disables
//!   enforcement.

use serde::{Deserialize, Serialize};

/// A role in the MVP permission table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Viewer,
    Editor,
    Admin,
}

impl Role {
    /// Parses a role name. Unknown names are rejected; they never fall back to
    /// a default role.
    pub fn parse(name: &str) -> Option<Role> {
        match name.trim().to_ascii_lowercase().as_str() {
            "viewer" => Some(Role::Viewer),
            "editor" => Some(Role::Editor),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }

    /// Returns the wire name of the role.
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Admin => "admin",
        }
    }
}

/// A verified identity produced only by [`crate::oidc::validate_token`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedIdentity {
    subject: String,
    issuer: String,
    roles: Vec<Role>,
    corpus_grants: Vec<String>,
}

impl VerifiedIdentity {
    /// Builds a verified identity. Callers must have checked the token
    /// signature, issuer, audience, expiry, and subject first.
    pub fn new(
        subject: impl Into<String>,
        issuer: impl Into<String>,
        roles: Vec<Role>,
        corpus_grants: Vec<String>,
    ) -> Self {
        let mut roles = roles;
        roles.sort();
        roles.dedup();
        let mut corpus_grants = corpus_grants;
        corpus_grants.sort();
        corpus_grants.dedup();
        Self {
            subject: subject.into(),
            issuer: issuer.into(),
            roles,
            corpus_grants,
        }
    }

    /// The token subject (`sub`).
    pub fn subject(&self) -> &str {
        &self.subject
    }

    /// The issuer that verified the token.
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// The effective roles.
    pub fn roles(&self) -> &[Role] {
        &self.roles
    }

    /// The effective corpus grants.
    pub fn corpus_grants(&self) -> &[String] {
        &self.corpus_grants
    }

    /// Reports whether the identity holds a role at or above `role`.
    ///
    /// Roles are hierarchical in the MVP table: an admin also has every editor
    /// and viewer capability, and an editor also has every viewer capability.
    pub fn has_role(&self, role: Role) -> bool {
        self.roles.iter().any(|held| *held >= role)
    }

    /// The highest role the identity holds, if any.
    pub fn highest_role(&self) -> Option<Role> {
        self.roles.iter().copied().max()
    }
}

/// An operation that the API protects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    /// Ingestion staging plus embeddings and writes.
    Ingest,
    /// Retrieval plus model generation.
    Query,
    /// Reading document ingestion status.
    DocumentStatus,
    /// Reading the caller's account.
    Account,
    /// Changing a role assignment (transactional and audited).
    RoleChange,
    /// Reading authorized genealogy records and evidence.
    GenealogyRead,
    /// Drafting a Head of Family nomination with citations.
    GenealogyNominate,
    /// Head of Family approval APIs.
    HeadOfFamilyApproval,
}

impl Operation {
    /// The minimum role required for the operation, from the MVP permission
    /// table. `None` means public and non-sensitive (only `/api/health`).
    pub fn required_role(&self) -> Option<Role> {
        match self {
            Operation::Query | Operation::DocumentStatus | Operation::GenealogyRead => {
                Some(Role::Viewer)
            }
            Operation::Ingest | Operation::GenealogyNominate => Some(Role::Editor),
            Operation::Account | Operation::RoleChange | Operation::HeadOfFamilyApproval => {
                Some(Role::Admin)
            }
        }
    }

    /// Reports whether the operation is sensitive enough to be rate limited.
    pub fn is_rate_limited(&self) -> bool {
        matches!(
            self,
            Operation::Ingest
                | Operation::Query
                | Operation::RoleChange
                | Operation::GenealogyNominate
                | Operation::HeadOfFamilyApproval
        )
    }

    /// The rate-limit bucket name.
    pub fn rate_limit_key(&self) -> &'static str {
        match self {
            Operation::Ingest => "ingest",
            Operation::Query => "query",
            Operation::RoleChange => "role_change",
            Operation::GenealogyRead => "genealogy_read",
            Operation::GenealogyNominate => "genealogy_nominate",
            Operation::DocumentStatus => "document_status",
            Operation::Account => "account",
            Operation::HeadOfFamilyApproval => "head_of_family_approval",
        }
    }
}

/// The central policy decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    /// No verified identity: answer 401.
    Unauthenticated,
    /// Verified identity without the required role: answer 403.
    Forbidden,
}

/// The single place that turns a verified identity into an allow/deny decision.
pub fn decide(identity: Option<&VerifiedIdentity>, operation: Operation) -> Decision {
    // Every authenticated account may read its own account information. This is
    // deliberately not a role check: a newly provisioned user has no role yet.
    if operation == Operation::Account {
        return match identity {
            Some(_) => Decision::Allow,
            None => Decision::Unauthenticated,
        };
    }
    let required = match operation.required_role() {
        Some(role) => role,
        None => return Decision::Allow,
    };
    let Some(identity) = identity else {
        return Decision::Unauthenticated;
    };
    if identity.has_role(required) {
        Decision::Allow
    } else {
        Decision::Forbidden
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(roles: &[Role]) -> VerifiedIdentity {
        VerifiedIdentity::new("sub-1", "https://issuer", roles.to_vec(), vec![])
    }

    #[test]
    fn default_deny_without_identity() {
        for operation in [
            Operation::Ingest,
            Operation::Query,
            Operation::DocumentStatus,
            Operation::Account,
            Operation::RoleChange,
            Operation::HeadOfFamilyApproval,
        ] {
            assert_eq!(decide(None, operation), Decision::Unauthenticated);
        }
    }

    #[test]
    fn viewer_permission_table() {
        let viewer = identity(&[Role::Viewer]);
        assert_eq!(decide(Some(&viewer), Operation::Query), Decision::Allow);
        assert_eq!(
            decide(Some(&viewer), Operation::DocumentStatus),
            Decision::Allow
        );
        assert_eq!(
            decide(Some(&viewer), Operation::Ingest),
            Decision::Forbidden
        );
        assert_eq!(decide(Some(&viewer), Operation::Account), Decision::Allow);
        assert_eq!(
            decide(Some(&viewer), Operation::GenealogyRead),
            Decision::Allow
        );
        assert_eq!(
            decide(Some(&viewer), Operation::GenealogyNominate),
            Decision::Forbidden
        );
    }

    #[test]
    fn editor_permission_table() {
        let editor = identity(&[Role::Editor]);
        assert_eq!(decide(Some(&editor), Operation::Ingest), Decision::Allow);
        assert_eq!(decide(Some(&editor), Operation::Account), Decision::Allow);
        assert_eq!(
            decide(Some(&editor), Operation::GenealogyNominate),
            Decision::Allow
        );
        assert_eq!(
            decide(Some(&editor), Operation::RoleChange),
            Decision::Forbidden
        );
    }

    #[test]
    fn admin_permission_table() {
        let admin = identity(&[Role::Admin]);
        assert_eq!(decide(Some(&admin), Operation::RoleChange), Decision::Allow);
        assert_eq!(decide(Some(&admin), Operation::Account), Decision::Allow);
        assert_eq!(
            decide(Some(&admin), Operation::HeadOfFamilyApproval),
            Decision::Allow
        );
        assert_eq!(decide(Some(&admin), Operation::Query), Decision::Allow);
        assert_eq!(
            decide(Some(&admin), Operation::DocumentStatus),
            Decision::Allow
        );
        assert_eq!(decide(Some(&admin), Operation::Ingest), Decision::Allow);
    }

    #[test]
    fn authenticated_account_without_a_role_can_read_own_account() {
        let account = VerifiedIdentity::new("sub-1", "https://issuer", vec![], vec![]);
        assert_eq!(decide(Some(&account), Operation::Account), Decision::Allow);
        assert_eq!(
            decide(Some(&account), Operation::GenealogyRead),
            Decision::Forbidden
        );
    }

    #[test]
    fn unknown_role_name_is_rejected() {
        assert_eq!(Role::parse("root"), None);
        assert_eq!(Role::parse("viewer"), Some(Role::Viewer));
    }
}
