//! PIR-003/PIR-004 integration test: restricted contact drafts, invitation
//! preparation, and explicit send/resend/revoke/status.
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
    identity_fingerprint, prepare_invitation, read_invitation_state, read_invitation_status,
    resend_invitation, revoke_invitation, save_contact_draft, send_invitation, DeliveryError,
    DeliveryReceipt, FakeAdapter, InvitationMessage, InvitationState, InvitationStatusView,
    LoginProfile, MailAdapter, PrepareReview, ProfileDeliveryState, ProfileInvitationError,
    RevokeInvitationOutcome, SendInvitationOutcome, SentMessage, UnconfiguredAdapter,
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

/// An adapter that always fails, so a delivery failure is deterministic.
struct FailingAdapter;

impl MailAdapter for FailingAdapter {
    fn name(&self) -> &'static str {
        "failing"
    }

    fn deliver(
        &self,
        _to: &str,
        _subject: &str,
        _body: &str,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        Err(DeliveryError::Provider(
            "synthetic_provider_failure".to_string(),
        ))
    }
}

/// Saves the fixture's draft contact so a send has an address to target.
fn save_fixture_draft(client: &mut Client, fixture: &Fixture) {
    let email = fixture.email();
    save_contact_draft(
        client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    )
    .expect("save draft contact");
}

fn count_pending(client: &mut Client, person_id: i64) -> i64 {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation
              WHERE person_id = $1 AND status = 'pending'",
            &[&person_id],
        )
        .expect("count pending invitations")
        .get(0)
}

