//! Integration test: invitation welcome, activation, and reviewed linking.
//!
//! Every scenario runs against a disposable genealogy database created through
//! `tests/support/genealogy_isolation.rs`. The suite proves token validation,
//! acceptance, the pending review request, the administrative decision, expiry,
//! revocation, reuse, and the wrong-person rules. Token possession never creates
//! a person link by itself.

use std::time::Duration;

use postgres::Client;
use rag_foundation::onboarding::{
    accept_invitation, create, decide_link_request, identity_fingerprint, link_for_person,
    link_reviewed, pending_link_requests, preview_invitation, ActivationError, ActivationState,
    IdentityAccount, LinkDecision, LinkRequestState, LinkState,
};

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn_or_skip, create_isolated_target, IsolatedTarget};

struct Fixture {
    target: IsolatedTarget,
    prefix: String,
}

impl Fixture {
    fn client(&self) -> Client {
        self.target.connect()
    }

    fn account(&self, label: &str, account_id: i64) -> IdentityAccount {
        let issuer = format!("{}-{label}-issuer", self.prefix);
        let subject = format!("{}-{label}-subject", self.prefix);
        IdentityAccount {
            account_id,
            identity_fingerprint: identity_fingerprint(&issuer, &subject),
        }
    }
}

fn build_fixture(label: &str) -> Option<Fixture> {
    let base = base_dsn_or_skip(label)?;
    let target = create_isolated_target(&base, label);
    let prefix = format!("synthetic-pir006-{}-{label}", std::process::id());
    Some(Fixture { target, prefix })
}

fn insert_person(client: &mut Client, key: &str, name: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
            &[&key, &name],
        )
        .expect("insert synthetic person")
        .get(0)
}

fn count_links_for_person(client: &mut Client, person_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person_id],
        )
        .expect("count person links")
        .get(0)
}

fn count_links_for_account(client: &mut Client, account_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_person_account_link WHERE account_id = $1",
            &[&account_id],
        )
        .expect("count account links")
        .get(0)
}

