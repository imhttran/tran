//! Deterministic contract tests for the USR-003D admin API surface.
//!
//! These tests do not start a server: they pin the authorization policy for the
//! two onboarding operations and the registered route paths. End-to-end browser
//! and HTTP behavior is covered by the USR-007 acceptance slice.

use rag_foundation::identity::{decide, Decision, Operation, Role, VerifiedIdentity};

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
