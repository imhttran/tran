//! USR-006-P: owner-initiated privacy restriction (decision A2).
//!
//! Synthetic PostgreSQL only. Uses `GENEALOGY_DATABASE_URL`, applies every
//! genealogy migration, and cleans only rows carrying its own prefix.

use std::fs;
use std::str::FromStr;
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    read_approved_values, read_family_profile, read_owner_profile, restrict_own_field,
    review_revision, IdentityAccount, ProfileAudience, ProfileVisibility, RestrictionError,
    ReviewDecision, RevisionError,
};

const MIGRATIONS: [&str; 21] = [
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
];

static DB_LOCK: Mutex<()> = Mutex::new(());

fn dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        eprintln!("GENEALOGY_DATABASE_URL unset; skipping profile restriction test");
        return None;
    }
    let config = postgres::Config::from_str(&dsn).expect("GENEALOGY_DATABASE_URL parses");
    let database = config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    Some(dsn)
}

struct TestDb {
    genealogy: Client,
    dsn: String,
    prefix: String,
    _guard: MutexGuard<'static, ()>,
}

impl TestDb {
    fn new() -> Option<Self> {
        let dsn = dsn()?;
        let guard = DB_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let mut genealogy = Client::connect(&dsn, NoTls).expect("genealogy database");
        for path in MIGRATIONS {
            let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
            genealogy
                .batch_execute(&sql)
                .unwrap_or_else(|err| panic!("apply {path}: {err}"));
        }
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        Some(Self {
            genealogy,
            dsn,
            prefix: format!("synthetic-restriction-{}-{nanos}", std::process::id()),
            _guard: guard,
        })
    }

    fn account(&mut self, label: &str) -> IdentityAccount {
        let id: i64 = self
            .genealogy
            .query_one(
                "SELECT (EXTRACT(EPOCH FROM now()) * 1000000)::bigint + $1",
                &[&(label.len() as i64 + self.prefix.len() as i64)],
            )
            .expect("account id")
            .get(0);
        IdentityAccount {
            account_id: id,
            identity_fingerprint: format!("{id:064x}"),
        }
    }

