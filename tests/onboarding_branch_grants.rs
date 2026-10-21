//! USR-005A synthetic tests for explicit branch reviewer grants and audit.

use std::fs;
use std::str::FromStr;
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::onboarding::{
    active_branch_grants, grant_branches, has_active_branch_grant, list_branches,
    list_grant_events, list_grants, revoke_grant, BranchGrantError, IdentityAccount,
};

const MIGRATIONS: [&str; 19] = [
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
];

static DB_LOCK: Mutex<()> = Mutex::new(());

fn dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        eprintln!("GENEALOGY_DATABASE_URL unset; skipping branch grant test");
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
        let test = Self {
            genealogy,
            dsn,
            prefix: format!("synthetic-grant-{}-{nanos}", std::process::id()),
            _guard: guard,
        };
        Some(test)
    }

    fn account(&mut self, label: &str) -> IdentityAccount {
        let id: i64 = self
            .genealogy
            .query_one(
                "SELECT (EXTRACT(EPOCH FROM now()) * 1000000)::bigint + $1",
                &[&(label.len() as i64)],
            )
            .expect("account id")
            .get(0);
        IdentityAccount {
            account_id: id,
            identity_fingerprint: format!("{id:064x}"),
        }
    }

    fn person(&mut self, label: &str) -> i64 {
        let key = format!("{}-person-{label}", self.prefix);
        self.genealogy
            .query_one(
                "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
                &[&key, &format!("Synthetic Grant Person {label}")],
            )
            .expect("insert person")
            .get(0)
    }

    fn branch(&mut self, label: &str) -> String {
        let key = format!("{}-branch-{label}", self.prefix);
        self.genealogy
            .execute(
                "INSERT INTO family_branch (branch_key, branch_label, branch_number)
                 VALUES ($1, $2, 1)",
                &[&key, &format!("Synthetic Branch {label}")],
            )
            .expect("insert branch");
        key
    }

    fn link(&mut self, account: &IdentityAccount, person_id: i64) {
        self.genealogy
            .execute(
                "INSERT INTO onboarding_person_account_link
                     (person_id, account_id, linked_by, identity_fingerprint)
                 VALUES ($1, $2, 'synthetic-reviewer', $3)",
                &[
                    &person_id,
                    &account.account_id,
                    &account.identity_fingerprint,
                ],
            )
            .expect("insert link");
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pattern = format!("{}%", self.prefix);
        // The event ledger is append-only by design. Synthetic events are left
        // in this disposable test database; assertions are scoped by account id.
        let _ = self.genealogy.execute(
            "DELETE FROM onboarding_branch_reviewer_grant
              WHERE family_branch_id IN (
                  SELECT family_branch_id FROM family_branch WHERE branch_key LIKE $1
              )",
            &[&pattern],
        );
        let _ = self
            .genealogy
            .execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
        let _ = self.genealogy.execute(
            "DELETE FROM family_branch WHERE branch_key LIKE $1",
            &[&pattern],
        );
    }
}

