//! Deterministic contract tests for the FAM-003 document API surface.
//!
//! No server is started: these tests pin the role policy and registered routes.
//! The persistence and boundary behavior is covered by
//! `tests/document_lifecycle.rs`.

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
fn document_operations_are_role_scoped() {
    for operation in [Operation::DocumentSubmit, Operation::DocumentRead] {
        assert_eq!(decide(None, operation), Decision::Unauthenticated);
        assert_eq!(
            decide(Some(&identity(&[Role::Viewer])), operation),
            Decision::Forbidden
        );
        assert_eq!(
            decide(Some(&identity(&[Role::Editor])), operation),
            Decision::Allow
        );
        assert_eq!(
            decide(Some(&identity(&[Role::Admin])), operation),
            Decision::Allow
        );
    }
    for operation in [Operation::DocumentReview, Operation::DocumentProcess] {
        assert_eq!(decide(None, operation), Decision::Unauthenticated);
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
fn document_routes_are_registered_and_ingest_is_reviewed() {
    let source = include_str!("../src/api.rs");
    for route in [
        "/api/documents/submissions",
        "/api/documents/submissions/{id}",
        "/api/documents/submissions/{id}/content",
        "/api/documents/submissions/{id}/review",
        "/api/documents/submissions/{id}/scope",
        "/api/documents/submissions/{id}/remove",
        "/api/documents/submissions/{id}/process",
        "/api/documents/legacy",
        "/api/documents/legacy/{id}/promote",
    ] {
        assert!(source.contains(route), "missing route {route}");
    }
    // The browser/API ingest route must enter the reviewed submission path,
    // never the direct chunk writer.
    assert!(source.contains("post(submit_document)"));
    assert!(!source.contains("post(ingest)"));
    assert!(source.contains("Operation::DocumentSubmit"));
    assert!(source.contains("Operation::DocumentReview"));
    assert!(source.contains("Operation::DocumentProcess"));
}