    fn person(&mut self, label: &str) -> (i64, String) {
        let key = format!("{}-person-{label}", self.prefix);
        let id = self
            .genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Restriction Person {label}")],
            )
            .expect("person")
            .get(0);
        (id, key)
    }

    fn link(&mut self, account: &IdentityAccount, person_id: i64) {
        self.genealogy
            .execute(
                "INSERT INTO onboarding_person_account_link \
                     (person_id, account_id, linked_by, identity_fingerprint) \
                 VALUES ($1, $2, 'synthetic-admin', $3)",
                &[
                    &person_id,
                    &account.account_id,
                    &account.identity_fingerprint,
                ],
            )
            .expect("link");
    }

    /// The disposable database is reused between runs, so a seeded version is
    /// derived from the field's current history rather than assuming the field
    /// starts empty.
    fn next_revision_number(&mut self, person_id: i64, field_key: &str) -> i64 {
        self.genealogy
            .query_one(
                "SELECT COALESCE(MAX(revision_number), 0) + 1 \
                   FROM onboarding_profile_revision \
                  WHERE person_id = $1 AND field_key = $2",
                &[&person_id, &field_key],
            )
            .expect("next revision number")
            .get(0)
    }

    /// Revisions are seeded by a per-run actor so a reused disposable database
    /// cannot collide with another run's open revision for the same person id.
    fn owner_actor(&self) -> String {
        format!("{}-owner", self.prefix)
    }

    /// Seeds an approved value through the real draft -> submitted -> approved
    /// path, which is what the value guard requires.
    fn approved(&mut self, person_id: i64, field_key: &str, value: &str, visibility: &str) {
        let revision_id = self.revision(person_id, field_key, value, visibility);
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'pending_review', submitted_at = now() \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("submit seed revision");
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'approved', reviewed_at = now(), \
                        reviewed_by = 'synthetic-reviewer', review_note = 'seed' \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("approve seed revision");
        let revision_number: i64 = self
            .genealogy
            .query_one(
                "SELECT revision_number FROM onboarding_profile_revision WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("seeded revision number")
            .get(0);
        self.genealogy
            .execute(
                "INSERT INTO onboarding_profile_value \
                     (person_id, field_key, value_text, visibility, updated_by, \
                      approved_revision_number, approved_revision_id) \
                 VALUES ($1, $2, $3, $4, 'synthetic-owner', $5, $6)",
                &[
                    &person_id,
                    &field_key,
                    &value,
                    &visibility,
                    &revision_number,
                    &revision_id,
                ],
            )
            .expect("seed approved value");
    }

    fn revision(&mut self, person_id: i64, field_key: &str, value: &str, visibility: &str) -> i64 {
        let revision_number = self.next_revision_number(person_id, field_key);
        let actor = self.owner_actor();
        // A draft captures the currently approved snapshot, exactly as the
        // product does; approval rejects a draft whose snapshot no longer
        // matches.
        let prior = self
            .genealogy
            .query_opt(
                "SELECT value_text, approved_revision_number \
                   FROM onboarding_profile_value WHERE person_id = $1 AND field_key = $2",
                &[&person_id, &field_key],
            )
            .expect("prior value");
        let (prior_text, prior_number): (Option<String>, i64) = match prior {
            Some(row) => (Some(row.get(0)), row.get(1)),
            None => (None, 0),
        };
        self.genealogy
            .query_one(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 'draft', $8) \
                 RETURNING revision_id",
                &[
                    &person_id,
                    &field_key,
                    &prior_text,
                    &prior_number,
                    &value,
                    &actor,
                    &revision_number,
                    &visibility,
                ],
            )
            .expect("revision")
            .get(0)
    }

    fn submit(&mut self, revision_id: i64) {
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'pending_review', submitted_at = now() \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("submit revision");
    }

    fn restriction_events(&mut self, person_id: i64, field_key: &str) -> Vec<String> {
        self.genealogy
            .query(
                "SELECT event FROM onboarding_profile_field_restriction_event \
                  WHERE person_id = $1 AND field_key = $2 ORDER BY event_id",
                &[&person_id, &field_key],
            )
            .expect("events")
            .into_iter()
            .map(|row| row.get(0))
            .collect()
    }

    fn rag_document_count(&self) -> Option<i64> {
        let dsn = std::env::var("DATABASE_URL").ok()?;
        if dsn.trim().is_empty() {
            return None;
        }
        let mut client = Client::connect(&dsn, NoTls).ok()?;
        let exists: bool = client
            .query_one("SELECT to_regclass('public.documents') IS NOT NULL", &[])
            .ok()?
            .get(0);
        if !exists {
            return None;
        }
        client
            .query_one("SELECT count(*) FROM documents", &[])
            .ok()
            .map(|row| row.get(0))
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pattern = format!("{}%", self.prefix);
        // Restrictions cascade with the person (migration 0021), and the
        // append-only restriction events deliberately outlive that cascade.
        // Profile values and revisions have no `person` cascade, and a
        // restriction references its approved revision with RESTRICT, so the
        // person ids are captured first: the person delete cascades the
        // restrictions, and only then can the revisions be removed.
        let _ = self
            .genealogy
            .execute("DROP TABLE IF EXISTS cleanup_person", &[]);
        let _ = self.genealogy.execute(
            "CREATE TEMP TABLE cleanup_person (person_id BIGINT PRIMARY KEY)",
            &[],
        );
        let _ = self.genealogy.execute(
            "INSERT INTO cleanup_person SELECT person_id FROM person WHERE person_key LIKE $1",
            &[&pattern],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_person_account_link \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM person WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        // The guard trigger permits this only once the person row is gone, and
        // the restriction's RESTRICT reference to the approved revision would
        // otherwise block the revision delete below.
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_profile_field_restriction \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_profile_value \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_profile_revision \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        let _ = self
            .genealogy
            .execute("DROP TABLE IF EXISTS cleanup_person", &[]);
    }
}

fn family_biography(db: &mut TestDb, tag: &str) -> (IdentityAccount, i64, String, String) {
    let owner = db.account(&format!("owner-{tag}"));
    let (person, person_key) = db.person(tag);
    db.link(&owner, person);
    let value = format!("{}-{tag}-family-biography", db.prefix);
    db.approved(person, "biography", &value, "family");
    (owner, person, person_key, value)
}

