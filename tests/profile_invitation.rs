//! PIR-003 integration test: restricted contact drafts and invitation preparation.
//!
//! Every scenario runs against a disposable genealogy database created through
//! `tests/support/genealogy_isolation.rs`. Each synthetic person, appointment,
//! link, and account uses a unique prefix and `example.test` addresses. The
//! suite never asserts on, or prints, an unmasked address.

use postgres::Client;
use rag_foundation::identity::{Role, VerifiedIdentity};
use rag_foundation::onboarding::account_map::IdentityAccount;
use rag_foundation::onboarding::candidate::Classification;
use rag_foundation::onboarding::{
    identity_fingerprint, prepare_invitation, read_invitation_state, save_contact_draft,
    InvitationState, PrepareReview, ProfileInvitationError,
};

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn_or_skip, create_isolated_target, IsolatedTarget};

const CALLER_BRANCH: &str = "branch-3";
const TARGET_BRANCH: &str = "branch-3";
const OTHER_BRANCH: &str = "branch-2";

struct Fixture {
    target: IsolatedTarget,
    prefix: String,
    target_person_key: String,
    target_person_id: i64,
    identity: VerifiedIdentity,
    account: IdentityAccount,
}

impl Fixture {
    fn client(&self) -> Client {
        self.target.connect()
    }

    fn email(&self) -> String {
        format!("{}@example.test", self.prefix)
    }
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

/// Insert one verified assertion with its own original-source evidence.
fn verified_assertion(
    client: &mut Client,
    prefix: &str,
    person_id: i64,
    kind: &str,
    key: &str,
    value: &str,
) {
    let relative_path = format!("{prefix}/{person_id}-{kind}-{key}.md");
    let source_id: i64 = client
        .query_one(
            "INSERT INTO source_document (relative_path, sha256, source_kind)
             VALUES ($1, repeat('a', 64), 'original')
             RETURNING source_document_id",
            &[&relative_path],
        )
        .expect("insert synthetic source")
        .get(0);
    let evidence_id: i64 = client
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page, locator)
             VALUES ($1, 1, 'p.1') RETURNING source_evidence_id",
            &[&source_id],
        )
        .expect("insert synthetic evidence")
        .get(0);
    let assertion_id: i64 = client
        .query_one(
            "INSERT INTO assertion
                 (person_id, assertion_kind, assertion_key, value_text, review_status)
             VALUES ($1, $2, $3, $4, 'unverified')
             RETURNING assertion_id",
            &[&person_id, &kind, &key, &value],
        )
        .expect("insert synthetic assertion")
        .get(0);
    client
        .execute(
            "INSERT INTO assertion_evidence
                 (assertion_id, source_evidence_id, document_ref, page)
             VALUES ($1, $2, $3, 1)",
            &[&assertion_id, &evidence_id, &relative_path],
        )
        .expect("insert synthetic citation");
    client
        .execute(
            "UPDATE assertion
                SET review_status = 'verified',
                    reviewed_by = 'synthetic-reviewer',
                    reviewed_at = now()
              WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("verify synthetic assertion");
}

fn seed_eligible_person(
    client: &mut Client,
    prefix: &str,
    key: &str,
    name: &str,
    branch: &str,
) -> i64 {
    let person_id = insert_person(client, key, name);
    verified_assertion(client, prefix, person_id, "branch_claim", "branch", branch);
    verified_assertion(client, prefix, person_id, "gender", "gender", "male");
    verified_assertion(client, prefix, person_id, "birth_order", "order", "first");
    verified_assertion(
        client,
        prefix,
        person_id,
        "vital_status",
        "living",
        "living",
    );
    person_id
}

fn insert_appointment(client: &mut Client, person_key: &str, branch: &str) {
    client
        .execute(
            "INSERT INTO head_of_family_appointment
                 (person_key, branch_key, approved_by, evidence_assertion_ids, status)
             VALUES ($1, $2, 'synthetic-reviewer', '[]'::jsonb, 'active')",
            &[&person_key, &branch],
        )
        .expect("insert synthetic appointment");
}

fn insert_link(client: &mut Client, person_id: i64, account_id: i64, fingerprint: &str) {
    client
        .execute(
            "INSERT INTO onboarding_person_account_link
                 (person_id, account_id, linked_by, identity_fingerprint)
             VALUES ($1, $2, 'synthetic-reviewer', $3)",
            &[&person_id, &account_id, &fingerprint],
        )
        .expect("insert synthetic reviewed link");
}

