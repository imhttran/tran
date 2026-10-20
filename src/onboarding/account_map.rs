//! Cross-database identity mapping for onboarding links.
//!
//! The identity database (`DATABASE_URL`) is the authority for the immutable
//! `(issuer, subject)` pair and owns `users.id`. The genealogy database owns the
//! person and stores only the opaque account id plus a one-way fingerprint. There
//! is no cross-database foreign key: resolution happens here, first, and a link
//! is written afterwards. A failure between the two steps leaves no link, and a
//! repeated call is safe.
//!
//! A subject is never resolved on its own when the issuer is missing or
//! ambiguous. A subject present under more than one issuer is rejected.

use std::fmt::Write as _;

use ring::digest;
use thiserror::Error;

/// An active identity resolved from the identity database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityAccount {
    /// The opaque identity database id. The genealogy database stores this value
    /// and never a raw subject.
    pub account_id: i64,
    /// The lowercase hex SHA-256 fingerprint of `issuer\nsubject`.
    pub identity_fingerprint: String,
}

/// Errors raised while resolving an identity.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum AccountMapError {
    /// The issuer or subject is blank.
    #[error("identity issuer and subject are required")]
    InvalidIdentity,
    /// No account exists for the exact pair.
    #[error("identity is not registered")]
    UnknownIdentity,
    /// The account exists but is deactivated.
    #[error("identity is deactivated")]
    DeactivatedIdentity,
    /// The subject exists under more than one issuer and the issuer was not
    /// supplied, so the identity must not be guessed.
    #[error("identity subject is ambiguous across issuers")]
    AmbiguousIdentity,
    /// The identity database query failed.
    #[error("identity database operation failed: {0}")]
    Database(String),
}

/// Returns the lowercase hex SHA-256 fingerprint of `issuer\nsubject`.
///
/// The newline keeps the two components unambiguous. The result is one-way and
/// is the only identity value the genealogy database stores.
pub fn identity_fingerprint(issuer: &str, subject: &str) -> String {
    let input = format!("{}\n{}", issuer.trim(), subject.trim());
    let output = digest::digest(&digest::SHA256, input.as_bytes());
    let mut hex = String::with_capacity(64);
    for byte in output.as_ref() {
        write!(hex, "{byte:02x}").expect("write to String");
    }
    hex
}

/// Resolves an exact `(issuer, subject)` to an active identity.
///
/// A blank subject is always invalid. A blank issuer is never guessed: a subject
/// matching zero rows is `UnknownIdentity`, and a subject matching more than one
/// issuer is `AmbiguousIdentity`. A deactivated account is rejected even though
/// the row exists.
pub fn resolve_account(
    client: &mut postgres::Client,
    issuer: &str,
    subject: &str,
) -> Result<IdentityAccount, AccountMapError> {
    let issuer = issuer.trim();
    let subject = subject.trim();
    if subject.is_empty() {
        return Err(AccountMapError::InvalidIdentity);
    }

    if issuer.is_empty() {
        let count: i64 = client
            .query_one("SELECT count(*) FROM users WHERE subject = $1", &[&subject])
            .map_err(|err| AccountMapError::Database(err.to_string()))?
            .get(0);
        return match count {
            0 => Err(AccountMapError::UnknownIdentity),
            1 => Err(AccountMapError::InvalidIdentity),
            _ => Err(AccountMapError::AmbiguousIdentity),
        };
    }

    let row = client
        .query_opt(
            "SELECT id, (deactivated_at IS NOT NULL) \
               FROM users WHERE issuer = $1 AND subject = $2",
            &[&issuer, &subject],
        )
        .map_err(|err| AccountMapError::Database(err.to_string()))?;

    let Some(row) = row else {
        // The exact pair is absent. Distinguish an ambiguous subject so the
        // caller never falls back to a subject-only match.
        let count: i64 = client
            .query_one("SELECT count(*) FROM users WHERE subject = $1", &[&subject])
            .map_err(|err| AccountMapError::Database(err.to_string()))?
            .get(0);
        return if count > 1 {
            Err(AccountMapError::AmbiguousIdentity)
        } else {
            Err(AccountMapError::UnknownIdentity)
        };
    };

    let deactivated: bool = row.get(1);
    if deactivated {
        return Err(AccountMapError::DeactivatedIdentity);
    }

    Ok(IdentityAccount {
        account_id: row.get(0),
        identity_fingerprint: identity_fingerprint(issuer, subject),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_pair_aware() {
        let first = identity_fingerprint("https://issuer", "subject-1");
        assert_eq!(
            first,
            identity_fingerprint(" https://issuer ", " subject-1 ")
        );
        assert_eq!(first.len(), 64);
        assert_ne!(first, identity_fingerprint("https://other", "subject-1"));
        assert_ne!(first, identity_fingerprint("https://issuer", "subject-2"));
    }

    #[test]
    fn fingerprint_is_lowercase_hex() {
        let value = identity_fingerprint("i", "s");
        assert!(value
            .chars()
            .all(|ch| ch.is_ascii_digit() || ('a'..='f').contains(&ch)));
    }
}
