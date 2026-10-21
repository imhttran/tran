//! USR-004A owner-scoped profile read tests.
//!
//! These tests use the disposable identity and genealogy databases only. They
//! create synthetic accounts, persons, links, and private field values, then
//! prove that a caller only ever sees the profile owned by its own reviewed
//! link. Every test skips when the two DSNs are absent and refuses a genealogy
//! destination that is not clearly disposable or that equals the identity
//! database.

use std::fs;
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    link_for_account, link_reviewed, read_own_profile, resolve_account, revoke_link,
    AccountMapError, IdentityAccount, LinkDecision, ProfileView,
};

const IDENTITY_MIGRATIONS: [&str; 3] = [
    "migrations/006_identity.sql",
    "migrations/007_user_lifecycle.sql",
    "migrations/008_identity_issuer_subject.sql",
];

const GENEALOGY_MIGRATIONS: [&str; 14] = [
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
];

const ISSUER: &str = "https://synthetic-issuer.example";

/// Tables that reference `person(person_id)`. The profile test cleans these
/// explicitly because another integration test drops `person` with `CASCADE`,
/// which removes the child foreign keys but leaves the child tables in place.
/// Explicit child deletes keep this test isolated from that shared-database
/// schema churn.
const PERSON_CHILD_TABLES: [&str; 7] = [
    "onboarding_profile_revision",
    "onboarding_profile_value",
    "onboarding_media_metadata",
    "onboarding_link_audit",
    "onboarding_person_account_link",
    "onboarding_invitation",
    "onboarding_username_reservation",
];

/// Serializes the migration setup. Each test owns its own unique row prefix, so
/// only the DDL needs the lock.
static DB_LOCK: Mutex<()> = Mutex::new(());

fn dsns() -> Option<(String, String)> {
    let identity = std::env::var("DATABASE_URL").ok()?;
    let genealogy = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let identity = identity.trim().to_string();
    let genealogy = genealogy.trim().to_string();
    if identity.is_empty() || genealogy.is_empty() {
        eprintln!(
            "DATABASE_URL or GENEALOGY_DATABASE_URL unset; skipping profile integration test"
        );
        return None;
    }

    let identity_config = postgres::Config::from_str(&identity).expect("DATABASE_URL parses");
    let genealogy_config =
        postgres::Config::from_str(&genealogy).expect("GENEALOGY_DATABASE_URL parses");
    let database = genealogy_config.get_dbname().unwrap_or_default();
    assert!(
        database.ends_with("_test") || database.ends_with("_disposable"),
        "refusing to write to non-disposable genealogy database {database:?}"
    );
    assert_ne!(
        (
            genealogy_config.get_dbname(),
            genealogy_config.get_hosts(),
            genealogy_config.get_ports()
        ),
        (
            identity_config.get_dbname(),
            identity_config.get_hosts(),
            identity_config.get_ports()
        ),
        "the profile integration test must not target the identity database"
    );
    Some((identity, genealogy))
}

struct TestDb {
    identity: Client,
    genealogy: Client,
    genealogy_dsn: String,
    prefix: String,
    _guard: MutexGuard<'static, ()>,
}

impl TestDb {
    fn new() -> Option<Self> {
        let (identity_dsn, genealogy_dsn) = dsns()?;
        let guard = DB_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        let mut identity = Client::connect(&identity_dsn, NoTls).expect("identity database");
        let mut genealogy = Client::connect(&genealogy_dsn, NoTls).expect("genealogy database");
        apply_migrations(&mut identity, &IDENTITY_MIGRATIONS);
        apply_migrations(&mut genealogy, &GENEALOGY_MIGRATIONS);
        clear_orphan_child_rows(&mut genealogy);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        Some(Self {
            identity,
            genealogy,
            genealogy_dsn,
            prefix: format!("synthetic-profile-{}-{nanos}", std::process::id()),
            _guard: guard,
        })
    }

    fn account(&mut self, label: &str) -> IdentityAccount {
        let subject = format!("{}-user-{label}", self.prefix);
        self.identity
            .execute(
                "INSERT INTO users (subject, issuer, display_name) VALUES ($1, $2, $3)",
                &[&subject, &ISSUER, &format!("Synthetic {label}")],
            )
            .expect("insert synthetic identity");
        resolve_account(&mut self.identity, ISSUER, &subject).expect("resolve synthetic identity")
    }

    fn deactivate(&mut self, label: &str) {
        let subject = format!("{}-user-{label}", self.prefix);
        self.identity
            .execute(
                "UPDATE users SET deactivated_at = now() WHERE subject = $1 AND issuer = $2",
                &[&subject, &ISSUER],
            )
            .expect("deactivate synthetic identity");
    }