#[test]
fn grants_are_explicit_scoped_and_audited() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("multi");
    let person = db.person("multi");
    db.link(&account, person);
    let branch_a = db.branch("a");
    let branch_b = db.branch("b");
    let branch_c = db.branch("c");

    let branches = list_branches(&mut db.genealogy).expect("list branches");
    assert!(branches.iter().any(|branch| branch.branch_key == branch_a));
    assert!(branches.iter().any(|branch| branch.branch_key == branch_b));

    let granted = grant_branches(
        &mut db.genealogy,
        &account,
        &[branch_a.clone(), branch_b.clone()],
        "admin-fingerprint",
        "trusted branch reviewer",
    )
    .expect("grant two branches");
    assert_eq!(granted.len(), 2);
    assert!(granted.iter().all(|grant| grant.active));
    assert!(granted
        .iter()
        .all(|grant| grant.identity_fingerprint.as_deref() == Some(&account.identity_fingerprint)));

    let active = active_branch_grants(&mut db.genealogy, &account).expect("active grants");
    assert_eq!(
        active
            .iter()
            .map(|grant| grant.branch_key.as_str())
            .collect::<Vec<_>>(),
        vec![branch_a.as_str(), branch_b.as_str()]
    );
    assert!(has_active_branch_grant(&mut db.genealogy, &account, &branch_a).expect("branch a"));
    assert!(!has_active_branch_grant(&mut db.genealogy, &account, &branch_c).expect("branch c"));

    let events = list_grant_events(&mut db.genealogy, Some(account.account_id)).expect("events");
    assert_eq!(events.len(), 2);
    assert!(events.iter().all(|event| event.event == "granted"));
    assert!(events
        .iter()
        .all(|event| event.reason.as_deref() == Some("trusted branch reviewer")));

    assert_eq!(
        grant_branches(
            &mut db.genealogy,
            &account,
            std::slice::from_ref(&branch_a),
            "admin-fingerprint",
            "duplicate",
        ),
        Err(BranchGrantError::AlreadyActive(branch_a.clone()))
    );

    let first = active
        .iter()
        .find(|grant| grant.branch_key == branch_a)
        .expect("branch a grant")
        .grant_id;
    let revoked =
        revoke_grant(&mut db.genealogy, first, "admin-fingerprint", "role ended").expect("revoke");
    assert!(!revoked.active);
    assert_eq!(revoked.revoked_by.as_deref(), Some("admin-fingerprint"));
    assert_eq!(revoked.revoke_reason.as_deref(), Some("role ended"));
    assert_eq!(
        revoke_grant(&mut db.genealogy, first, "admin-fingerprint", "again"),
        Err(BranchGrantError::NotActive(branch_a.clone()))
    );
    let active = active_branch_grants(&mut db.genealogy, &account).expect("active after revoke");
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].branch_key, branch_b);

    let regranted = grant_branches(
        &mut db.genealogy,
        &account,
        std::slice::from_ref(&branch_a),
        "second-admin",
        "reappointed",
    )
    .expect("regrant");
    assert_ne!(regranted[0].grant_id, first);
    assert!(regranted[0].active);
    assert!(regranted[0].usable);
    let events = list_grant_events(&mut db.genealogy, Some(account.account_id)).expect("events");
    assert_eq!(events.len(), 4);
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event == "revoked")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event == "granted")
            .count(),
        3
    );

    let history = list_grants(&mut db.genealogy, Some(account.account_id)).expect("history");
    assert_eq!(history.len(), 3);
    assert_eq!(history.iter().filter(|grant| grant.active).count(), 2);
}

#[test]
fn grant_requires_a_reviewed_link_and_a_known_branch() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("unlinked");
    let person = db.person("unlinked");
    let branch = db.branch("linked");
    assert_eq!(
        grant_branches(
            &mut db.genealogy,
            &account,
            std::slice::from_ref(&branch),
            "admin",
            "reason",
        ),
        Err(BranchGrantError::Unlinked)
    );

    db.link(&account, person);
    assert_eq!(
        grant_branches(
            &mut db.genealogy,
            &account,
            &["missing-branch".to_string()],
            "admin",
            "reason",
        ),
        Err(BranchGrantError::UnknownBranch(
            "missing-branch".to_string()
        ))
    );
    assert_eq!(
        grant_branches(&mut db.genealogy, &account, &[], "admin", "reason"),
        Err(BranchGrantError::EmptySelection)
    );
    assert_eq!(
        grant_branches(
            &mut db.genealogy,
            &account,
            std::slice::from_ref(&branch),
            "admin",
            "   ",
        ),
        Err(BranchGrantError::InvalidReason)
    );

    let wrong = IdentityAccount {
        account_id: account.account_id,
        identity_fingerprint: "f".repeat(64),
    };
    assert_eq!(
        grant_branches(&mut db.genealogy, &wrong, &[branch], "admin", "reason"),
        Err(BranchGrantError::Unlinked)
    );
}

#[test]
fn account_replacement_does_not_inherit_grants() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let first = db.account("replacement-first");
    let second = db.account("replacement-second");
    let person = db.person("replacement");
    let branch = db.branch("replacement");
    db.link(&first, person);
    grant_branches(
        &mut db.genealogy,
        &first,
        std::slice::from_ref(&branch),
        "admin",
        "first account",
    )
    .expect("grant first");

    db.genealogy
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person],
        )
        .expect("revoke first link");
    assert!(active_branch_grants(&mut db.genealogy, &first)
        .expect("first active")
        .is_empty());
    assert!(!has_active_branch_grant(&mut db.genealogy, &first, &branch).expect("first check"));

    db.link(&second, person);
    assert!(active_branch_grants(&mut db.genealogy, &second)
        .expect("second active")
        .is_empty());
    let second_grant = grant_branches(
        &mut db.genealogy,
        &second,
        std::slice::from_ref(&branch),
        "admin",
        "second account",
    )
    .expect("grant second");
    assert_eq!(second_grant.len(), 1);
    assert_eq!(
        second_grant[0].identity_fingerprint.as_deref(),
        Some(second.identity_fingerprint.as_str())
    );
}

