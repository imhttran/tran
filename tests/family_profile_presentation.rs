//! USR-006-A: approved-only family profile presentation.
//!
//! Synthetic PostgreSQL only. The suite uses `GENEALOGY_DATABASE_URL`, applies
//! every genealogy migration, and cleans only rows carrying its own prefix.

use std::fs;
use std::str::FromStr;
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    read_family_profile, read_owner_profile, FamilyProfileError, IdentityAccount, ProfileVisibility,
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
        eprintln!("GENEALOGY_DATABASE_URL unset; skipping family profile presentation test");
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
            prefix: format!("synthetic-family-profile-{}-{nanos}", std::process::id()),
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
                &[&key, &format!("Synthetic Family Person {label}")],
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

    /// Drafts are seeded by a per-run actor so a reused disposable database
    /// cannot collide with another run's open revision for the same person id.
    fn owner_actor(&self) -> String {
        format!("{}-owner", self.prefix)
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

    /// Seeds an approved value through the real draft -> submitted -> approved
    /// path, which is what the value guard requires.
    fn approved(&mut self, person_id: i64, field_key: &str, value: &str, visibility: &str) {
        let revision_number = self.next_revision_number(person_id, field_key);
        let actor = self.owner_actor();
        let revision_id: i64 = self
            .genealogy
            .query_one(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, NULL, 0, $3, $4, $5, 'draft', $6) \
                 RETURNING revision_id",
                &[
                    &person_id,
                    &field_key,
                    &value,
                    &actor,
                    &revision_number,
                    &visibility,
                ],
            )
            .expect("seed revision")
            .get(0);
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

    /// A draft that is never approved: it must not reach a family read.
    fn draft(&mut self, person_id: i64, field_key: &str, value: &str) {
        let revision_number = self.next_revision_number(person_id, field_key);
        let actor = self.owner_actor();
        self.genealogy
            .execute(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, NULL, 0, $3, $4, $5, 'draft', 'family')",
                &[&person_id, &field_key, &value, &actor, &revision_number],
            )
            .expect("seed draft");
    }

    /// Inserts a draft revision and returns its id.
    fn draft_revision_id(&mut self, person_id: i64, field_key: &str, value: &str) -> i64 {
        let revision_number = self.next_revision_number(person_id, field_key);
        let actor = self.owner_actor();
        self.genealogy
            .query_one(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, NULL, 0, $3, $4, $5, 'draft', 'family') \
                 RETURNING revision_id",
                &[&person_id, &field_key, &value, &actor, &revision_number],
            )
            .expect("draft revision")
            .get(0)
    }

    /// Runs a closure with the value guard disabled, then always re-enables it.
    ///
    /// Only used to simulate rows a legacy or out-of-band writer could leave
    /// behind; foreign keys and check constraints still apply. Disabling a user
    /// trigger requires table ownership, which the project's disposable
    /// databases grant to the role that applies migrations (the same role the
    /// tests connect as).
    fn without_value_guard<F>(&mut self, writes: F) -> Result<(), postgres::Error>
    where
        F: FnOnce(&mut Client) -> Result<(), postgres::Error>,
    {
        self.genealogy
            .batch_execute("ALTER TABLE onboarding_profile_value DISABLE TRIGGER USER")?;
        let outcome = writes(&mut self.genealogy);
        let restored = self
            .genealogy
            .batch_execute("ALTER TABLE onboarding_profile_value ENABLE TRIGGER USER");
        outcome.and(restored)
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
        Some(
            client
                .query_one("SELECT count(*) FROM documents", &[])
                .ok()?
                .get(0),
        )
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pattern = format!("{}%", self.prefix);
        // Only rows this test created, always matched by its unique prefix.
        //
        // `onboarding_profile_revision` has no `person` cascade, and a
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
            "DELETE FROM onboarding_profile_review_event \
              WHERE revision_id IN (SELECT revision_id FROM onboarding_profile_revision \
                                     WHERE person_id IN (SELECT person_id FROM cleanup_person))",
            &[],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_media_metadata \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_profile_value \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
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
            "DELETE FROM onboarding_profile_revision \
              WHERE person_id IN (SELECT person_id FROM cleanup_person)",
            &[],
        );
        let _ = self
            .genealogy
            .execute("DROP TABLE IF EXISTS cleanup_person", &[]);
    }
}

