//! Shareable invitation links: redeem → claim → administrator decision.
//!
//! Uses a dedicated disposable genealogy target derived from
//! `GENEALOGY_DATABASE_URL` (see `tests/support/genealogy_isolation.rs`).
//! Accounts are synthetic opaque ids; no identity database is needed because
//! the genealogy side stores only the account id and fingerprint.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::time::Duration;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use postgres::Client;
use rag_foundation::onboarding::delivery::issue_deliverable_invitation;
use rag_foundation::onboarding::{
    claim_invitation, decide_claim, identity_fingerprint, link_for_account, link_for_person,
    pending_claims, IdentityAccount, InvitationError, LinkDecision, LinkError,
};

const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

fn prepare(label: &str) -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    let target = create_isolated_target(&base, label);
    target.apply_migrations();
    Some(target)
}

fn person(client: &mut Client, key: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
            &[&key, &format!("Synthetic {key}")],
        )
        .expect("insert person")
        .get(0)
}

fn account(id: i64) -> IdentityAccount {
    IdentityAccount {
        account_id: id,
        identity_fingerprint: identity_fingerprint("https://synthetic.test", &format!("sub-{id}")),
    }
}

fn share(client: &mut Client, person_id: i64) -> String {
    issue_deliverable_invitation(client, person_id, WEEK, "admin")
        .expect("issue")
        .expect("token")
        .1
}

#[test]
fn a_redeemed_link_waits_for_confirmation_then_links_once() {
    let Some(target) = prepare("invitation_claims_confirm") else {
        return;
    };
    let mut client = target.connect();
    let person_id = person(&mut client, "synthetic-claim");
    let token = share(&mut client, person_id);

    // Redeeming records a claim, not a link.
    claim_invitation(&mut client, &token, &account(901), "  Synthetic Member ").expect("claim");
    assert!(link_for_person(&mut client, person_id).unwrap().is_none());
    let claims = pending_claims(&mut client).unwrap();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].claimed_name, "Synthetic Member");
    assert_eq!(claims[0].account_id, 901);
    assert_eq!(claims[0].person_key, "synthetic-claim");

    // The token is single-use.
    assert!(matches!(
        claim_invitation(&mut client, &token, &account(902), "Someone Else").unwrap_err(),
        InvitationError::AlreadyAccepted
    ));

    // Confirming writes the reviewed link; the claim cannot be decided twice.
    let record = decide_claim(
        &mut client,
        claims[0].invitation_id,
        LinkDecision::Approve,
        "admin",
    )
    .expect("confirm")
    .expect("link");
    assert_eq!(record.person_id, person_id);
    assert_eq!(record.account_id, 901);
    assert!(pending_claims(&mut client).unwrap().is_empty());
    assert_eq!(
        decide_claim(
            &mut client,
            claims[0].invitation_id,
            LinkDecision::Approve,
            "admin"
        )
        .unwrap_err(),
        LinkError::UnknownLink
    );
    let audit: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_link_audit WHERE person_id = $1 AND decision = 'linked'",
            &[&person_id],
        )
        .unwrap()
        .get(0);
    assert_eq!(audit, 1);

    drop(client);
    target.cleanup();
}

#[test]
fn a_rejected_claim_writes_no_link_and_leaves_the_person_invitable() {
    let Some(target) = prepare("invitation_claims_reject") else {
        return;
    };
    let mut client = target.connect();
    let person_id = person(&mut client, "synthetic-reject");
    let token = share(&mut client, person_id);
    let invitation_id =
        claim_invitation(&mut client, &token, &account(903), "Unknown Holder").unwrap();

    assert!(
        decide_claim(&mut client, invitation_id, LinkDecision::Reject, "admin")
            .unwrap()
            .is_none()
    );
    assert!(link_for_person(&mut client, person_id).unwrap().is_none());
    assert!(
        link_for_account(&mut client, 903, &account(903).identity_fingerprint)
            .unwrap()
            .is_none()
    );
    assert!(pending_claims(&mut client).unwrap().is_empty());

    // A fresh link can be shared and claimed by the right member.
    let again = share(&mut client, person_id);
    claim_invitation(&mut client, &again, &account(904), "Right Member").expect("new claim");

    drop(client);
    target.cleanup();
}

#[test]
fn already_linked_people_or_accounts_and_bad_input_do_not_burn_the_token() {
    let Some(target) = prepare("invitation_claims_guards") else {
        return;
    };
    let mut client = target.connect();
    let linked_person = person(&mut client, "synthetic-linked");
    let open_person = person(&mut client, "synthetic-open");

    // Link account 905 to the first person through a confirmed claim.
    let first = share(&mut client, linked_person);
    let id = claim_invitation(&mut client, &first, &account(905), "Linked Member").unwrap();
    decide_claim(&mut client, id, LinkDecision::Approve, "admin").unwrap();

    let token = share(&mut client, open_person);
    // A blank or overlong name is refused before the token is consumed.
    assert!(matches!(
        claim_invitation(&mut client, &token, &account(906), "   ").unwrap_err(),
        InvitationError::InvalidName
    ));
    assert!(matches!(
        claim_invitation(&mut client, &token, &account(906), &"x".repeat(101)).unwrap_err(),
        InvitationError::InvalidName
    ));
    // An account that is already linked cannot claim a second person.
    assert!(matches!(
        claim_invitation(&mut client, &token, &account(905), "Linked Member").unwrap_err(),
        InvitationError::AlreadyLinked
    ));
    // A wrong token is not a claim.
    assert!(matches!(
        claim_invitation(&mut client, "not-a-real-token", &account(906), "Member").unwrap_err(),
        InvitationError::InvalidToken
    ));
    // The token still works for the right account.
    claim_invitation(&mut client, &token, &account(906), "Open Member").expect("claim");

    drop(client);
    target.cleanup();
}