#[test]
fn legacy_grant_without_fingerprint_is_unusable() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("legacy");
    let person = db.person("legacy");
    let branch = db.branch("legacy");
    db.link(&account, person);
    let branch_id: i64 = db
        .genealogy
        .query_one(
            "SELECT family_branch_id FROM family_branch WHERE branch_key = $1",
            &[&branch],
        )
        .expect("branch id")
        .get(0);
    db.genealogy
        .execute(
            "INSERT INTO onboarding_branch_reviewer_grant
                 (account_id, family_branch_id, granted_by)
             VALUES ($1, $2, 'legacy-admin')",
            &[&account.account_id, &branch_id],
        )
        .expect("legacy grant");
    assert!(active_branch_grants(&mut db.genealogy, &account)
        .expect("legacy active")
        .is_empty());

    let rebound = grant_branches(
        &mut db.genealogy,
        &account,
        std::slice::from_ref(&branch),
        "admin",
        "rebind identity",
    )
    .expect("rebind");
    assert!(rebound[0].active);
    assert_eq!(
        rebound[0].identity_fingerprint.as_deref(),
        Some(account.identity_fingerprint.as_str())
    );
}

#[test]
fn concurrent_grants_have_exactly_one_winner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("concurrent");
    let person = db.person("concurrent");
    let branch = db.branch("concurrent");
    db.link(&account, person);

    let barrier = Arc::new(Barrier::new(2));
    let dsn_one = db.dsn.clone();
    let dsn_two = db.dsn.clone();
    let account_one = account.clone();
    let account_two = account.clone();
    let branch_one = branch.clone();
    let branch_two = branch.clone();
    let one = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut client = Client::connect(&dsn_one, NoTls).expect("grant one db");
            barrier.wait();
            grant_branches(
                &mut client,
                &account_one,
                &[branch_one],
                "admin-one",
                "concurrent one",
            )
        })
    };
    let two = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut client = Client::connect(&dsn_two, NoTls).expect("grant two db");
            barrier.wait();
            grant_branches(
                &mut client,
                &account_two,
                &[branch_two],
                "admin-two",
                "concurrent two",
            )
        })
    };
    let outcomes = [
        one.join().expect("one joined"),
        two.join().expect("two joined"),
    ];
    assert_eq!(
        outcomes.iter().filter(|result| result.is_ok()).count(),
        1,
        "exactly one concurrent grant may commit"
    );
    assert!(outcomes
        .iter()
        .any(|result| matches!(result, Err(BranchGrantError::AlreadyActive(_)))));
    let grants: i64 = db
        .genealogy
        .query_one(
            "SELECT count(*) FROM onboarding_branch_reviewer_grant
              WHERE account_id = $1 AND revoked_at IS NULL",
            &[&account.account_id],
        )
        .expect("grant count")
        .get(0);
    assert_eq!(grants, 1);
    let events = list_grant_events(&mut db.genealogy, Some(account.account_id)).expect("events");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event == "granted")
            .count(),
        1
    );
}

#[test]
fn grant_events_are_append_only() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let account = db.account("append-only");
    let person = db.person("append-only");
    let branch = db.branch("append-only");
    db.link(&account, person);
    grant_branches(
        &mut db.genealogy,
        &account,
        &[branch],
        "admin",
        "append-only test",
    )
    .expect("grant");
    let event_id: i64 = db
        .genealogy
        .query_one(
            "SELECT event_id FROM onboarding_branch_reviewer_grant_event
              WHERE account_id = $1 ORDER BY event_id LIMIT 1",
            &[&account.account_id],
        )
        .expect("event id")
        .get(0);
    let update = db.genealogy.execute(
        "UPDATE onboarding_branch_reviewer_grant_event SET reason = 'tampered'
          WHERE event_id = $1",
        &[&event_id],
    );
    assert!(update.is_err(), "grant events must be append-only");
    let delete = db.genealogy.execute(
        "DELETE FROM onboarding_branch_reviewer_grant_event WHERE event_id = $1",
        &[&event_id],
    );
    assert!(delete.is_err(), "grant events must not be deletable");
}