#[test]
fn family_read_returns_only_approved_family_visible_values() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let (subject, subject_key) = db.person("subject");

    let family_marker = format!("{}-family-biography", db.prefix);
    let private_marker = format!("{}-private-location", db.prefix);
    db.approved(subject, "display_name", "Family Name", "family");
    db.approved(subject, "biography", &family_marker, "family");
    db.approved(
        subject,
        "current_location",
        &private_marker,
        "owner_private",
    );
    db.approved(subject, "preferred_name", "Nickname", "family");
    db.draft(subject, "my_story", "Unsubmitted family story");

    let profile = read_family_profile(&mut db.genealogy, &reader, &subject_key)
        .expect("family profile for a linked member");

    let keys: Vec<&str> = profile
        .fields
        .iter()
        .map(|f| f.field_key.as_str())
        .collect();
    assert_eq!(keys, vec!["biography", "display_name", "preferred_name"]);
    assert_eq!(profile.display_name, "Synthetic Family Person subject");
    let biography = profile
        .fields
        .iter()
        .find(|field| field.field_key == "biography")
        .expect("biography");
    assert_eq!(biography.value, family_marker);
    assert_eq!(biography.visibility, "family");
    assert_eq!(biography.approved_revision_number, 1);
    assert!(!biography.updated_at.is_empty());

    // Neither the private value nor the unsubmitted draft is readable.
    let rendered = format!("{profile:?}");
    assert!(!rendered.contains(&private_marker));
    assert!(!rendered.contains("Unsubmitted family story"));
    assert!(profile
        .fields
        .iter()
        .all(|field| field.field_key != "current_location"));
    assert!(profile
        .fields
        .iter()
        .all(|field| field.field_key != "my_story"));
}

#[test]
fn family_read_never_returns_media_references_or_another_persons_values() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let (subject, subject_key) = db.person("subject");
    let (other, other_key) = db.person("other");

    db.approved(subject, "biography", "Subject public biography", "family");
    // Media references are approved values, but presentation has its own rule.
    db.approved(subject, "profile_photo", "12", "family");
    db.approved(subject, "personal_document", "13", "family");

    let other_marker = format!("{}-other-private", db.prefix);
    db.approved(other, "current_location", &other_marker, "owner_private");
    db.approved(other, "biography", "Other family biography", "family");

    let profile = read_family_profile(&mut db.genealogy, &reader, &subject_key).expect("subject");
    let keys: Vec<&str> = profile
        .fields
        .iter()
        .map(|f| f.field_key.as_str())
        .collect();
    assert_eq!(keys, vec!["biography"]);
    assert!(!format!("{profile:?}").contains(&other_marker));

    // Reading the other person returns only that person's family value.
    let other_profile =
        read_family_profile(&mut db.genealogy, &reader, &other_key).expect("other person");
    assert_eq!(other_profile.fields.len(), 1);
    assert_eq!(other_profile.fields[0].value, "Other family biography");
    assert!(!format!("{other_profile:?}").contains(&other_marker));
}

#[test]
fn unlinked_members_cannot_read_a_family_profile() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let stranger = db.account("stranger");
    let (subject, subject_key) = db.person("subject");
    db.approved(subject, "biography", "Family biography", "family");

    // A stranger is refused before the person is looked up, so an existing and
    // a missing key produce the same answer.
    assert_eq!(
        read_family_profile(&mut db.genealogy, &stranger, &subject_key),
        Err(FamilyProfileError::NotFamilyMember)
    );
    assert_eq!(
        read_family_profile(&mut db.genealogy, &stranger, "missing-person-key"),
        Err(FamilyProfileError::NotFamilyMember)
    );
}

#[test]
fn revoked_link_and_unknown_person_fail_closed() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let (subject, subject_key) = db.person("subject");
    db.approved(subject, "biography", "Family biography", "family");

    assert!(read_family_profile(&mut db.genealogy, &reader, &subject_key).is_ok());

    // Revoking the reviewed link is immediate: the next read is refused.
    db.genealogy
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE person_id = $1",
            &[&reader_person],
        )
        .expect("revoke link");
    assert_eq!(
        read_family_profile(&mut db.genealogy, &reader, &subject_key),
        Err(FamilyProfileError::NotFamilyMember)
    );

    // A linked member asking for an unknown key gets a plain not-found.
    db.link(&reader, reader_person);
    assert_eq!(
        read_family_profile(&mut db.genealogy, &reader, "missing-person-key"),
        Err(FamilyProfileError::UnknownPerson)
    );
}

#[test]
fn authorized_profiles_without_family_values_are_empty_not_private() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let (subject, subject_key) = db.person("subject");
    db.approved(
        subject,
        "current_location",
        "Private address",
        "owner_private",
    );

    let profile = read_family_profile(&mut db.genealogy, &reader, &subject_key).expect("profile");
    assert!(profile.fields.is_empty());
    assert_eq!(profile.person_key, subject_key);
    assert!(!format!("{profile:?}").contains("Private address"));
}

