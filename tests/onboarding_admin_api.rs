//! Deterministic contract tests for the USR-003D admin API surface.
//!
//! These tests do not start a server: they pin the authorization policy for the
//! two onboarding operations and the registered route paths. End-to-end browser
//! and HTTP behavior is covered by the USR-007 acceptance slice.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use rag_foundation::identity::{decide, Decision, Operation, Role, VerifiedIdentity};
use rag_foundation::onboarding::preview_candidates;

fn identity(roles: &[Role]) -> VerifiedIdentity {
    VerifiedIdentity::new(
        "synthetic-subject",
        "https://synthetic-issuer",
        roles.to_vec(),
        vec![],
    )
}

#[test]
fn onboarding_admin_operations_are_admin_only() {
    for operation in [
        Operation::OnboardingInvite,
        Operation::OnboardingLink,
        Operation::OnboardingDryRun,
        Operation::OnboardingBulkPrepare,
        Operation::OnboardingBulkSend,
    ] {
        assert_eq!(decide(None, operation), Decision::Unauthenticated);
        assert_eq!(
            decide(Some(&identity(&[Role::Viewer])), operation),
            Decision::Forbidden
        );
        assert_eq!(
            decide(Some(&identity(&[Role::Editor])), operation),
            Decision::Forbidden
        );
        assert_eq!(
            decide(Some(&identity(&[Role::Admin])), operation),
            Decision::Allow
        );
    }
}

#[test]
fn admin_onboarding_routes_are_registered() {
    let source = include_str!("../src/api.rs");
    for route in [
        "/api/admin/family-members/invitations",
        "/api/admin/family-members/invitations/{id}/resend",
        "/api/admin/family-members/invitations/{id}/revoke",
        "/api/admin/family-members/links",
        "/api/admin/family-members/links/{id}/revoke",
        "/api/admin/family-members/bulk/preview",
        "/api/admin/family-members/bulk/prepare",
        "/api/admin/family-members/bulk/recipients",
        "/api/admin/family-members/bulk/send",
        "/api/admin/family-members/bulk/batches/{id}",
    ] {
        assert!(source.contains(route), "missing route {route}");
    }
    assert!(source.contains("Operation::OnboardingInvite"));
    assert!(source.contains("Operation::OnboardingLink"));
    assert!(source.contains("Operation::OnboardingDryRun"));
    assert!(source.contains("Operation::OnboardingBulkPrepare"));
    assert!(source.contains("Operation::OnboardingBulkSend"));
}

#[test]
fn preview_rows_carry_the_name_and_only_a_masked_contact() {
    let Some(base) = genealogy_isolation::base_dsn_or_skip("onboarding admin preview") else {
        return;
    };
    let target = genealogy_isolation::create_isolated_target(&base, "onboarding_admin_preview");
    let mut client = target.connect();

    let with_contact: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name, birth_year)
             VALUES ('bot-102-with-contact', 'Ada Example', '1980')
             RETURNING person_id",
            &[],
        )
        .expect("insert person with contact")
        .get(0);
    client
        .execute(
            "INSERT INTO onboarding_recipient_contact
                 (person_id, email, verified_at, verified_by)
             VALUES ($1, 'ada@example.test', now(), 'synthetic-admin')",
            &[&with_contact],
        )
        .expect("insert verified contact");

    let without_contact: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name, birth_year)
             VALUES ('bot-102-without-contact', 'Bea Example', '1981')
             RETURNING person_id",
            &[],
        )
        .expect("insert person without contact")
        .get(0);
    assert!(without_contact > 0);

    let preview = preview_candidates(&mut client).expect("preview candidates");
    let with_row = preview
        .rows
        .iter()
        .find(|row| row.person_key == "bot-102-with-contact")
        .expect("row with contact");
    assert_eq!(with_row.display_name, "Ada Example");
    assert_eq!(
        with_row.contact_email_masked.as_deref(),
        Some("a•••@example.test")
    );

    let without_row = preview
        .rows
        .iter()
        .find(|row| row.person_key == "bot-102-without-contact")
        .expect("row without contact");
    assert_eq!(without_row.display_name, "Bea Example");
    assert_eq!(without_row.contact_email_masked, None);

    // The preview row model carries only the masked value; the full address is
    // never part of the serializable projection.
    let rendered = format!("{preview:?}");
    assert!(!rendered.contains("ada@example.test"));
}
