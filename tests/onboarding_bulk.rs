//! Synthetic PostgreSQL integration test for bulk onboarding (BULK-003/004).
//!
//! The test writes only to the explicitly configured disposable genealogy
//! database. It skips when `GENEALOGY_DATABASE_URL` is absent and refuses a
//! destination that is not clearly a test database or that equals
//! `DATABASE_URL`.
//!
//! Every row is synthetic, prefixed, and removed before the test ends. The test
//! proves: eligibility classification, a zero-mutation dry run, idempotent and
//! concurrent preparation, one reservation per person, partial-failure
//! preservation, an unconfigured delivery adapter that creates no invitation,
//! an explicitly approved fake send, retry idempotency, and lifecycle sync.

use std::collections::HashMap;
use std::fs;
use std::str::FromStr;
use std::sync::{Arc, Barrier};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    batch_status_items, confirm_batch, create, issue_deliverable_invitation, load_name_parts,
    preview_candidates, preview_dry_run, reclassify_person, reserve, send_batch,
    sync_invitation_status, verify_recipient, Classification, DeliveryError, DeliveryStatus,
    DryRunFilters, FakeAdapter, ItemStatus, Page, Selector, UnconfiguredAdapter,
};

const MIGRATIONS: [&str; 27] = [
    "genealogy/migrations/0001_genealogy_schema_version.sql",
    "genealogy/migrations/0002_catalog_keys.sql",
    "genealogy/migrations/0003_assertions_evidence_and_links.sql",
    "genealogy/migrations/0004_review_workflow.sql",
    "genealogy/migrations/0005_source_kind.sql",
    "genealogy/migrations/0006_head_of_family.sql",
    "genealogy/migrations/0007_onboarding.sql",
    "genealogy/migrations/0008_onboarding_identity.sql",
    "genealogy/migrations/0009_onboarding_link_audit.sql",
    "genealogy/migrations/0010_onboarding_foreign_keys.sql",
    "genealogy/migrations/0011_onboarding_foreign_key_reconciliation.sql",
    "genealogy/migrations/0012_profile_revisions.sql",
    "genealogy/migrations/0013_profile_revision_actor_scope.sql",
    "genealogy/migrations/0014_profile_revision_draft_withdrawal.sql",
    "genealogy/migrations/0015_profile_media.sql",
    "genealogy/migrations/0016_profile_media_withdrawal.sql",
    "genealogy/migrations/0017_profile_media_withdrawal_audit.sql",
    "genealogy/migrations/0018_onboarding_batch.sql",
    "genealogy/migrations/0019_branch_reviewer_grants.sql",
    "genealogy/migrations/0020_branch_review_proposals.sql",
    "genealogy/migrations/0021_profile_field_restriction.sql",
    "genealogy/migrations/0022_profile_revision_restriction_generation.sql",
    "genealogy/migrations/0023_document_extraction.sql",
    "genealogy/migrations/0024_document_evidence_provenance.sql",
    "genealogy/migrations/0025_assertion_dates.sql",
    "genealogy/migrations/0026_authority_decisions.sql",
    "genealogy/migrations/0027_source_narrative.sql",
];

fn genealogy_dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        return None;
    }
    let config = postgres::Config::from_str(&dsn).expect("GENEALOGY_DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    if let Ok(rag) = std::env::var("DATABASE_URL") {
        let rag_config = postgres::Config::from_str(&rag).expect("DATABASE_URL parses");
        assert_ne!(
            (config.get_dbname(), config.get_hosts(), config.get_ports()),
            (
                rag_config.get_dbname(),
                rag_config.get_hosts(),
                rag_config.get_ports()
            ),
            "the bulk integration test must not target the RAG database"
        );
    }
    Some(dsn)
}

fn unique_prefix() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    format!("synthetic-bulk-{}-{nanos}", std::process::id())
}

/// Removes onboarding rows whose person was deleted by an earlier test or E2E
/// run. Migration 0011 validates the person foreign keys and fails loudly on
/// such orphans. Guarded by table existence so a fresh database is untouched.
fn clear_orphans(client: &mut Client) {
    let person_exists: bool = client
        .query_one("SELECT to_regclass('person') IS NOT NULL", &[])
        .map(|row| row.get(0))
        .unwrap_or(false);
    if !person_exists {
        return;
    }
    for table in [
        "onboarding_invitation",
        "onboarding_username_reservation",
        "onboarding_person_account_link",
        "onboarding_link_audit",
    ] {
        let exists: bool = client
            .query_one("SELECT to_regclass($1) IS NOT NULL", &[&table])
            .map(|row| row.get(0))
            .unwrap_or(false);
        if exists {
            let sql = format!(
                "DELETE FROM {table} t WHERE NOT EXISTS \
                 (SELECT 1 FROM person p WHERE p.person_id = t.person_id)"
            );
            let _ = client.execute(&sql, &[]);
        }
    }
}