    fn person(&mut self, label: &str) -> i64 {
        let key = format!("{}-person-{label}", self.prefix);
        self.genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Person {label}")],
            )
            .expect("insert synthetic person")
            .get(0)
    }

    fn link(&mut self, account: &IdentityAccount, person_id: i64) {
        link_reviewed(
            &mut self.genealogy,
            person_id,
            account,
            LinkDecision::Approve,
            "synthetic-reviewer",
        )
        .expect("link synthetic account");
    }

    fn field(&mut self, person_id: i64, key: &str, value: &str) {
        let revision_id: i64 = self
            .genealogy
            .query_one(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, NULL, 0, $3, 'synthetic-member', 1, 'draft', 'owner_private') \
                 RETURNING revision_id",
                &[&person_id, &key, &value],
            )
            .expect("insert synthetic profile revision")
            .get(0);
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'pending_review', submitted_at = now() \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("submit synthetic profile revision");
        self.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'approved', reviewed_at = now(), \
                        reviewed_by = 'synthetic-reviewer', review_note = 'seed' \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("approve synthetic profile revision");
        self.genealogy
            .execute(
                "INSERT INTO onboarding_profile_value \
                     (person_id, field_key, value_text, updated_by, \
                      approved_revision_number, approved_revision_id) \
                 VALUES ($1, $2, $3, 'synthetic-member', 1, $4)",
                &[&person_id, &key, &value, &revision_id],
            )
            .expect("write synthetic profile field");
    }

    fn read(&mut self, account: &IdentityAccount) -> Option<ProfileView> {
        read_own_profile(&mut self.genealogy, account).expect("read synthetic profile")
    }

    fn extra_genealogy(&self) -> Client {
        Client::connect(&self.genealogy_dsn, NoTls).expect("second genealogy connection")
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pattern = format!("{}%", self.prefix);
        // Delete child rows first. A prior `DROP TABLE person CASCADE` may have
        // removed the child foreign keys, so relying on cascade alone would
        // leave private values behind for a reused person id.
        for table in PERSON_CHILD_TABLES {
            let sql = format!(
                "DELETE FROM {table} AS child USING person p \
                  WHERE child.person_id = p.person_id AND p.person_key LIKE $1"
            );
            let _ = self.genealogy.execute(&sql, &[&pattern]);
        }
        let _ = self
            .genealogy
            .execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
        let _ = self
            .identity
            .execute("DELETE FROM users WHERE subject LIKE $1", &[&pattern]);
    }
}

/// Removes child rows whose person row no longer exists. This can happen when
/// another integration test resets the `person` table while the onboarding
/// child tables survive; a later insert may then reuse a person id.
fn clear_orphan_child_rows(genealogy: &mut Client) {
    for table in PERSON_CHILD_TABLES {
        let sql = format!(
            "DELETE FROM {table} AS child \
              WHERE NOT EXISTS (SELECT 1 FROM person p WHERE p.person_id = child.person_id)"
        );
        let _ = genealogy.execute(&sql, &[]);
    }
}

fn apply_migrations(client: &mut Client, paths: &[&str]) {
    for path in paths {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn field_value<'a>(profile: &'a ProfileView, key: &str) -> Option<&'a str> {
    profile
        .fields
        .iter()
        .find(|field| field.field_key == key)
        .map(|field| field.value.as_str())
}

#[test]
fn linked_account_reads_only_its_own_private_profile() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account_a = db.account("a");
    let person_a = db.person("a");
    db.link(&account_a, person_a);
    db.field(person_a, "biography", "A private biography");

    let account_b = db.account("b");
    let person_b = db.person("b");
    db.link(&account_b, person_b);
    db.field(person_b, "biography", "B private biography");

    let profile_a = db.read(&account_a).expect("account A is linked");
    assert_eq!(profile_a.display_name, "Synthetic Person a");
    assert_eq!(
        field_value(&profile_a, "biography"),
        Some("A private biography")
    );
    assert!(
        profile_a
            .fields
            .iter()
            .all(|field| field.value != "B private biography"),
        "account A must never receive account B's private field"
    );

    let profile_b = db.read(&account_b).expect("account B is linked");
    assert_eq!(profile_b.display_name, "Synthetic Person b");
    assert_eq!(
        field_value(&profile_b, "biography"),
        Some("B private biography")
    );
    assert!(
        profile_b
            .fields
            .iter()
            .all(|field| field.value != "A private biography"),
        "account B must never receive account A's private field"
    );
}