fn invitation_status(client: &mut Client, invitation_id: i64) -> String {
    client
        .query_one(
            "SELECT status FROM onboarding_invitation WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .expect("read invitation status")
        .get(0)
}

fn request_count(client: &mut Client, invitation_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_link_request WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .expect("count link requests")
        .get(0)
}

fn request_status(client: &mut Client, invitation_id: i64) -> String {
    client
        .query_one(
            "SELECT status FROM onboarding_link_request WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .expect("read link request status")
        .get(0)
}

fn issue(client: &mut Client, person_id: i64) -> (i64, String) {
    let issued = create(
        client,
        person_id,
        Duration::from_secs(3600),
        "synthetic-admin",
    )
    .expect("issue invitation");
    (
        issued.invitation_id,
        issued.token.expect("a new invitation carries a token"),
    )
}

#[test]
fn an_invitation_is_requested_reviewed_and_then_linked() {
    let Some(fixture) = build_fixture("pir006-valid") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let account = fixture.account("member", 11);
    let (invitation_id, token) = issue(&mut client, person_id);

    let preview = preview_invitation(&mut client, &token).expect("preview");
    assert_eq!(preview.state, ActivationState::Pending);
    assert_eq!(preview.person_key, person_key);
    assert_eq!(preview.display_name, "Synthetic Person");
    assert!(!preview.person_linked);
    assert_eq!(preview.request_state, LinkRequestState::None);
    // A preview writes nothing.
    assert_eq!(invitation_status(&mut client, invitation_id), "pending");
    assert_eq!(count_links_for_person(&mut client, person_id), 0);

    let outcome = accept_invitation(&mut client, &token, &account).expect("accept");
    assert_eq!(outcome.state, ActivationState::Accepted);
    assert_eq!(outcome.link_state, LinkState::PendingReview);
    assert_eq!(invitation_status(&mut client, invitation_id), "accepted");
    // Possessing the token records a request; it never approves the link.
    assert_eq!(count_links_for_person(&mut client, person_id), 0);
    assert_eq!(request_count(&mut client, invitation_id), 1);
    assert_eq!(request_status(&mut client, invitation_id), "pending");

    let preview = preview_invitation(&mut client, &token).expect("preview after accept");
    assert_eq!(preview.state, ActivationState::Accepted);
    assert_eq!(preview.request_state, LinkRequestState::Pending);

    // A repeated acceptance is idempotent: one request, no second row.
    let again = accept_invitation(&mut client, &token, &account).expect("repeat accept");
    assert_eq!(again.link_state, LinkState::PendingReview);
    assert_eq!(request_count(&mut client, invitation_id), 1);

    let pending = pending_link_requests(&mut client).expect("list requests");
    let request = pending
        .iter()
        .find(|request| request.invitation_id == invitation_id)
        .expect("the request is listed");
    assert_eq!(request.person_key, person_key);
    assert_eq!(request.account_id, account.account_id);

    let decided = decide_link_request(
        &mut client,
        request.request_id,
        LinkDecision::Approve,
        "admin",
    )
    .expect("approve");
    assert!(decided.linked);
    assert!(!decided.already_decided);
    assert_eq!(count_links_for_person(&mut client, person_id), 1);
    assert_eq!(count_links_for_account(&mut client, account.account_id), 1);
    assert_eq!(request_status(&mut client, invitation_id), "approved");

    let preview = preview_invitation(&mut client, &token).expect("preview after review");
    assert!(preview.person_linked);
    assert_eq!(preview.request_state, LinkRequestState::Approved);

    // A repeated approval is reported as already decided and creates no link.
    let again = decide_link_request(
        &mut client,
        request.request_id,
        LinkDecision::Approve,
        "admin",
    )
    .expect("repeat approve");
    assert!(again.already_decided);
    assert_eq!(count_links_for_person(&mut client, person_id), 1);
}

#[test]
fn an_administrator_can_reject_a_request_and_it_stays_rejected() {
    let Some(fixture) = build_fixture("pir006-reject") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let account = fixture.account("member", 12);
    let (invitation_id, token) = issue(&mut client, person_id);

    accept_invitation(&mut client, &token, &account).expect("accept");
    let request_id: i64 = client
        .query_one(
            "SELECT request_id FROM onboarding_link_request WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .expect("request id")
        .get(0);

    let decided = decide_link_request(&mut client, request_id, LinkDecision::Reject, "admin")
        .expect("reject");
    assert!(!decided.linked);
    assert_eq!(request_status(&mut client, invitation_id), "rejected");
    assert_eq!(count_links_for_person(&mut client, person_id), 0);

    let retry = accept_invitation(&mut client, &token, &account);
    assert!(matches!(retry, Err(ActivationError::LinkRejected)));
    assert_eq!(count_links_for_person(&mut client, person_id), 0);

    // An approval after a rejection is refused, not silently reversed.
    let approval = decide_link_request(&mut client, request_id, LinkDecision::Approve, "admin");
    assert!(matches!(
        approval,
        Err(ActivationError::LinkRequestAlreadyDecided)
    ));
    assert_eq!(count_links_for_person(&mut client, person_id), 0);
}

#[test]
fn an_expired_invitation_is_refused_and_marked() {
    let Some(fixture) = build_fixture("pir006-expired") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let account = fixture.account("member", 13);
    let (invitation_id, token) = issue(&mut client, person_id);
    client
        .execute(
            "UPDATE onboarding_invitation SET expires_at = now() - interval '1 hour'
              WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .expect("expire invitation");

    let preview = preview_invitation(&mut client, &token).expect("preview");
    assert_eq!(preview.state, ActivationState::Expired);
    // The preview is read-only.
    assert_eq!(invitation_status(&mut client, invitation_id), "pending");

    let result = accept_invitation(&mut client, &token, &account);
    assert!(matches!(result, Err(ActivationError::Expired)));
    assert_eq!(invitation_status(&mut client, invitation_id), "expired");
    assert_eq!(count_links_for_person(&mut client, person_id), 0);
    assert_eq!(request_count(&mut client, invitation_id), 0);
}

#[test]
fn a_revoked_invitation_is_refused() {
    let Some(fixture) = build_fixture("pir006-revoked") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let account = fixture.account("member", 14);
    let (invitation_id, token) = issue(&mut client, person_id);
    client
        .execute(
            "UPDATE onboarding_invitation SET status = 'revoked' WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .expect("revoke invitation");

    let preview = preview_invitation(&mut client, &token).expect("preview");
    assert_eq!(preview.state, ActivationState::Revoked);

    let result = accept_invitation(&mut client, &token, &account);
    assert!(matches!(result, Err(ActivationError::Revoked)));
    assert_eq!(count_links_for_person(&mut client, person_id), 0);
}

#[test]
fn an_accepted_invitation_cannot_be_reused_by_another_account() {
    let Some(fixture) = build_fixture("pir006-accepted") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let first = fixture.account("first", 15);
    let second = fixture.account("second", 16);
    let (invitation_id, token) = issue(&mut client, person_id);

    accept_invitation(&mut client, &token, &first).expect("first acceptance");

    let result = accept_invitation(&mut client, &token, &second);
    assert!(matches!(result, Err(ActivationError::AlreadyAccepted)));
    assert_eq!(request_count(&mut client, invitation_id), 1);
    assert_eq!(count_links_for_account(&mut client, second.account_id), 0);
}

#[test]
fn an_unknown_or_blank_token_is_refused() {
    let Some(fixture) = build_fixture("pir006-invalid") else {
        return;
    };
    let mut client = fixture.client();
    let account = fixture.account("member", 17);

    let unknown = "0".repeat(64);
    let preview = preview_invitation(&mut client, &unknown);
    assert!(matches!(preview, Err(ActivationError::InvalidToken)));
    let result = accept_invitation(&mut client, &unknown, &account);
    assert!(matches!(result, Err(ActivationError::InvalidToken)));

    let preview = preview_invitation(&mut client, "   ");
    assert!(matches!(preview, Err(ActivationError::InvalidToken)));

    let unknown_request = decide_link_request(&mut client, 999_999, LinkDecision::Approve, "admin");
    assert!(matches!(
        unknown_request,
        Err(ActivationError::UnknownLinkRequest)
    ));
}

#[test]
fn an_account_linked_to_another_person_is_refused() {
    let Some(fixture) = build_fixture("pir006-account-elsewhere") else {
        return;
    };
    let mut client = fixture.client();
    let other_key = format!("{}-other", fixture.prefix);
    let target_key = format!("{}-target", fixture.prefix);
    let other_id = insert_person(&mut client, &other_key, "Synthetic Other");
    let target_id = insert_person(&mut client, &target_key, "Synthetic Target");
    let account = fixture.account("member", 18);

    link_reviewed(
        &mut client,
        other_id,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    )
    .expect("existing reviewed link");
    let (invitation_id, token) = issue(&mut client, target_id);

    let result = accept_invitation(&mut client, &token, &account);
    assert!(matches!(
        result,
        Err(ActivationError::AccountLinkedElsewhere)
    ));
    assert_eq!(count_links_for_person(&mut client, other_id), 1);
    assert_eq!(count_links_for_person(&mut client, target_id), 0);
    assert_eq!(invitation_status(&mut client, invitation_id), "pending");
    assert_eq!(request_count(&mut client, invitation_id), 0);
}

#[test]
fn a_person_linked_to_another_account_is_never_reclaimed() {
    let Some(fixture) = build_fixture("pir006-person-linked") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let owner = fixture.account("owner", 19);
    let stranger = fixture.account("stranger", 20);

    link_reviewed(
        &mut client,
        person_id,
        &owner,
        LinkDecision::Approve,
        "synthetic-reviewer",
    )
    .expect("existing reviewed link");
    let (invitation_id, token) = issue(&mut client, person_id);

    let result = accept_invitation(&mut client, &token, &stranger);
    assert!(matches!(result, Err(ActivationError::PersonAlreadyLinked)));
    assert_eq!(count_links_for_person(&mut client, person_id), 1);
    assert_eq!(count_links_for_account(&mut client, stranger.account_id), 0);
    assert_eq!(invitation_status(&mut client, invitation_id), "pending");
    assert_eq!(request_count(&mut client, invitation_id), 0);
}

#[test]
fn an_account_already_linked_to_the_invited_person_is_idempotent() {
    let Some(fixture) = build_fixture("pir006-already-linked") else {
        return;
    };
    let mut client = fixture.client();
    let person_key = format!("{}-target", fixture.prefix);
    let person_id = insert_person(&mut client, &person_key, "Synthetic Person");
    let account = fixture.account("member", 21);

    link_reviewed(
        &mut client,
        person_id,
        &account,
        LinkDecision::Approve,
        "synthetic-reviewer",
    )
    .expect("existing reviewed link");
    let (invitation_id, token) = issue(&mut client, person_id);

    let outcome = accept_invitation(&mut client, &token, &account).expect("accept");
    assert_eq!(outcome.link_state, LinkState::Linked);
    assert_eq!(invitation_status(&mut client, invitation_id), "accepted");
    assert_eq!(count_links_for_person(&mut client, person_id), 1);
    let existing = link_for_person(&mut client, person_id)
        .expect("read link")
        .expect("link exists");
    assert_eq!(existing.account_id, account.account_id);
}