#[test]
fn family_read_does_not_change_the_owners_private_view_or_create_retrieval_rows() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let owner = db.account("owner");
    let (subject, subject_key) = db.person("subject");
    db.link(&owner, subject);
    db.approved(subject, "biography", "Family biography", "family");
    db.approved(
        subject,
        "current_location",
        "Private address",
        "owner_private",
    );

    let rag_before = db.rag_document_count();
    read_family_profile(&mut db.genealogy, &reader, &subject_key).expect("family read");

    // The owner's own view still contains both approved values.
    let owner_view = read_owner_profile(&mut db.genealogy, &owner)
        .expect("owner profile")
        .expect("linked owner");
    let owner_keys: Vec<&str> = owner_view
        .approved
        .iter()
        .map(|value| value.field_key.as_str())
        .collect();
    assert_eq!(owner_keys, vec!["biography", "current_location"]);
    let private = owner_view
        .approved
        .iter()
        .find(|value| value.field_key == "current_location")
        .expect("private value");
    assert_eq!(private.visibility, ProfileVisibility::OwnerPrivate);

    if let Some(before) = rag_before {
        assert_eq!(
            db.rag_document_count(),
            Some(before),
            "presenting a profile must not create retrieval documents"
        );
    }
}

#[test]
fn inconsistent_or_missing_provenance_is_never_presented() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let reader = db.account("reader");
    let (reader_person, _) = db.person("reader");
    db.link(&reader, reader_person);
    let (subject, subject_key) = db.person("subject");

    // One fully consistent value written through the product path.
    let valid = format!("{}-valid-biography", db.prefix);
    db.approved(subject, "biography", &valid, "family");

    // One family value per provenance clause, each made inconsistent only in
    // that clause. The write-time guard enforces all of them, so these rows can
    // only come from a legacy or out-of-band writer; the read must fail closed
    // rather than trust them.
    db.approved(subject, "preferred_name", "Revision name", "family");
    db.approved(subject, "family_memories", "Family memory", "owner_private");
    db.approved(subject, "whatsapp", "+0000000000", "family");
    db.approved(subject, "my_story", "A long story", "family");
    let draft_revision = db.draft_revision_id(subject, "contact_preferences", "Draft text");

    db.without_value_guard(|genealogy| {
        // 1. Value text no longer matches the approved revision.
        genealogy.execute(
            "UPDATE onboarding_profile_value SET value_text = 'Mismatched text' \
              WHERE person_id = $1 AND field_key = 'preferred_name'",
            &[&subject],
        )?;
        // 2. Visibility claims family, the approved revision says otherwise.
        genealogy.execute(
            "UPDATE onboarding_profile_value SET visibility = 'family' \
              WHERE person_id = $1 AND field_key = 'family_memories'",
            &[&subject],
        )?;
        // 3. Revision number no longer matches the approved revision.
        genealogy.execute(
            "UPDATE onboarding_profile_value SET approved_revision_number = 99 \
              WHERE person_id = $1 AND field_key = 'my_story'",
            &[&subject],
        )?;
        // 4. The value points at a revision that is not approved.
        genealogy.execute(
            "UPDATE onboarding_profile_value SET approved_revision_id = $2 \
              WHERE person_id = $1 AND field_key = 'whatsapp'",
            &[&subject, &draft_revision],
        )?;
        // 5. A legacy row with no approved revision at all: migration 0012
        // leaves pre-existing values without a backfilled revision.
        genealogy.execute(
            "INSERT INTO onboarding_profile_value \
                 (person_id, field_key, value_text, visibility, updated_by, \
                  approved_revision_number) \
             VALUES ($1, 'current_location', 'Legacy unprovenanced value', 'family', \
                     'legacy', 0)",
            &[&subject],
        )?;
        Ok(())
    })
    .expect("simulate inconsistent legacy rows");

    let profile = read_family_profile(&mut db.genealogy, &reader, &subject_key)
        .expect("family profile with inconsistent rows present");

    // Only the value with a complete, matching provenance is presented.
    assert_eq!(
        profile
            .fields
            .iter()
            .map(|field| field.field_key.as_str())
            .collect::<Vec<_>>(),
        vec!["biography"]
    );
    let rendered = format!("{profile:?}");
    for excluded in [
        "Mismatched text",
        "Family memory",
        "+0000000000",
        "A long story",
        "Legacy unprovenanced value",
    ] {
        assert!(
            !rendered.contains(excluded),
            "inconsistent provenance leaked: {excluded}"
        );
    }
}