fn build_fixture(label: &str, target_branch: &str) -> Option<Fixture> {
    let base = base_dsn_or_skip(label)?;
    let target = create_isolated_target(&base, label);
    let prefix = format!("synthetic-pir003-{}-{label}", std::process::id());
    let mut client = target.connect();

    let caller_key = format!("{prefix}-caller");
    let target_key = format!("{prefix}-target");
    seed_eligible_person(
        &mut client,
        &prefix,
        &caller_key,
        "Synthetic Head",
        CALLER_BRANCH,
    );
    let target_person_id = seed_eligible_person(
        &mut client,
        &prefix,
        &target_key,
        "Synthetic Target",
        target_branch,
    );
    insert_appointment(&mut client, &caller_key, CALLER_BRANCH);

    let issuer = format!("{prefix}-issuer");
    let subject = format!("{prefix}-subject");
    let account = IdentityAccount {
        account_id: 1,
        identity_fingerprint: identity_fingerprint(&issuer, &subject),
    };
    let caller_person_id: i64 = client
        .query_one(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&caller_key],
        )
        .expect("caller person")
        .get(0);
    insert_link(
        &mut client,
        caller_person_id,
        account.account_id,
        &account.identity_fingerprint,
    );

    let identity = VerifiedIdentity::new(subject, issuer, vec![Role::Viewer], vec![]);
    Some(Fixture {
        target,
        prefix,
        target_person_key: target_key,
        target_person_id,
        identity,
        account,
    })
}

fn admin_fixture(label: &str) -> Option<Fixture> {
    let base = base_dsn_or_skip(label)?;
    let target = create_isolated_target(&base, label);
    let prefix = format!("synthetic-pir003-{}-{label}", std::process::id());
    let mut client = target.connect();
    let target_key = format!("{prefix}-target");
    let target_person_id = seed_eligible_person(
        &mut client,
        &prefix,
        &target_key,
        "Synthetic Target",
        TARGET_BRANCH,
    );
    let issuer = format!("{prefix}-issuer");
    let subject = format!("{prefix}-subject");
    let account = IdentityAccount {
        account_id: 2,
        identity_fingerprint: identity_fingerprint(&issuer, &subject),
    };
    let identity = VerifiedIdentity::new(subject, issuer, vec![Role::Admin], vec![]);
    Some(Fixture {
        target,
        prefix,
        target_person_key: target_key,
        target_person_id,
        identity,
        account,
    })
}

fn count_invitations(client: &mut Client, person_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation WHERE person_id = $1",
            &[&person_id],
        )
        .expect("count invitations")
        .get(0)
}

fn count_drafts(client: &mut Client, person_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation_contact_draft WHERE person_id = $1",
            &[&person_id],
        )
        .expect("count drafts")
        .get(0)
}

fn count_recipients(client: &mut Client, person_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_recipient_contact WHERE person_id = $1",
            &[&person_id],
        )
        .expect("count recipients")
        .get(0)
}

fn assert_masked_only(masked: Option<&str>, local_part: &str) {
    let Some(masked) = masked else {
        return;
    };
    assert!(
        !masked.contains(local_part),
        "masked contact leaked the local part: {masked}"
    );
    assert!(
        masked.ends_with("@example.test"),
        "masked contact dropped the domain: {masked}"
    );
}

/// Missing contact: a saved draft reads back as a masked contact and creates no
/// invitation row and no delivery recipient.
#[test]
fn missing_contact_saves_as_draft_without_sending() {
    let Some(fixture) = build_fixture("pir003-missing-contact", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();

    let state: InvitationState = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("read state before draft");
    assert!(state.masked_contact.is_none());
    assert!(state
        .blockers
        .iter()
        .any(|blocker| blocker.code == "missing_contact"));

    let refused = prepare_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    );
    assert!(matches!(
        refused,
        Err(ProfileInvitationError::MissingContact)
    ));

    let email = fixture.email();
    let local = fixture.prefix.clone();
    let masked = save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("save draft contact");
    assert_masked_only(Some(&masked), &local);

    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("read state after draft");
    assert_masked_only(state.masked_contact.as_deref(), &local);
    assert!(!state
        .blockers
        .iter()
        .any(|blocker| blocker.code == "missing_contact"));
    assert_eq!(count_drafts(&mut client, fixture.target_person_id), 1);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
    assert_eq!(count_recipients(&mut client, fixture.target_person_id), 0);
    assert_eq!(state.invitation.delivery_state, "not_sent");
    assert!(!state.invitation.pending);
}