#[test]
fn fingerprint_mismatch_returns_no_profile() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account_a = db.account("fingerprint-a");
    let person_a = db.person("fingerprint-a");
    db.link(&account_a, person_a);
    db.field(person_a, "biography", "A fingerprint private biography");

    let account_b = db.account("fingerprint-b");
    let person_b = db.person("fingerprint-b");
    db.link(&account_b, person_b);
    db.field(person_b, "biography", "B fingerprint private biography");

    // The account id alone is not ownership: a row paired with another
    // identity's fingerprint must not be readable.
    let mismatched = IdentityAccount {
        account_id: account_a.account_id,
        identity_fingerprint: account_b.identity_fingerprint.clone(),
    };
    assert_eq!(db.read(&mismatched), None);

    // The same check at the link lookup boundary.
    assert_eq!(
        link_for_account(
            &mut db.genealogy,
            account_a.account_id,
            &account_b.identity_fingerprint
        )
        .expect("lookup mismatch"),
        None
    );
}

#[test]
fn unlinked_reader_cannot_see_a_populated_person() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let private_person = db.person("populated-private");
    db.field(private_person, "biography", "populated private biography");

    let stranger = db.account("unlinked-stranger");
    assert_eq!(
        db.read(&stranger),
        None,
        "an unlinked account must not receive another person's private values"
    );
}

#[test]
fn concurrent_revocation_is_atomic_with_the_profile_read() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("atomic-revoke");
    let person = db.person("atomic-revoke");
    db.link(&account, person);
    db.field(person, "biography", "before concurrent revoke");

    let link = link_for_account(
        &mut db.genealogy,
        account.account_id,
        &account.identity_fingerprint,
    )
    .expect("find active link")
    .expect("active link exists");

    let mut revoker = db.extra_genealogy();
    let mut revoke_tx = revoker.transaction().expect("begin revoke transaction");
    revoke_tx
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE link_id = $1",
            &[&link.link_id],
        )
        .expect("delete link in uncommitted transaction");

    // The revoke is not committed yet, so a concurrent read still sees the
    // last committed link and its owner's values.
    let before = db.read(&account).expect("committed link remains visible");
    assert_eq!(
        field_value(&before, "biography"),
        Some("before concurrent revoke")
    );

    revoke_tx.commit().expect("commit revoke");
    assert_eq!(
        db.read(&account),
        None,
        "after the revoke commits, the same read must return no private values"
    );
}

#[test]
fn contended_revocation_cannot_interleave_with_the_profile_read() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("contended-revoke");
    let person = db.person("contended-revoke");
    db.link(&account, person);
    db.field(person, "biography", "contended private biography");

    let link = link_for_account(
        &mut db.genealogy,
        account.account_id,
        &account.identity_fingerprint,
    )
    .expect("find active link")
    .expect("active link exists");

    // Hold a row lock so the revoke cannot complete while the read runs. The
    // profile read must still see the last committed link, and it must not
    // return any value after the revoke finally commits.
    let mut reader = db.extra_genealogy();
    let mut blocker = db.genealogy.transaction().expect("begin link-lock holder");
    blocker
        .query_one(
            "SELECT link_id FROM onboarding_person_account_link \
              WHERE link_id = $1 FOR UPDATE",
            &[&link.link_id],
        )
        .expect("lock active link row");

    let (ready_tx, ready_rx) = mpsc::channel();
    let dsn = db.genealogy_dsn.clone();
    let link_id = link.link_id;
    let deleter = thread::spawn(move || {
        let mut client = Client::connect(&dsn, NoTls).expect("revoker connection");
        let pid: i32 = client
            .query_one("SELECT pg_backend_pid()", &[])
            .expect("read revoker backend pid")
            .get(0);
        ready_tx.send(pid).expect("signal revoker");
        client
            .execute(
                "DELETE FROM onboarding_person_account_link WHERE link_id = $1",
                &[&link_id],
            )
            .expect("delete after the row lock is released")
    });
    let revoker_pid: i32 = ready_rx.recv().expect("revoker started");

    // Wait until PostgreSQL reports the revoke is actually blocked on the row
    // lock instead of relying on a timing sleep.
    let mut blocked = false;
    for _ in 0..200 {
        let waiting: bool = reader
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity \
                  WHERE pid = $1 AND wait_event_type = 'Lock')",
                &[&revoker_pid],
            )
            .expect("inspect revoker wait state")
            .get(0);
        if waiting {
            blocked = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(blocked, "revoker did not block on the link row lock");

    let during = read_own_profile(&mut reader, &account)
        .expect("read while the revoke waits")
        .expect("the committed link is still visible");
    assert_eq!(
        field_value(&during, "biography"),
        Some("contended private biography")
    );

    blocker.commit().expect("release the link row lock");
    let deleted = deleter.join().expect("join revoker thread");
    assert_eq!(deleted, 1);
    assert_eq!(
        read_own_profile(&mut reader, &account).expect("read after the revoke commits"),
        None
    );
}