#[test]
fn owner_restriction_hides_family_value_and_keeps_it_private() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, person, person_key, value) = family_biography(&mut db, "basic");
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);

    // Visible before the restriction.
    let before = read_family_profile(&mut db.genealogy, &reader, &person_key).expect("family read");
    assert!(before.fields.iter().any(|field| field.value == value));

    let rag_before = db.rag_document_count();
    let outcome = restrict_own_field(&mut db.genealogy, &owner, "biography").expect("restrict");
    assert_eq!(outcome.generation, 1);
    assert!(!outcome.already_restricted);

    // Immediately absent from the family read for any other member.
    let after = read_family_profile(&mut db.genealogy, &reader, &person_key).expect("family read");
    assert!(after.fields.is_empty(), "restricted value is not presented");
    assert!(!format!("{after:?}").contains(&value));
    // The audience-filtered helper applies the same rule.
    let family_values = read_approved_values(&mut db.genealogy, person, ProfileAudience::Family)
        .expect("family approved values");
    assert!(family_values.is_empty());

    // The owner still sees it, the stored visibility is unchanged, and the
    // effective state is reported as restricted.
    let owner_view = read_owner_profile(&mut db.genealogy, &owner)
        .expect("owner profile")
        .expect("linked owner");
    let biography = owner_view
        .approved
        .iter()
        .find(|row| row.field_key == "biography")
        .expect("owner biography");
    assert_eq!(biography.value_text, value);
    assert_eq!(biography.visibility, ProfileVisibility::Family);
    assert!(biography.restricted_from_family);
    assert_eq!(biography.restriction_generation, Some(1));

    // Audit trail records the restriction once.
    assert_eq!(
        db.restriction_events(person, "biography"),
        vec!["restricted".to_string()]
    );

    // Historical approval evidence is untouched.
    let (status, visibility): (String, String) = db
        .genealogy
        .query_one(
            "SELECT status, visibility FROM onboarding_profile_revision \
              WHERE person_id = $1 AND field_key = 'biography'",
            &[&person],
        )
        .map(|row| (row.get(0), row.get(1)))
        .expect("revision");
    assert_eq!(
        (status.as_str(), visibility.as_str()),
        ("approved", "family")
    );

    if let Some(before) = rag_before {
        assert_eq!(
            db.rag_document_count(),
            Some(before),
            "restriction must not touch the retrieval store"
        );
    }
}

#[test]
fn restriction_requires_the_owned_person_and_an_approved_family_value() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, person, person_key, value) = family_biography(&mut db, "guards");
    let stranger = db.account("stranger");
    let (stranger_person, _) = db.person("stranger");
    db.link(&stranger, stranger_person);

    // An unrelated account can only act on its own person, where no such value
    // exists, so another person's field cannot be restricted.
    assert_eq!(
        restrict_own_field(&mut db.genealogy, &stranger, "biography"),
        Err(RestrictionError::NoApprovedValue)
    );
    let family =
        read_family_profile(&mut db.genealogy, &stranger, &person_key).expect("family read");
    assert!(family.fields.iter().any(|field| field.value == value));

    // A revoked link cannot restrict anything.
    db.genealogy
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE account_id = $1",
            &[&owner.account_id],
        )
        .expect("revoke link");
    assert_eq!(
        restrict_own_field(&mut db.genealogy, &owner, "biography"),
        Err(RestrictionError::Unlinked)
    );
    db.link(&owner, person);

    // Invalid and unknown fields are refused.
    assert_eq!(
        restrict_own_field(&mut db.genealogy, &owner, "not_a_field"),
        Err(RestrictionError::InvalidField)
    );
    assert_eq!(
        restrict_own_field(&mut db.genealogy, &owner, "my_story"),
        Err(RestrictionError::NoApprovedValue)
    );

    // An approved owner-private value is already private; it is not something
    // this endpoint can publish, so it is refused rather than "restricted".
    db.approved(person, "my_story", "private story", "owner_private");
    assert_eq!(
        restrict_own_field(&mut db.genealogy, &owner, "my_story"),
        Err(RestrictionError::NotFamilyVisible)
    );

    // Repeated restriction is idempotent and does not duplicate the audit trail.
    let first = restrict_own_field(&mut db.genealogy, &owner, "biography").expect("first");
    let second = restrict_own_field(&mut db.genealogy, &owner, "biography").expect("second");
    assert_eq!(second.generation, first.generation);
    assert!(second.already_restricted);
    assert_eq!(
        db.restriction_events(person, "biography"),
        vec!["restricted".to_string()]
    );
}

#[test]
fn a_stale_pending_approval_cannot_republish_a_restricted_field() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, person, person_key, value) = family_biography(&mut db, "stale");
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let reviewer = db.account("reviewer");
    let (reviewer_person, _) = db.person("reviewer");
    db.link(&reviewer, reviewer_person);
    let reviewer_id = reviewer.identity_fingerprint.clone();

    // A pending submission exists before the restriction.
    let pending = db.revision(person, "biography", &format!("{value}-updated"), "family");
    db.submit(pending);

    restrict_own_field(&mut db.genealogy, &owner, "biography").expect("restrict");

    // Approving it now would republish an outdated value, so the review is
    // refused and the field stays private.
    assert_eq!(
        review_revision(
            &mut db.genealogy,
            pending,
            &reviewer_id,
            ReviewDecision::Approve,
            None
        ),
        Err(RevisionError::RestrictionConflict)
    );
    let family = read_family_profile(&mut db.genealogy, &reader, &person_key).expect("family read");
    assert!(family.fields.is_empty());
    let status: String = db
        .genealogy
        .query_one(
            "SELECT status FROM onboarding_profile_revision WHERE revision_id = $1",
            &[&pending],
        )
        .expect("status")
        .get(0);
    assert_eq!(status, "pending_review", "a refused approval rolls back");
}