#[test]
fn saved_draft_does_not_send() {
    let Some(fixture) = build_fixture("pir003-no-send", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let email = fixture.email();
    let local = fixture.prefix.clone();
    save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("first draft save");
    let masked = save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("second draft save");
    assert_masked_only(Some(&masked), &local);
    assert_eq!(count_drafts(&mut client, fixture.target_person_id), 1);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
    assert_eq!(count_recipients(&mut client, fixture.target_person_id), 0);
    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("read state");
    assert_eq!(state.invitation.delivery_state, "not_sent");
}

#[test]
fn duplicate_pending_invitation_is_reused() {
    let Some(fixture) = build_fixture("pir003-duplicate", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let email = fixture.email();
    let local = fixture.prefix.clone();
    save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("save draft");
    let first: PrepareReview = prepare_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("first prepare");
    assert_eq!(first.access, "viewer");
    assert_masked_only(Some(&first.masked_contact), &local);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
    let second = prepare_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("second prepare");
    assert_eq!(second.invitation_id, first.invitation_id);
    assert!(second.invitation_reused);
    assert!(!second.invitation_created);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
    assert_eq!(count_recipients(&mut client, fixture.target_person_id), 0);
}

#[test]
fn linked_account_is_reported_and_refused() {
    let Some(fixture) = build_fixture("pir003-linked", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let other_fingerprint = "c".repeat(64);
    insert_link(&mut client, fixture.target_person_id, 2, &other_fingerprint);
    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("read state");
    assert!(state.linked_account);
    assert_eq!(state.eligibility, Classification::AlreadyLinked.as_str());
    assert!(state
        .blockers
        .iter()
        .any(|blocker| blocker.code == "already_linked"));
    let refused = prepare_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    );
    assert!(matches!(
        refused,
        Err(ProfileInvitationError::AlreadyLinked) | Err(ProfileInvitationError::NotEligible)
    ));
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
}

#[test]
fn wrong_branch_family_head_is_denied() {
    let Some(fixture) = build_fixture("pir003-wrong-branch", OTHER_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let email = fixture.email();
    let result = save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    );
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
    assert_eq!(count_drafts(&mut client, fixture.target_person_id), 0);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
}

#[test]
fn system_admin_receives_masked_summary() {
    let Some(fixture) = admin_fixture("pir003-admin") else {
        return;
    };
    let mut client = fixture.client();
    let email = fixture.email();
    let local = fixture.prefix.clone();
    let masked = save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("admin saves draft");
    assert_masked_only(Some(&masked), &local);
    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("admin reads state");
    assert_eq!(state.authorization, "system_admin");
    assert_masked_only(state.masked_contact.as_deref(), &local);
}

#[test]
fn preparation_returns_mask_username_and_viewer_access() {
    let Some(fixture) = build_fixture("pir003-prepare", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let email = fixture.email();
    let local = fixture.prefix.clone();
    save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("save draft");
    let review = prepare_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("prepare");
    assert_eq!(review.access, "viewer");
    assert_masked_only(Some(&review.masked_contact), &local);
    assert!(review.proposed_username.is_some());
    assert!(review.invitation_created);
}

#[test]
fn unlinked_callers_are_denied() {
    let Some(fixture) = admin_fixture("pir003-unlinked") else {
        return;
    };
    let mut client = fixture.client();
    let viewer = VerifiedIdentity::new(
        "unlinked-subject",
        "unlinked-issuer",
        vec![Role::Viewer],
        vec![],
    );
    let account = IdentityAccount {
        account_id: 99,
        identity_fingerprint: identity_fingerprint("unlinked-issuer", "unlinked-subject"),
    };
    let result = read_invitation_state(&mut client, &viewer, &account, &fixture.target_person_key);
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
}

#[test]
fn revoked_callers_are_denied() {
    let Some(fixture) = build_fixture("pir003-revoked", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    client
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE account_id = $1",
            &[&fixture.account.account_id],
        )
        .expect("revoke link");
    let result = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    );
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
}

#[test]
fn migration_is_additive() {
    let sql = std::fs::read_to_string(
        "genealogy/migrations/0033_onboarding_invitation_contact_draft.sql",
    )
    .expect("read draft migration");
    assert!(sql.contains("CREATE TABLE IF NOT EXISTS onboarding_invitation_contact_draft"));
    assert!(!sql.to_lowercase().contains("drop "));
    assert!(!sql.to_lowercase().contains("alter table"));
}