#[test]
fn concurrent_replacement_never_returns_the_previous_owner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("atomic-replace");
    let person_a = db.person("atomic-replace-a");
    let person_b = db.person("atomic-replace-b");
    db.link(&account, person_a);
    db.field(person_a, "biography", "owner A biography");
    db.field(person_b, "biography", "owner B biography");

    let link = link_for_account(
        &mut db.genealogy,
        account.account_id,
        &account.identity_fingerprint,
    )
    .expect("find active link")
    .expect("active link exists");

    let mut replacer = db.extra_genealogy();
    let mut replace_tx = replacer
        .transaction()
        .expect("begin replacement transaction");
    replace_tx
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE link_id = $1",
            &[&link.link_id],
        )
        .expect("remove old link");
    replace_tx
        .execute(
            "INSERT INTO onboarding_person_account_link \
                 (person_id, account_id, identity_fingerprint, linked_by) \
             VALUES ($1, $2, $3, 'synthetic-reviewer')",
            &[
                &person_b,
                &account.account_id,
                &account.identity_fingerprint,
            ],
        )
        .expect("insert replacement link");

    // The replacement is not committed yet, so the read still sees owner A.
    let during = db
        .read(&account)
        .expect("old committed link remains visible");
    assert_eq!(during.display_name, "Synthetic Person atomic-replace-a");
    assert_eq!(field_value(&during, "biography"), Some("owner A biography"));

    replace_tx.commit().expect("commit replacement");
    let after = db.read(&account).expect("replacement link is readable");
    assert_eq!(after.display_name, "Synthetic Person atomic-replace-b");
    assert_eq!(field_value(&after, "biography"), Some("owner B biography"));
    assert!(
        after
            .fields
            .iter()
            .all(|field| field.value != "owner A biography"),
        "the previous owner's private value must not survive replacement"
    );
}

#[test]
fn unlinked_and_revoked_accounts_receive_no_private_fields() {
    let Some(mut db) = TestDb::new() else {
        return;
    };

    let unlinked = db.account("unlinked");
    assert_eq!(db.read(&unlinked), None);

    let revoked = db.account("revoked");
    let person = db.person("revoked");
    db.link(&revoked, person);
    db.field(person, "biography", "revoked private biography");
    assert!(db.read(&revoked).is_some(), "link starts active");

    let link = link_for_account(
        &mut db.genealogy,
        revoked.account_id,
        &revoked.identity_fingerprint,
    )
    .expect("find active link")
    .expect("active link exists");
    revoke_link(&mut db.genealogy, link.link_id, "synthetic-reviewer").expect("revoke link");

    assert_eq!(
        db.read(&revoked),
        None,
        "a revoked link must not expose any private field"
    );
}

#[test]
fn inactive_identity_is_rejected_before_the_profile_is_read() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("inactive");
    let person = db.person("inactive");
    db.link(&account, person);
    db.field(person, "biography", "inactive private biography");
    db.deactivate("inactive");

    // The link and private value still exist, but no account can be minted for
    // the deactivated identity, so the profile path is unreachable.
    assert!(
        link_for_account(
            &mut db.genealogy,
            account.account_id,
            &account.identity_fingerprint
        )
        .expect("find link")
        .is_some(),
        "the inactive link remains until an admin revokes it"
    );
    let private_values: i64 = db
        .genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_profile_value WHERE person_id = $1",
            &[&person],
        )
        .expect("count private values")
        .get(0);
    assert_eq!(private_values, 1);

    let subject = format!("{}-user-inactive", db.prefix);
    assert_eq!(
        resolve_account(&mut db.identity, ISSUER, &subject),
        Err(AccountMapError::DeactivatedIdentity)
    );
}

#[test]
fn linked_profile_without_values_is_empty_not_an_error() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("empty");
    let person = db.person("empty");
    db.link(&account, person);

    let profile = db.read(&account).expect("linked profile");
    assert_eq!(profile.display_name, "Synthetic Person empty");
    assert!(
        profile.fields.is_empty(),
        "unexpected fields for {}: {:?}",
        profile.person_key,
        profile.fields
    );
}

#[test]
fn profile_route_and_operation_are_registered() {
    let source = include_str!("../src/api.rs");
    assert!(source.contains("/api/profile"));
    assert!(source.contains("Operation::ProfileRead"));
    assert!(source.contains("read_owner_profile"));
}