fn status_for(client: &mut Client, fixture: &Fixture) -> InvitationStatusView {
    read_invitation_status(
        client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("read invitation status")
}

fn send_with(
    client: &mut Client,
    fixture: &Fixture,
    adapter: &dyn MailAdapter,
) -> Result<SendInvitationOutcome, ProfileInvitationError> {
    send_invitation(
        client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        adapter,
        "https://example.test/activate",
    )
}

fn resend_with(
    client: &mut Client,
    fixture: &Fixture,
    adapter: &dyn MailAdapter,
) -> Result<SendInvitationOutcome, ProfileInvitationError> {
    resend_invitation(
        client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        adapter,
        "https://example.test/activate",
    )
}

/// One explicit send creates exactly one invitation, reports the adapter's real
/// receipt, and never duplicates or re-sends on a second call.
#[test]
fn explicit_send_reports_delivery_and_never_duplicates() {
    let Some(fixture) = build_fixture("pir004-send", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();

    let outcome = send_with(&mut client, &fixture, &adapter).expect("explicit send");
    assert!(outcome.delivered);
    assert_eq!(outcome.delivery_state, ProfileDeliveryState::Sent);
    assert_eq!(outcome.provider.as_deref(), Some("fake"));
    assert_masked_only(Some(&outcome.masked_contact), &fixture.prefix);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
    assert_eq!(count_pending(&mut client, fixture.target_person_id), 1);
    assert_eq!(count_recipients(&mut client, fixture.target_person_id), 0);

    let messages = adapter.messages();
    assert_eq!(messages.len(), 1, "one explicit send delivers one message");
    let link = messages[0]
        .body
        .split_whitespace()
        .find(|part| part.contains("/activate/"))
        .expect("message carries a link");
    let token = link
        .split("/activate/")
        .nth(1)
        .expect("link carries a token")
        .trim_end_matches(['.', ')', ']', '"', '\''])
        .to_string();
    assert!(!token.is_empty());
    let rendered = format!("{outcome:?}");
    assert!(!rendered.contains(&token), "the outcome leaked the token");
    assert!(
        !rendered.contains(&fixture.email()),
        "the outcome leaked the address"
    );

    let status = status_for(&mut client, &fixture);
    assert_eq!(status.invitation_status, Some("pending"));
    assert_eq!(status.delivery_state, ProfileDeliveryState::Sent);
    assert_eq!(status.delivery_attempts, 1);
    assert_eq!(status.delivery_provider.as_deref(), Some("fake"));
    let rendered = format!("{status:?}");
    assert!(!rendered.contains(&token), "the status leaked the token");
    assert!(
        !rendered.contains(&fixture.email()),
        "the status leaked the address"
    );

    let again = send_with(&mut client, &fixture, &adapter).expect("second send is reported");
    assert_eq!(again.invitation_id, outcome.invitation_id);
    assert!(again.delivered);
    assert_eq!(
        adapter.messages().len(),
        1,
        "a duplicate send sends nothing"
    );
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
}

/// An unconfigured adapter fails closed before an invitation is created.
#[test]
fn unconfigured_adapter_creates_no_invitation() {
    let Some(fixture) = build_fixture("pir004-unconfigured", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = UnconfiguredAdapter;
    let result = send_with(&mut client, &fixture, &adapter);
    assert!(matches!(
        result,
        Err(ProfileInvitationError::Delivery(
            DeliveryError::NotConfigured
        ))
    ));
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
}

/// A provider failure leaves the invitation recoverable: the failure is recorded,
/// no second invitation appears, and a retry revokes the failed row and issues
/// exactly one replacement.
#[test]
fn adapter_failure_stays_recoverable_then_retry_succeeds() {
    let Some(fixture) = build_fixture("pir004-failure", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let failing = FailingAdapter;

    let result = send_with(&mut client, &fixture, &failing);
    assert!(matches!(
        result,
        Err(ProfileInvitationError::Delivery(DeliveryError::Provider(_)))
    ));
    let rendered = format!("{result:?}");
    assert!(
        !rendered.contains(&fixture.email()),
        "the error leaked the address"
    );
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);

    let status = status_for(&mut client, &fixture);
    assert_eq!(status.invitation_status, Some("pending"));
    assert_eq!(status.delivery_state, ProfileDeliveryState::Failed);
    assert_eq!(status.delivery_attempts, 1);
    assert_eq!(
        status.last_delivery_error.as_deref(),
        Some("provider_failed")
    );

    let adapter = FakeAdapter::accepting();
    let retried = send_with(&mut client, &fixture, &adapter).expect("retry after failure");
    assert!(retried.delivered);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 2);
    assert_eq!(count_pending(&mut client, fixture.target_person_id), 1);
    let revoked: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation
              WHERE person_id = $1 AND status = 'revoked'",
            &[&fixture.target_person_id],
        )
        .expect("count revoked")
        .get(0);
    assert_eq!(
        revoked, 1,
        "the failed invitation is revoked, not left live"
    );
    assert_eq!(adapter.messages().len(), 1);
}

/// A resend revokes the previous link and leaves exactly one live invitation.
#[test]
fn resend_revokes_previous_and_issues_one_link() {
    let Some(fixture) = build_fixture("pir004-resend", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    let first = send_with(&mut client, &fixture, &adapter).expect("first send");
    let second = resend_with(&mut client, &fixture, &adapter).expect("resend");

    assert_ne!(second.invitation_id, first.invitation_id);
    assert!(second.delivered);
    assert_eq!(adapter.messages().len(), 2);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 2);
    assert_eq!(count_pending(&mut client, fixture.target_person_id), 1);
    let status = status_for(&mut client, &fixture);
    assert_eq!(status.invitation_id, Some(second.invitation_id));
    assert_eq!(status.delivery_state, ProfileDeliveryState::Sent);
}

/// Revoking reports the resulting state and is idempotent.
#[test]
fn revoke_reports_state_and_is_idempotent() {
    let Some(fixture) = build_fixture("pir004-revoke", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    let sent = send_with(&mut client, &fixture, &adapter).expect("send");

    let revoked: RevokeInvitationOutcome = revoke_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("revoke");
    assert!(revoked.revoked);
    assert_eq!(revoked.invitation_id, Some(sent.invitation_id));
    assert_eq!(revoked.delivery_state, ProfileDeliveryState::Revoked);
    assert_eq!(count_pending(&mut client, fixture.target_person_id), 0);

    let status = status_for(&mut client, &fixture);
    assert_eq!(status.invitation_status, Some("revoked"));
    assert_eq!(status.delivery_state, ProfileDeliveryState::Revoked);

    let again = revoke_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    )
    .expect("second revoke");
    assert!(!again.revoked);
    assert_eq!(again.reason, Some("no_pending_invitation"));
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
}

/// A queued message is not reported as delivered and is not re-sent implicitly.
#[test]
fn queued_delivery_is_not_reported_as_delivered() {
    let Some(fixture) = build_fixture("pir004-queued", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::queueing();
    let outcome = send_with(&mut client, &fixture, &adapter).expect("queued send");
    assert!(!outcome.delivered, "a queued message is not delivered");
    assert_eq!(outcome.delivery_state, ProfileDeliveryState::Queued);
    assert_eq!(adapter.messages().len(), 1);

    let again = send_with(&mut client, &fixture, &adapter).expect("second send");
    assert!(!again.delivered);
    assert_eq!(again.invitation_id, outcome.invitation_id);
    assert_eq!(adapter.messages().len(), 1, "a queued send is not repeated");
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
}

/// An expired invitation is reported as expired and can be reissued.
#[test]
fn expired_invitation_is_reported_and_reissued() {
    let Some(fixture) = build_fixture("pir004-expired", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    let first = send_with(&mut client, &fixture, &adapter).expect("send");
    client
        .execute(
            "UPDATE onboarding_invitation SET expires_at = now() - interval '1 hour'
              WHERE invitation_id = $1",
            &[&first.invitation_id],
        )
        .expect("expire invitation");

    let status = status_for(&mut client, &fixture);
    assert_eq!(status.invitation_status, Some("expired"));
    assert_eq!(status.delivery_state, ProfileDeliveryState::Expired);
    // A status read writes nothing: the stored row is still `pending`.
    let stored: String = client
        .query_one(
            "SELECT status FROM onboarding_invitation WHERE invitation_id = $1",
            &[&first.invitation_id],
        )
        .expect("read stored status")
        .get(0);
    assert_eq!(stored, "pending");

    let reissued = send_with(&mut client, &fixture, &adapter).expect("send after expiry");
    assert_ne!(reissued.invitation_id, first.invitation_id);
    assert!(reissued.delivered);
    assert_eq!(count_pending(&mut client, fixture.target_person_id), 1);
    assert_eq!(adapter.messages().len(), 2);
}

/// An accepted invitation is reported as accepted and cannot be resent.
#[test]
fn accepted_invitation_cannot_be_resent() {
    let Some(fixture) = build_fixture("pir004-accepted", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    let sent = send_with(&mut client, &fixture, &adapter).expect("send");
    client
        .execute(
            "UPDATE onboarding_invitation
                SET status = 'accepted', used_at = now(), accepted_at = now()
              WHERE invitation_id = $1",
            &[&sent.invitation_id],
        )
        .expect("accept invitation");

    let status = status_for(&mut client, &fixture);
    assert_eq!(status.invitation_status, Some("accepted"));
    assert_eq!(status.delivery_state, ProfileDeliveryState::Accepted);
    assert!(status.accepted_at.is_some());

    let result = resend_with(&mut client, &fixture, &adapter);
    assert!(matches!(
        result,
        Err(ProfileInvitationError::InvitationNotPending("accepted"))
    ));
    assert_eq!(adapter.messages().len(), 1, "accepted is never resent");
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 1);
}

/// A family head outside the target's branch cannot send, resend, or revoke.
#[test]
fn wrong_branch_family_head_cannot_send_or_revoke() {
    let Some(fixture) = build_fixture("pir004-wrong-branch", OTHER_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft_forbidden(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    let result = send_with(&mut client, &fixture, &adapter);
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
    let result = resend_with(&mut client, &fixture, &adapter);
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
    let result = revoke_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
    );
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
    assert_eq!(adapter.messages().len(), 0);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
}

/// A linked person cannot be sent an invitation.
#[test]
fn linked_person_cannot_be_sent_an_invitation() {
    let Some(fixture) = build_fixture("pir004-linked", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    insert_link(&mut client, fixture.target_person_id, 7, &"d".repeat(64));
    let adapter = FakeAdapter::accepting();
    let result = send_with(&mut client, &fixture, &adapter);
    assert!(matches!(
        result,
        Err(ProfileInvitationError::AlreadyLinked) | Err(ProfileInvitationError::NotEligible)
    ));
    assert_eq!(adapter.messages().len(), 0);
    assert_eq!(count_invitations(&mut client, fixture.target_person_id), 0);
}

/// The draft save in a wrong-branch fixture is refused, so the send path cannot
/// even be reached with an address.
fn save_fixture_draft_forbidden(client: &mut Client, fixture: &Fixture) {
    let email = fixture.email();
    let result = save_contact_draft(
        client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &email,
    );
    assert!(matches!(result, Err(ProfileInvitationError::Refused(_))));
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

/// A linked caller with no Head of Family appointment cannot invite their own
/// person: a reviewed link is no longer an invitation permission.
#[test]
fn a_linked_caller_without_an_appointment_cannot_invite_themselves() {
    let Some(fixture) = build_fixture("pir008-self-no-appointment", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let caller_key = format!("{}-caller", fixture.prefix);
    let caller_person_id: i64 = client
        .query_one(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&caller_key],
        )
        .expect("caller person")
        .get(0);
    client
        .execute(
            "DELETE FROM head_of_family_appointment WHERE person_key = $1",
            &[&caller_key],
        )
        .expect("remove the appointment");

    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &caller_key,
    );
    assert!(matches!(state, Err(ProfileInvitationError::Refused(_))));

    let email = fixture.email();
    let refused = save_contact_draft(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &caller_key,
        &email,
    );
    assert!(matches!(refused, Err(ProfileInvitationError::Refused(_))));
    assert_eq!(count_drafts(&mut client, caller_person_id), 0);
    assert_eq!(count_invitations(&mut client, caller_person_id), 0);
}

/// A family head may invite their own person only through their branch scope.
#[test]
fn a_family_head_may_invite_themselves_only_through_their_branch() {
    let Some(fixture) = build_fixture("pir008-self-branch", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    let caller_key = format!("{}-caller", fixture.prefix);
    let caller_person_id: i64 = client
        .query_one(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&caller_key],
        )
        .expect("caller person")
        .get(0);

    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &caller_key,
    )
    .expect("self within the branch");
    assert_eq!(state.authorization, "family_head_branch");

    // Moving the caller's own verified claim out of the appointment branch
    // removes the authority: there is no branch-independent self permission.
    client
        .execute(
            "UPDATE assertion SET value_text = $2
              WHERE person_id = $1 AND assertion_kind = 'branch_claim'",
            &[&caller_person_id, &OTHER_BRANCH],
        )
        .expect("move the caller branch claim");
    let state = read_invitation_state(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &caller_key,
    );
    assert!(matches!(state, Err(ProfileInvitationError::Refused(_))));
}

/// The activation link is a path segment on the `/activate/[token]` route, and
/// both the explicit send and the resend hand it to the adapter unchanged.
#[test]
fn send_and_resend_use_the_path_activation_link() {
    let Some(fixture) = build_fixture("pir011-link", TARGET_BRANCH) else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    // A trailing-slash base must not double the separator or fall back to a query.
    let base = "https://example.test/activate///";

    let link_from = |messages: &[SentMessage]| -> String {
        let message = messages.last().expect("one delivered message");
        message
            .body
            .split_whitespace()
            .find(|word| word.starts_with("https://example.test/"))
            .expect("the body carries the activation link")
            .to_string()
    };

    send_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &adapter,
        base,
    )
    .expect("explicit send");
    let first = link_from(&adapter.messages());
    assert!(
        first.starts_with("https://example.test/activate/"),
        "unexpected link: {first}"
    );
    assert!(
        !first.contains('?'),
        "the token must be a path segment: {first}"
    );
    let first_token = first.rsplit('/').next().expect("token segment").to_string();
    assert_eq!(first_token.len(), 64);
    assert!(first_token.chars().all(|c| c.is_ascii_hexdigit()));

    resend_invitation(
        &mut client,
        &fixture.identity,
        &fixture.account,
        &fixture.target_person_key,
        &adapter,
        base,
    )
    .expect("resend");
    let second = link_from(&adapter.messages());
    assert!(
        second.starts_with("https://example.test/activate/"),
        "unexpected link: {second}"
    );
    let second_token = second
        .rsplit('/')
        .next()
        .expect("token segment")
        .to_string();
    assert_ne!(
        first_token, second_token,
        "a resend issues a new single-use token"
    );
}

/// The delivery migration only adds nullable or defaulted columns.
#[test]
fn delivery_migration_is_additive() {
    let sql =
        std::fs::read_to_string("genealogy/migrations/0034_onboarding_invitation_delivery.sql")
            .expect("read delivery migration");
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS delivery_state"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS delivery_provider"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS last_delivery_error"));
    assert!(sql.contains("ADD COLUMN IF NOT EXISTS delivery_attempts"));
    let lower = sql.to_lowercase();
    assert!(!lower.contains("drop "));
    assert!(!lower.contains("delete "));
    assert!(!lower.contains("not null default null"));
}

/// An adapter that creates sign-ins: it records the login profile and link it
/// was handed, or fails with a fixed sign-in code.
struct LoginRecorder {
    fail: Option<&'static str>,
    seen: std::sync::Mutex<Vec<(LoginProfile, String)>>,
}

impl LoginRecorder {
    fn new(fail: Option<&'static str>) -> Self {
        Self {
            fail,
            seen: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn seen(&self) -> Vec<(LoginProfile, String)> {
        self.seen.lock().unwrap().clone()
    }
}

impl MailAdapter for LoginRecorder {
    fn name(&self) -> &'static str {
        "keycloak"
    }

    fn provisions_logins(&self) -> bool {
        true
    }

    fn deliver(&self, _: &str, _: &str, _: &str) -> Result<DeliveryReceipt, DeliveryError> {
        panic!("a sign-in adapter receives the whole invitation");
    }

    fn deliver_invitation(
        &self,
        message: &InvitationMessage<'_>,
    ) -> Result<DeliveryReceipt, DeliveryError> {
        if let Some(code) = self.fail {
            return Err(DeliveryError::Login(code));
        }
        let login = message.login.expect("a login profile is supplied").clone();
        assert!(
            message.body.contains(message.link),
            "the body carries the link"
        );
        self.seen
            .lock()
            .unwrap()
            .push((login, message.link.to_string()));
        Ok(DeliveryReceipt {
            provider: "keycloak".to_string(),
            accepted: true,
            message_id: None,
        })
    }
}

fn reserved_usernames(client: &mut Client, person_id: i64) -> Vec<String> {
    client
        .query(
            "SELECT username FROM onboarding_username_reservation
              WHERE person_id = $1 AND status <> 'released'",
            &[&person_id],
        )
        .expect("read reservations")
        .iter()
        .map(|row| row.get(0))
        .collect()
}

/// The sign-in adapter receives the person's reserved username, verified name
/// parts, and the activation link; a resend reuses the same username.
#[test]
fn sign_in_adapter_receives_the_reserved_username_and_link() {
    let Some(fixture) = admin_fixture("kc-login-send") else {
        return;
    };
    let mut client = fixture.client();
    client
        .execute(
            "UPDATE person SET display_name = 'Nguyen Van An', family_name = 'Nguyen',
                    given_name = 'An'
              WHERE person_id = $1",
            &[&fixture.target_person_id],
        )
        .expect("name the synthetic person");
    save_fixture_draft(&mut client, &fixture);
    let adapter = LoginRecorder::new(None);

    let outcome = send_with(&mut client, &fixture, &adapter).expect("send");
    assert!(outcome.delivered);
    assert_eq!(outcome.provider.as_deref(), Some("keycloak"));
    let reserved = reserved_usernames(&mut client, fixture.target_person_id);
    assert_eq!(reserved.len(), 1, "sending reserves exactly one username");

    let seen = adapter.seen();
    assert_eq!(seen.len(), 1);
    let (login, link) = &seen[0];
    assert_eq!(login.username, reserved[0]);
    assert_eq!(login.username, "nvan");
    assert_eq!(login.first_name.as_deref(), Some("Van An"));
    assert_eq!(login.last_name.as_deref(), Some("Nguyen"));
    assert!(link.starts_with("https://example.test/activate/"));

    resend_with(&mut client, &fixture, &adapter).expect("resend");
    let seen = adapter.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[1].0.username, "nvan", "a resend keeps the username");
    assert_ne!(seen[1].1, seen[0].1, "a resend issues a new link");
    assert_eq!(
        reserved_usernames(&mut client, fixture.target_person_id).len(),
        1
    );
}

/// A sign-in failure is recorded by its code and stays recoverable.
#[test]
fn sign_in_failure_is_recorded_and_recoverable() {
    let Some(fixture) = admin_fixture("kc-login-fail") else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let failing = LoginRecorder::new(Some("login_username_taken"));

    let result = send_with(&mut client, &fixture, &failing);
    assert!(matches!(
        result,
        Err(ProfileInvitationError::Delivery(DeliveryError::Login(
            "login_username_taken"
        )))
    ));
    let status = status_for(&mut client, &fixture);
    assert_eq!(status.delivery_state, ProfileDeliveryState::Failed);
    assert_eq!(
        status.last_delivery_error.as_deref(),
        Some("login_username_taken")
    );

    let adapter = LoginRecorder::new(None);
    let retried = send_with(&mut client, &fixture, &adapter).expect("retry");
    assert!(retried.delivered);
    assert_eq!(count_pending(&mut client, fixture.target_person_id), 1);
}

/// A plain mail adapter reserves no username: the sign-in step is opt-in.
#[test]
fn plain_adapter_reserves_no_username() {
    let Some(fixture) = admin_fixture("kc-login-plain") else {
        return;
    };
    let mut client = fixture.client();
    save_fixture_draft(&mut client, &fixture);
    let adapter = FakeAdapter::accepting();
    send_with(&mut client, &fixture, &adapter).expect("send");
    assert!(reserved_usernames(&mut client, fixture.target_person_id).is_empty());
}
