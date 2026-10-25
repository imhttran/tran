//! The single definition of a *current* genealogy assertion.
//!
//! Every current-state read — the family tree, name search, the archive person
//! answer, the person detail, the family profile, and the Head of Family
//! eligibility input — uses one of the two fragments below, so those paths
//! cannot drift apart.
//!
//! An assertion is current when both hold:
//!
//! 1. no authority decision superseded it (`superseded_at IS NULL`);
//! 2. its supporting evidence has not been withdrawn.
//!
//! Rule 2 is what makes a removed document stop supporting answers and badges.
//! It is deliberately written as "has live evidence **or** never had evidence":
//! only an explicit withdrawal removes support, so an assertion that no read
//! ever attached evidence to keeps behaving exactly as before.
//!
//! Withdrawal never restores an older assertion and never changes
//! `review_status`. The authority decision that relied on the withdrawn source
//! is preserved and reported as needing attention instead.

/// SQL predicate over the `a` alias of the `assertion` table: the row once had
/// evidence and all of it has been withdrawn.
macro_rules! assertion_unsupported {
    () => {
        "EXISTS (
           SELECT 1 FROM assertion_evidence any_evidence
            WHERE any_evidence.assertion_id = a.assertion_id)
         AND NOT EXISTS (
           SELECT 1 FROM assertion_evidence live_evidence
            WHERE live_evidence.assertion_id = a.assertion_id
              AND live_evidence.withdrawn_at IS NULL)"
    };
}

/// SQL predicate over the `a` alias of the `assertion` table: a current
/// assertion whose supporting evidence still exists.
///
/// Callers must alias `assertion` as `a`. The predicate is a plain expression,
/// so it composes with each read's own `WHERE` clause.
pub const CURRENT_ASSERTION: &str = concat!(
    "a.superseded_at IS NULL AND NOT (",
    assertion_unsupported!(),
    ")"
);

/// SQL predicate over the `a` alias of the `assertion` table: the assertion lost
/// every supporting citation to a withdrawn source.
///
/// Used by the authorized history view to explain why an active decision needs
/// attention, and never by a current-state read.
pub const ASSERTION_UNSUPPORTED: &str = assertion_unsupported!();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_predicates_cover_both_rules() {
        // Guards the contract the reads depend on: supersession plus withdrawn
        // support. A change here changes every current view at once.
        assert!(CURRENT_ASSERTION.contains("a.superseded_at IS NULL"));
        assert!(CURRENT_ASSERTION.contains("withdrawn_at IS NULL"));
        assert!(CURRENT_ASSERTION.contains(ASSERTION_UNSUPPORTED));
    }

    #[test]
    fn an_assertion_that_never_had_evidence_is_not_unsupported() {
        // The unsupported predicate requires at least one evidence row, so an
        // assertion created without evidence stays current.
        assert!(ASSERTION_UNSUPPORTED.contains("EXISTS"));
        assert!(ASSERTION_UNSUPPORTED.contains("any_evidence"));
    }
}