fn apply_migrations(client: &mut Client) {
    for path in MIGRATIONS {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn db_error<E: std::fmt::Debug>(err: &E) -> String {
    format!("{err:?}")
}

fn connect(dsn: &str) -> Client {
    Client::connect(dsn, NoTls).expect("connect to the disposable genealogy database")
}

/// Removes this run's rows even when an assertion panics. A panic unwinds, so
/// the explicit cleanup path is not enough.
struct Cleaner {
    dsn: String,
    prefix: String,
}

impl Drop for Cleaner {
    fn drop(&mut self) {
        if let Ok(mut client) = Client::connect(&self.dsn, NoTls) {
            cleanup(&mut client, &self.prefix);
        }
    }
}

#[derive(Default)]
struct Seeded {
    eligible: Vec<i64>,
    deceased: i64,
    disputed: i64,
    already_linked: i64,
    already_invited: i64,
    ambiguous: i64,
    minor: i64,
    unverified: i64,
    collision: Vec<i64>,
}

#[test]
fn bulk_onboarding_is_prepared_sent_and_idempotent() {
    let Some(dsn) = genealogy_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping bulk onboarding integration test");
        return;
    };
    let mut client = connect(&dsn);
    clear_orphans(&mut client);
    apply_migrations(&mut client);
    let prefix = unique_prefix();
    let _cleaner = Cleaner {
        dsn: dsn.clone(),
        prefix: prefix.clone(),
    };
    let result = run(&dsn, &mut client, &prefix);
    result.unwrap_or_else(|err| panic!("{err}"));
}

fn run(dsn: &str, client: &mut Client, prefix: &str) -> Result<(), String> {
    let seeded = seed(client, prefix)?;

    // 1. Classification of synthetic living and ineligible people.
    let preview = preview_candidates(client).map_err(|err| db_error(&err))?;
    macro_rules! find {
        ($id:expr) => {
            find_row(&preview, client, $id)
        };
    }
    for id in &seeded.eligible {
        assert_eq!(find!(*id).classification, Classification::EligibleForInvite);
    }
    assert_eq!(
        find!(seeded.deceased).classification,
        Classification::NotEligible
    );
    assert_eq!(
        find!(seeded.disputed).classification,
        Classification::NeedsReview
    );
    assert_eq!(
        find!(seeded.already_linked).classification,
        Classification::AlreadyLinked
    );
    assert_eq!(
        find!(seeded.already_invited).classification,
        Classification::Invited
    );
    assert_eq!(
        find!(seeded.ambiguous).classification,
        Classification::NeedsReview
    );
    assert_eq!(
        find!(seeded.minor).classification,
        Classification::NeedsReview
    );
    assert_eq!(
        find!(seeded.unverified).classification,
        Classification::NotEligible
    );
    for id in &seeded.collision {
        assert_eq!(find!(*id).classification, Classification::EligibleForInvite);
    }

    // 2. Dry run is read-only.
    let before = mutation_counts(client)?;
    let selector = Selector::AllCurrentlyEligible;
    let report = preview_dry_run(
        &preview,
        &selector,
        &DryRunFilters::default(),
        Page::default(),
    );
    assert!(report.ok);
    assert!(report.eligible_for_invite >= seeded.eligible.len());
    let after = mutation_counts(client)?;
    assert_eq!(before, after, "dry-run must not mutate any table");

    // 3. Prepare exactly the two eligible people. The actor carries the run
    // prefix so a leftover batch from an earlier run cannot collide with this
    // run's idempotency key.
    let actor = format!("synthetic-admin-{prefix}");
    let ids = seeded.eligible.clone();
    let first = confirm_batch(client, &actor, ids.len(), &ids, &no_overrides())
        .map_err(|err| db_error(&err))?;
    assert!(first.created);
    assert_eq!(first.count(ItemStatus::Prepared), ids.len());
    let reservations: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_username_reservation
              WHERE person_id = ANY($1) AND status = 'reserved'",
            &[&ids],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(reservations as usize, ids.len());
    let status = batch_status_items(client, first.batch_id).map_err(|err| db_error(&err))?;
    let mut names: Vec<&str> = status
        .iter()
        .map(|item| item.display_name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["Eligible One", "Eligible Two"]);
    let wire = serde_json::to_value(&status[0]).map_err(|err| db_error(&err))?;
    assert!(wire.get("display_name").is_some() && wire.get("person_key").is_some());

    // 4. Repeating the same batch changes no logical state.
    let second = confirm_batch(client, &actor, ids.len(), &ids, &no_overrides())
        .map_err(|err| db_error(&err))?;
    assert_eq!(second.batch_id, first.batch_id);
    assert!(!second.created);
    let item_count: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_batch_item WHERE batch_id = $1",
            &[&first.batch_id],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(item_count as usize, ids.len());
    let reservations_after: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_username_reservation
              WHERE person_id = ANY($1) AND status = 'reserved'",
            &[&ids],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(reservations_after, reservations);

    // 5. Concurrent identical submissions still produce one batch and one
    // reservation per person.
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let dsn = dsn.to_string();
        let ids = ids.clone();
        let barrier = Arc::clone(&barrier);
        let concurrent_actor = format!("concurrent-admin-{prefix}");
        handles.push(std::thread::spawn(move || {
            let mut thread_client = connect(&dsn);
            barrier.wait();
            confirm_batch(
                &mut thread_client,
                &concurrent_actor,
                ids.len(),
                &ids,
                &no_overrides(),
            )
        }));
    }
    let mut batch_ids = Vec::new();
    for handle in handles {
        let report = handle
            .join()
            .map_err(|_| "concurrent worker panicked".to_string())?
            .map_err(|err| db_error(&err))?;
        batch_ids.push(report.batch_id);
    }
    assert_eq!(batch_ids[0], batch_ids[1]);
    let concurrent_reservations: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_username_reservation
              WHERE person_id = ANY($1) AND status <> 'released'",
            &[&ids],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(concurrent_reservations as usize, ids.len());

    // 6. A different batch reselecting an already prepared person is skipped.
    let reselected = confirm_batch(
        client,
        &format!("other-admin-{prefix}"),
        ids.len(),
        &ids,
        &no_overrides(),
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(
        reselected.count(ItemStatus::SkippedAlreadyPrepared),
        ids.len()
    );

    // 6b. Username collisions resolve to distinct reservations.
    let collision_batch = confirm_batch(
        client,
        &format!("collision-admin-{prefix}"),
        seeded.collision.len(),
        &seeded.collision,
        &no_overrides(),
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(
        collision_batch.count(ItemStatus::Prepared),
        seeded.collision.len()
    );
    let usernames: Vec<String> = client
        .query(
            "SELECT username FROM onboarding_username_reservation
              WHERE person_id = ANY($1) AND status <> 'released'",
            &[&seeded.collision],
        )
        .map_err(|err| db_error(&err))?
        .iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(usernames.len(), seeded.collision.len());
    assert_eq!(
        usernames
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        usernames.len()
    );

    // 6c. Admin username overrides go through the same policy and uniqueness.
    let tag: String = prefix
        .chars()
        .filter(char::is_ascii_digit)
        .rev()
        .take(12)
        .collect();
    let chosen = format!("Override{tag}");
    let first_override = insert_person(client, prefix, "Override First", Some(1980))?;
    mark_living(client, prefix, first_override, true, false)?;
    let accepted = confirm_batch(
        client,
        &format!("override-admin-{prefix}"),
        1,
        &[first_override],
        &HashMap::from([(first_override, chosen.clone())]),
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(accepted.count(ItemStatus::Prepared), 1);
    let stored = format!("override{tag}");
    assert_eq!(reserved_username(client, first_override)?, stored);

    // A colliding override takes the next numeric suffix, like a generated one.
    let second_override = insert_person(client, prefix, "Override Second", Some(1980))?;
    mark_living(client, prefix, second_override, true, false)?;
    let collided = confirm_batch(
        client,
        &format!("override-collision-admin-{prefix}"),
        1,
        &[second_override],
        &HashMap::from([(second_override, chosen.clone())]),
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(collided.count(ItemStatus::Prepared), 1);
    assert_eq!(
        reserved_username(client, second_override)?,
        format!("{stored}2")
    );

    // A policy-breaking override fails only its own row.
    let bad_override = insert_person(client, prefix, "Override Bad", Some(1980))?;
    mark_living(client, prefix, bad_override, true, false)?;
    let plain = insert_person(client, prefix, "Override Plain", Some(1980))?;
    mark_living(client, prefix, plain, true, false)?;
    let mixed_overrides = confirm_batch(
        client,
        &format!("override-policy-admin-{prefix}"),
        2,
        &[bad_override, plain],
        &HashMap::from([(bad_override, "not allowed!".to_string())]),
    )
    .map_err(|err| db_error(&err))?;
    let bad_item = mixed_overrides
        .items
        .iter()
        .find(|item| item.person_id == bad_override)
        .ok_or("bad override item")?;
    assert_eq!(bad_item.status, ItemStatus::Failed);
    assert_eq!(bad_item.reason_code.as_deref(), Some("username_policy"));
    assert_eq!(bad_item.reservation_id, None);
    let plain_item = mixed_overrides
        .items
        .iter()
        .find(|item| item.person_id == plain)
        .ok_or("plain item")?;
    assert_eq!(plain_item.status, ItemStatus::Prepared);

    // Correcting the override and preparing the same selection again succeeds
    // for the failed row and keeps the earlier reservation of the other.
    let plain_username = reserved_username(client, plain)?;
    let corrected = confirm_batch(
        client,
        &format!("override-policy-admin-{prefix}"),
        2,
        &[bad_override, plain],
        &HashMap::from([(bad_override, format!("Fixed{tag}"))]),
    )
    .map_err(|err| db_error(&err))?;
    assert!(corrected.created);
    assert_ne!(corrected.batch_id, mixed_overrides.batch_id);
    let fixed_item = corrected
        .items
        .iter()
        .find(|item| item.person_id == bad_override)
        .ok_or("corrected item")?;
    assert_eq!(fixed_item.status, ItemStatus::Prepared);
    assert_eq!(
        reserved_username(client, bad_override)?,
        format!("fixed{tag}")
    );
    let plain_again = corrected
        .items
        .iter()
        .find(|item| item.person_id == plain)
        .ok_or("plain item again")?;
    assert_eq!(plain_again.status, ItemStatus::SkippedAlreadyPrepared);
    assert_eq!(reserved_username(client, plain)?, plain_username);

    // An override never replaces an active reservation.
    let parts = load_name_parts(client, first_override).map_err(|err| db_error(&err))?;
    let kept = reserve(
        client,
        first_override,
        &parts,
        Some(&format!("Different{tag}")),
        "synthetic-admin",
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(kept.username, stored);
    assert_eq!(reserved_username(client, first_override)?, stored);

    // 7. Partial failure preserves the successful row.
    let fresh = insert_person(client, prefix, "Fresh Eligible", Some(1990))?;
    mark_living(client, prefix, fresh, true, false)?;
    let partial = insert_person(client, prefix, "Partial Failure", Some(1990))?;
    mark_living(client, prefix, partial, true, false)?;
    revoke_living(client, partial)?;
    let mixed = confirm_batch(
        client,
        &format!("recovery-admin-{prefix}"),
        2,
        &[partial, fresh],
        &no_overrides(),
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(mixed.count(ItemStatus::Prepared), 1);
    assert!(mixed.count(ItemStatus::NeedsReview) + mixed.count(ItemStatus::Failed) >= 1);

    // 8. An unconfigured adapter creates no invitation and never claims delivery.
    let before = invitation_count(client, mixed.batch_id)?;
    let unconfigured = UnconfiguredAdapter;
    assert_eq!(
        send_batch(
            client,
            mixed.batch_id,
            None,
            &unconfigured,
            "/activate",
            Duration::from_secs(3600),
            "synthetic-admin",
            Duration::ZERO,
        ),
        Err(DeliveryError::NotConfigured)
    );
    assert_eq!(invitation_count(client, mixed.batch_id)?, before);

    // 9. Verified recipients + an explicitly approved fake send.
    for id in &ids {
        verify_recipient(client, *id, "synthetic@example.test", "synthetic-admin")
            .map_err(|err| db_error(&err))?;
    }
    let adapter = FakeAdapter::accepting();
    let outcomes = send_batch(
        client,
        first.batch_id,
        None,
        &adapter,
        "/activate",
        Duration::from_secs(3600),
        "synthetic-admin",
        Duration::ZERO,
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(outcomes.len(), ids.len());
    assert!(outcomes
        .iter()
        .all(|outcome| outcome.status == DeliveryStatus::Sent));
    assert_eq!(adapter.messages().len(), ids.len());
    // The message body carries the one-time link; the API outcome does not.
    assert!(adapter.messages()[0].body.contains("token="));
    assert!(!format!("{:?}", outcomes[0]).contains("token="));

    // 10. A repeat send creates no second invitation.
    let invitations_before: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation WHERE person_id = ANY($1)",
            &[&ids],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    let repeated = send_batch(
        client,
        first.batch_id,
        None,
        &adapter,
        "/activate",
        Duration::from_secs(3600),
        "synthetic-admin",
        Duration::ZERO,
    )
    .map_err(|err| db_error(&err))?;
    assert!(repeated
        .iter()
        .all(|outcome| outcome.reason == Some("already_delivered")));
    let invitations_after: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_invitation WHERE person_id = ANY($1)",
            &[&ids],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(invitations_before, invitations_after);

    // 11. The stored invitation is a hash, never the plaintext token.
    let hashes: Vec<String> = client
        .query(
            "SELECT token_hash FROM onboarding_invitation WHERE person_id = ANY($1)",
            &[&ids],
        )
        .map_err(|err| db_error(&err))?
        .iter()
        .map(|row| row.get(0))
        .collect();
    for hash in hashes {
        assert_eq!(hash.len(), 64);
        assert!(!adapter
            .messages()
            .iter()
            .any(|message| message.body.contains(&hash)));
    }

    // 12. Acceptance and expiry are reflected from the invitation row.
    let invitation_id: i64 = client
        .query_one(
            "SELECT invitation_id FROM onboarding_invitation WHERE person_id = $1",
            &[&ids[0]],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    client
        .execute(
            "UPDATE onboarding_invitation SET status = 'accepted', accepted_at = now()
              WHERE invitation_id = $1",
            &[&invitation_id],
        )
        .map_err(|err| db_error(&err))?;
    sync_invitation_status(client, first.batch_id).map_err(|err| db_error(&err))?;
    let accepted: String = client
        .query_one(
            "SELECT delivery_status FROM onboarding_batch_item
              WHERE batch_id = $1 AND person_id = $2",
            &[&first.batch_id, &ids[0]],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(accepted, "accepted");

    // 13. Revoked evidence blocks preparation on re-evaluation.
    let revoked_check = insert_person(client, prefix, "Revocation Check", Some(1990))?;
    mark_living(client, prefix, revoked_check, true, false)?;
    let revoked_key = key_for(client, revoked_check).expect("fresh person exists");
    let (classification, _) =
        reclassify_person(client, revoked_check, &revoked_key).map_err(|err| db_error(&err))?;
    assert_eq!(classification, Classification::EligibleForInvite);
    revoke_living(client, revoked_check)?;
    let (classification, reasons) =
        reclassify_person(client, revoked_check, &revoked_key).map_err(|err| db_error(&err))?;
    assert_eq!(classification, Classification::NeedsReview);
    assert!(
        reasons.contains(&rag_foundation::onboarding::Reason::Disputed)
            || reasons.contains(&rag_foundation::onboarding::Reason::NoLivingEvidence)
    );

    // 14. A non-parseable birth year is age-uncertain, never assumed adult.
    let uncertain_age = insert_person(client, prefix, "Uncertain Age", Some(2015))?;
    client
        .execute(
            "UPDATE person SET birth_year = 'circa 2015' WHERE person_id = $1",
            &[&uncertain_age],
        )
        .map_err(|err| db_error(&err))?;
    mark_living(client, prefix, uncertain_age, true, false)?;
    let uncertain_key = key_for(client, uncertain_age).expect("uncertain person exists");
    let (classification, reasons) =
        reclassify_person(client, uncertain_age, &uncertain_key).map_err(|err| db_error(&err))?;
    assert_eq!(classification, Classification::NeedsReview);
    assert!(reasons.contains(&rag_foundation::onboarding::Reason::MinorGuardianReview));

    // 15. Explicit structured name parts make a one-token display safe.
    let structured = insert_person(client, prefix, "Madonna", Some(1970))?;
    client
        .execute(
            "UPDATE person SET family_name = 'Tran', given_name = 'Madonna'
              WHERE person_id = $1",
            &[&structured],
        )
        .map_err(|err| db_error(&err))?;
    mark_living(client, prefix, structured, true, false)?;
    let structured_key = key_for(client, structured).expect("structured person exists");
    let (classification, reasons) =
        reclassify_person(client, structured, &structured_key).map_err(|err| db_error(&err))?;
    assert_eq!(classification, Classification::EligibleForInvite);
    assert!(!reasons.contains(&rag_foundation::onboarding::Reason::AmbiguousSegmentation));

    // 16. An expired pending invitation leaves `pending` and allows a safe retry.
    let expiring = insert_person(client, prefix, "Expiring Person", Some(1985))?;
    mark_living(client, prefix, expiring, true, false)?;
    let expiry_batch = confirm_batch(
        client,
        &format!("expiry-admin-{prefix}"),
        1,
        &[expiring],
        &no_overrides(),
    )
    .map_err(|err| db_error(&err))?;
    verify_recipient(client, expiring, "expiry@example.test", "synthetic-admin")
        .map_err(|err| db_error(&err))?;
    let expiry_adapter = FakeAdapter::accepting();
    let first_send = send_batch(
        client,
        expiry_batch.batch_id,
        None,
        &expiry_adapter,
        "/activate",
        Duration::from_secs(3600),
        "synthetic-admin",
        Duration::ZERO,
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(first_send[0].status, DeliveryStatus::Sent);
    let old_invitation: i64 = client
        .query_one(
            "SELECT invitation_id FROM onboarding_batch_item
              WHERE batch_id = $1 AND person_id = $2",
            &[&expiry_batch.batch_id, &expiring],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    client
        .execute(
            "UPDATE onboarding_invitation
                SET expires_at = now() - interval '1 hour'
              WHERE invitation_id = $1",
            &[&old_invitation],
        )
        .map_err(|err| db_error(&err))?;
    sync_invitation_status(client, expiry_batch.batch_id).map_err(|err| db_error(&err))?;
    let expired_status: String = client
        .query_one(
            "SELECT delivery_status FROM onboarding_batch_item
              WHERE batch_id = $1 AND person_id = $2",
            &[&expiry_batch.batch_id, &expiring],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(expired_status, "expired");
    let expiry_key = key_for(client, expiring).expect("expiring person exists");
    let (classification, _) =
        reclassify_person(client, expiring, &expiry_key).map_err(|err| db_error(&err))?;
    assert!(matches!(
        classification,
        Classification::EligibleForInvite | Classification::AlreadyPrepared
    ));
    let retry = send_batch(
        client,
        expiry_batch.batch_id,
        None,
        &expiry_adapter,
        "/activate",
        Duration::from_secs(3600),
        "synthetic-admin",
        Duration::ZERO,
    )
    .map_err(|err| db_error(&err))?;
    assert_eq!(retry[0].status, DeliveryStatus::Sent);
    let new_invitation: i64 = client
        .query_one(
            "SELECT invitation_id FROM onboarding_batch_item
              WHERE batch_id = $1 AND person_id = $2",
            &[&expiry_batch.batch_id, &expiring],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_ne!(new_invitation, old_invitation);

    // 17. A consumed pending invitation reissues under the replacement id.
    let reissue_person = insert_person(client, prefix, "Reissue Person", Some(1985))?;
    mark_living(client, prefix, reissue_person, true, false)?;
    let first_issue = create(
        client,
        reissue_person,
        Duration::from_secs(3600),
        "synthetic-admin",
    )
    .map_err(|err| db_error(&err))?;
    let (replacement_id, token) = issue_deliverable_invitation(
        client,
        reissue_person,
        Duration::from_secs(3600),
        "synthetic-admin",
    )
    .map_err(|err| db_error(&err))?
    .expect("replacement invitation carries a token");
    assert_ne!(replacement_id, first_issue.invitation_id);
    assert!(!token.is_empty());
    let old_status: String = client
        .query_one(
            "SELECT status FROM onboarding_invitation WHERE invitation_id = $1",
            &[&first_issue.invitation_id],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    assert_eq!(old_status, "revoked");

    Ok(())
}

fn seed(client: &mut Client, prefix: &str) -> Result<Seeded, String> {
    let mut seeded = Seeded::default();
    for name in ["Eligible One", "Eligible Two"] {
        let id = insert_person(client, prefix, name, Some(1980))?;
        mark_living(client, prefix, id, true, false)?;
        seeded.eligible.push(id);
    }
    let id = insert_person(client, prefix, "Deceased Person", Some(1940))?;
    client
        .execute(
            "INSERT INTO assertion (person_id, assertion_kind, assertion_key, value_text, review_status)
             VALUES ($1, 'vital_status', 'death', 'deceased', 'unverified')",
            &[&id],
        )
        .map_err(|err| db_error(&err))?;
    seeded.deceased = id;

    let id = insert_person(client, prefix, "Disputed Person", Some(1970))?;
    mark_living(client, prefix, id, true, true)?;
    seeded.disputed = id;

    let id = insert_person(client, prefix, "Linked Person", Some(1970))?;
    mark_living(client, prefix, id, true, false)?;
    client
        .execute(
            "INSERT INTO onboarding_person_account_link (person_id, account_id, linked_by)
             VALUES ($1, 1, 'synthetic-reviewer')",
            &[&id],
        )
        .map_err(|err| db_error(&err))?;
    seeded.already_linked = id;

    let id = insert_person(client, prefix, "Invited Person", Some(1970))?;
    mark_living(client, prefix, id, true, false)?;
    client
        .execute(
            "INSERT INTO onboarding_invitation
                 (person_id, token_hash, expires_at, status, created_by)
             VALUES ($1, lpad(to_hex($1::bigint), 64, '0'), now() + interval '1 hour', 'pending', 'synthetic-admin')",
            &[&id],
        )
        .map_err(|err| db_error(&err))?;
    seeded.already_invited = id;

    let id = insert_person(client, prefix, "Ambiguous", Some(1970))?;
    mark_living(client, prefix, id, true, false)?;
    seeded.ambiguous = id;

    let current_year: i32 = client
        .query_one("SELECT EXTRACT(YEAR FROM now())::int", &[])
        .map_err(|err| db_error(&err))?
        .get(0);
    let id = insert_person(client, prefix, "Minor Person", Some(current_year - 10))?;
    mark_living(client, prefix, id, true, false)?;
    seeded.minor = id;

    let id = insert_person(client, prefix, "Unverified Living", Some(1970))?;
    mark_living(client, prefix, id, false, false)?;
    seeded.unverified = id;

    for name in ["Collision Nguyen", "Collision Nguyen"] {
        let id = insert_person(client, prefix, name, Some(1970))?;
        mark_living(client, prefix, id, true, false)?;
        seeded.collision.push(id);
    }
    Ok(seeded)
}

fn insert_person(
    client: &mut Client,
    prefix: &str,
    display: &str,
    birth_year: Option<i32>,
) -> Result<i64, String> {
    let key = format!("{prefix}-{}", unique_suffix());
    let id: i64 = client
        .query_one(
            "INSERT INTO person (person_key, display_name, birth_year)
             VALUES ($1, $2, $3) RETURNING person_id",
            &[&key, &display, &birth_year.map(|year| year.to_string())],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    Ok(id)
}

fn reserved_username(client: &mut Client, person_id: i64) -> Result<String, String> {
    client
        .query_one(
            "SELECT username FROM onboarding_username_reservation
              WHERE person_id = $1 AND status <> 'released'",
            &[&person_id],
        )
        .map(|row| row.get(0))
        .map_err(|err| db_error(&err))
}

fn no_overrides() -> HashMap<i64, String> {
    HashMap::new()
}

fn unique_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

fn mark_living(
    client: &mut Client,
    prefix: &str,
    person_id: i64,
    verify: bool,
    dispute: bool,
) -> Result<(), String> {
    let document = format!("{prefix}-source-{person_id}.md");
    client
        .execute(
            "INSERT INTO source_document (relative_path, sha256, source_kind)
             VALUES ($1, repeat('c', 64), 'original')",
            &[&document],
        )
        .map_err(|err| db_error(&err))?;
    let evidence_id: i64 = client
        .query_one(
            "INSERT INTO source_evidence (source_document_id, page)
             SELECT source_document_id, 1 FROM source_document WHERE relative_path = $1
             RETURNING source_evidence_id",
            &[&document],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    let assertion_id: i64 = client
        .query_one(
            "INSERT INTO assertion (person_id, assertion_kind, assertion_key, value_text)
             VALUES ($1, 'vital_status', 'living', 'alive') RETURNING assertion_id",
            &[&person_id],
        )
        .map_err(|err| db_error(&err))?
        .get(0);
    client
        .execute(
            "INSERT INTO assertion_evidence
                 (assertion_id, source_evidence_id, document_ref, page)
             VALUES ($1, $2, $3, 1)",
            &[&assertion_id, &evidence_id, &document],
        )
        .map_err(|err| db_error(&err))?;
    let status = if dispute {
        "disputed"
    } else if verify {
        "verified"
    } else {
        "unverified"
    };
    client
        .execute(
            "UPDATE assertion
                SET review_status = $2,
                    reviewed_by = CASE WHEN $2 = 'verified' THEN 'synthetic-reviewer' ELSE NULL END,
                    reviewed_at = CASE WHEN $2 = 'verified' THEN now() ELSE NULL END
              WHERE assertion_id = $1",
            &[&assertion_id, &status],
        )
        .map_err(|err| db_error(&err))?;
    Ok(())
}

fn revoke_living(client: &mut Client, person_id: i64) -> Result<(), String> {
    client
        .execute(
            "UPDATE assertion SET review_status = 'disputed'
              WHERE person_id = $1 AND assertion_kind = 'vital_status'",
            &[&person_id],
        )
        .map_err(|err| db_error(&err))?;
    Ok(())
}

fn find_row<'a>(
    preview: &'a rag_foundation::onboarding::CandidatePreview,
    client: &mut Client,
    person_id: i64,
) -> &'a rag_foundation::onboarding::CandidateRow {
    let key = key_for(client, person_id).expect("seeded person exists");
    preview
        .rows
        .iter()
        .find(|row| row.person_key == key)
        .expect("seeded row is present")
}

fn key_for(client: &mut Client, person_id: i64) -> Result<String, String> {
    client
        .query_opt(
            "SELECT person_key FROM person WHERE person_id = $1",
            &[&person_id],
        )
        .map_err(|err| db_error(&err))?
        .map(|row| row.get(0))
        .ok_or_else(|| "unknown person".to_string())
}

fn mutation_counts(client: &mut Client) -> Result<(i64, i64, i64, i64), String> {
    let mut one = |sql: &str| -> Result<i64, String> {
        client
            .query_one(sql, &[])
            .map_err(|err| db_error(&err))
            .map(|row| row.get(0))
    };
    Ok((
        one("SELECT count(*) FROM onboarding_batch")?,
        one("SELECT count(*) FROM onboarding_batch_item")?,
        one("SELECT count(*) FROM onboarding_username_reservation")?,
        one("SELECT count(*) FROM onboarding_invitation")?,
    ))
}

fn invitation_count(client: &mut Client, batch_id: i64) -> Result<i64, String> {
    client
        .query_one(
            "SELECT count(*) FROM onboarding_batch_item
              WHERE batch_id = $1 AND invitation_id IS NOT NULL",
            &[&batch_id],
        )
        .map_err(|err| db_error(&err))
        .map(|row| row.get(0))
}

fn cleanup(client: &mut Client, prefix: &str) {
    let pattern = format!("{prefix}%");
    for statement in [
        "DELETE FROM onboarding_batch_audit WHERE batch_id IN
             (SELECT batch_id FROM onboarding_batch_item WHERE person_id IN
                 (SELECT person_id FROM person WHERE person_key LIKE $1))",
        "DELETE FROM onboarding_batch_item WHERE person_id IN
             (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_batch WHERE batch_id NOT IN
             (SELECT batch_id FROM onboarding_batch_item)",
        "DELETE FROM onboarding_batch WHERE batch_idempotency_key LIKE '%' || $1 || '%'",
        "DELETE FROM onboarding_recipient_contact WHERE person_id IN
             (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_link_audit WHERE person_id IN
             (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_person_account_link WHERE person_id IN
             (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_invitation WHERE person_id IN
             (SELECT person_id FROM person WHERE person_key LIKE $1)",
        "DELETE FROM onboarding_username_reservation WHERE person_id IN
             (SELECT person_id FROM person WHERE person_key LIKE $1)",
    ] {
        let _ = client.execute(statement, &[&pattern]);
    }
    let _ = client.execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
    let _ = client.execute(
        "DELETE FROM source_document WHERE relative_path LIKE $1",
        &[&pattern],
    );
}