#[test]
fn a_post_restriction_family_approval_lifts_the_restriction() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, person, person_key, _value) = family_biography(&mut db, "lift");
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let reviewer = db.account("reviewer");
    let (reviewer_person, _) = db.person("reviewer");
    db.link(&reviewer, reviewer_person);
    let reviewer_id = reviewer.identity_fingerprint.clone();

    restrict_own_field(&mut db.genealogy, &owner, "biography").expect("restrict");

    // An owner-private approval after the restriction must not lift it.
    let private_revision = db.revision(person, "biography", "private rewrite", "owner_private");
    db.submit(private_revision);
    review_revision(
        &mut db.genealogy,
        private_revision,
        &reviewer_id,
        ReviewDecision::Approve,
        None,
    )
    .expect("approve owner-private revision");
    let still_private =
        read_family_profile(&mut db.genealogy, &reader, &person_key).expect("family read");
    assert!(
        still_private.fields.is_empty(),
        "owner-private approval does not expand"
    );
    assert_eq!(
        db.restriction_events(person, "biography"),
        vec!["restricted".to_string()]
    );

    // A new family-visible submission after the restriction, independently
    // approved, restores family visibility and records the lift.
    let expanded = db.revision(
        person,
        "biography",
        &format!("{}-expanded-family-biography", db.prefix),
        "family",
    );
    db.submit(expanded);
    review_revision(
        &mut db.genealogy,
        expanded,
        &reviewer_id,
        ReviewDecision::Approve,
        None,
    )
    .expect("approve expansion");
    let visible =
        read_family_profile(&mut db.genealogy, &reader, &person_key).expect("family read");
    assert!(visible
        .fields
        .iter()
        .any(|field| field.value.ends_with("expanded-family-biography")));
    assert_eq!(
        db.restriction_events(person, "biography"),
        vec!["restricted".to_string(), "lifted".to_string()]
    );
    let owner_view = read_owner_profile(&mut db.genealogy, &owner)
        .expect("owner profile")
        .expect("linked owner");
    let biography = owner_view
        .approved
        .iter()
        .find(|row| row.field_key == "biography")
        .expect("biography");
    assert!(!biography.restricted_from_family);
    assert_eq!(biography.restriction_generation, None);
}

#[test]
fn concurrent_restriction_and_approval_serialize() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let (owner, person, person_key, value) = family_biography(&mut db, "race");
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let reviewer = db.account("reviewer");
    let (reviewer_person, _) = db.person("reviewer");
    db.link(&reviewer, reviewer_person);

    // A pending family revision created before the restriction.
    let pending = db.revision(person, "biography", &format!("{value}-race"), "family");
    db.submit(pending);

    let barrier = Arc::new(Barrier::new(2));
    let dsn = db.dsn.clone();
    let owner_account = owner.clone();
    let reviewer_id = reviewer.identity_fingerprint.clone();
    let barrier_restrict = Arc::clone(&barrier);
    let restrict_handle = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("restriction connection");
        barrier_restrict.wait();
        restrict_own_field(&mut client, &owner_account, "biography")
    });
    let dsn = db.dsn.clone();
    let barrier_approve = Arc::clone(&barrier);
    let approve_handle = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("approval connection");
        barrier_approve.wait();
        review_revision(
            &mut client,
            pending,
            &reviewer_id,
            ReviewDecision::Approve,
            None,
        )
    });

    let restricted = restrict_handle.join().expect("restriction thread");
    let approved = approve_handle.join().expect("approval thread");
    assert!(restricted.is_ok(), "the restriction must commit");

    // Whatever the interleaving, the end state is fail-closed: the value is not
    // family-visible. Either the approval ran first (and was then restricted) or
    // it was refused as stale.
    if let Err(err) = approved {
        assert_eq!(err, RevisionError::RestrictionConflict);
    }
    let family = read_family_profile(&mut db.genealogy, &reader, &person_key).expect("family read");
    assert!(
        family.fields.is_empty(),
        "no family value survives the race"
    );
    let active: i64 = db
        .genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_profile_field_restriction \
              WHERE person_id = $1 AND field_key = 'biography' AND lifted_at IS NULL",
            &[&person],
        )
        .expect("active restriction")
        .get(0);
    assert_eq!(active, 1);
}
