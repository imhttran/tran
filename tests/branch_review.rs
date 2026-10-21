//! USR-005B synthetic tests for branch-scoped shared-fact proposals.

use std::fs;
use std::str::FromStr;
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use postgres::{Client, NoTls};
use rag_foundation::genealogy::branch_review::{
    decide_proposal, effective_grants, list_proposals, propose_fact, read_proposal,
    BranchReviewError, ProposalEvidence,
};
use rag_foundation::onboarding::{grant_branches, IdentityAccount};

const MIGRATIONS: [&str; 20] = [
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
];

static DB_LOCK: Mutex<()> = Mutex::new(());

fn dsn() -> Option<String> {
    let dsn = std::env::var("GENEALOGY_DATABASE_URL").ok()?;
    let dsn = dsn.trim().to_string();
    if dsn.is_empty() {
        eprintln!("GENEALOGY_DATABASE_URL unset; skipping branch review test");
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
    evidence_seq: u64,
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
            prefix: format!("synthetic-branch-review-{}-{nanos}", std::process::id()),
            evidence_seq: 0,
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
                &[&key, &format!("Synthetic Branch Person {label}")],
            )
            .expect("person")
            .get(0);
        (id, key)
    }

    fn branch(&mut self, label: &str) -> String {
        let key = format!("{}-branch-{label}", self.prefix);
        self.genealogy
            .execute(
                "INSERT INTO family_branch (branch_key, branch_label, branch_number)
                 VALUES ($1, $2, 1)",
                &[&key, &format!("Synthetic Branch {label}")],
            )
            .expect("branch");
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
            .expect("link");
    }

    fn verified_branch(&mut self, person_id: i64, branch_key: &str) {
        self.assertion(
            person_id,
            "branch_claim",
            branch_key,
            branch_key,
            "verified",
        );
    }

    fn assertion(&mut self, person_id: i64, kind: &str, key: &str, value: &str, status: &str) {
        let evidence = self.evidence();
        let assertion_id: i64 = self
            .genealogy
            .query_one(
                "INSERT INTO assertion
                     (person_id, assertion_kind, assertion_key, value_text, review_status)
                 VALUES ($1, $2, $3, $4, 'unverified') RETURNING assertion_id",
                &[&person_id, &kind, &key, &value],
            )
            .expect("assertion")
            .get(0);
        self.genealogy
            .execute(
                "INSERT INTO assertion_evidence
                     (assertion_id, source_evidence_id, document_ref, page)
                 VALUES ($1, $2, $3, $4)",
                &[
                    &assertion_id,
                    &evidence.source_evidence_id,
                    &evidence.document_ref,
                    &evidence.page,
                ],
            )
            .expect("assertion evidence");
        if status != "unverified" {
            self.genealogy
                .execute(
                    "UPDATE assertion
                        SET review_status = $2, reviewed_by = 'synthetic-reviewer', reviewed_at = now()
                      WHERE assertion_id = $1",
                    &[&assertion_id, &status],
                )
                .expect("assertion status");
        }
    }

    fn evidence(&mut self) -> ProposalEvidence {
        self.evidence_seq += 1;
        let path = format!("{}/source-{}.md", self.prefix, self.evidence_seq);
        let document_id: i64 = self
            .genealogy
            .query_one(
                "INSERT INTO source_document (relative_path, sha256, source_kind)
                 VALUES ($1, repeat('a', 64), 'original') RETURNING source_document_id",
                &[&path],
            )
            .expect("source document")
            .get(0);
        let evidence_id: i64 = self
            .genealogy
            .query_one(
                "INSERT INTO source_evidence (source_document_id, page, locator)
                 VALUES ($1, 1, 'p.1') RETURNING source_evidence_id",
                &[&document_id],
            )
            .expect("source evidence")
            .get(0);
        ProposalEvidence {
            source_evidence_id: evidence_id,
            document_ref: path,
            page: 1,
        }
    }
}

impl Drop for TestDb {
    fn drop(&mut self) {
        let pattern = format!("{}%", self.prefix);
        let _ = self
            .genealogy
            .execute("DELETE FROM person WHERE person_key LIKE $1", &[&pattern]);
        let _ = self.genealogy.execute(
            "DELETE FROM source_document WHERE relative_path LIKE $1",
            &[&pattern],
        );
        let _ = self.genealogy.execute(
            "DELETE FROM family_branch WHERE branch_key LIKE $1",
            &[&pattern],
        );
    }
}

fn proposal(
    db: &mut TestDb,
    account: &IdentityAccount,
    person_key: &str,
    branch: &str,
    kind: &str,
    key: &str,
    value: &str,
) -> i64 {
    let evidence = db.evidence();
    propose_fact(
        &mut db.genealogy,
        account,
        person_key,
        branch,
        kind,
        key,
        value,
        std::slice::from_ref(&evidence),
        Some("synthetic proposal"),
    )
    .expect("proposal")
}

#[test]
fn branch_reviewer_approves_an_in_scope_source_backed_proposal() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner");
    let reviewer = db.account("reviewer");
    let (person_id, person_key) = db.person("subject");
    let (reviewer_person, _) = db.person("reviewer");
    let branch = db.branch("one");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch);
    grant_branches(
        &mut db.genealogy,
        &reviewer,
        std::slice::from_ref(&branch),
        "admin",
        "branch reviewer",
    )
    .expect("grant reviewer");

    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );
    let grants = effective_grants(&mut db.genealogy, &reviewer).expect("grants");
    assert_eq!(grants[0].branch_key, branch);

    let queue =
        list_proposals(&mut db.genealogy, &reviewer, Some(&branch), None, None, 25).expect("queue");
    assert_eq!(queue.submissions.len(), 1);
    assert!(queue.submissions[0].can_decide);
    let detail = read_proposal(&mut db.genealogy, &reviewer, proposal_id).expect("detail");
    assert!(detail.current_assertion.is_none());
    assert_eq!(detail.summary.proposed_value_text, "female");

    // The submitter may hold a grant, but independence still blocks self-review.
    grant_branches(
        &mut db.genealogy,
        &owner,
        std::slice::from_ref(&branch),
        "admin",
        "owner reviewer",
    )
    .expect("grant owner");
    assert_eq!(
        decide_proposal(
            &mut db.genealogy,
            &owner,
            proposal_id,
            "approve",
            Some("self"),
        ),
        Err(BranchReviewError::SelfReview)
    );

    let decision = decide_proposal(
        &mut db.genealogy,
        &reviewer,
        proposal_id,
        "approve",
        Some("source supports this"),
    )
    .expect("approve");
    assert_eq!(decision.status, "approved");
    let assertion_id = decision.applied_assertion_id.expect("assertion id");
    let row = db
        .genealogy
        .query_one(
            "SELECT value_text, review_status FROM assertion WHERE assertion_id = $1",
            &[&assertion_id],
        )
        .expect("assertion row");
    assert_eq!(row.get::<_, String>(0), "female");
    assert_eq!(row.get::<_, String>(1), "verified");
    let after = read_proposal(&mut db.genealogy, &reviewer, proposal_id).expect("detail after");
    assert_eq!(after.summary.status, "approved");
    assert_eq!(
        after
            .current_assertion
            .as_ref()
            .map(|item| item.value_text.as_deref()),
        Some(Some("female"))
    );
    assert!(after.events.iter().any(|event| event.event == "approved"));
}

#[test]
fn rejection_and_needs_changes_leave_the_verified_fact_unchanged() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-reject");
    let reviewer = db.account("reviewer-reject");
    let (person_id, person_key) = db.person("reject");
    let (reviewer_person, _) = db.person("reviewer-reject");
    let branch = db.branch("reject");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch);
    db.assertion(person_id, "gender", "gender", "male", "verified");
    grant_branches(
        &mut db.genealogy,
        &reviewer,
        std::slice::from_ref(&branch),
        "admin",
        "branch reviewer",
    )
    .expect("grant");

    let first = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );
    assert_eq!(
        decide_proposal(&mut db.genealogy, &reviewer, first, "reject", None),
        Err(BranchReviewError::ReviewNoteRequired)
    );
    decide_proposal(
        &mut db.genealogy,
        &reviewer,
        first,
        "reject",
        Some("insufficient evidence"),
    )
    .expect("reject");
    let value: String = db
        .genealogy
        .query_one(
            "SELECT value_text FROM assertion WHERE person_id = $1 AND assertion_kind = 'gender'",
            &[&person_id],
        )
        .expect("assertion")
        .get(0);
    assert_eq!(value, "male");

    let second = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );
    decide_proposal(
        &mut db.genealogy,
        &reviewer,
        second,
        "needs_changes",
        Some("add a second source"),
    )
    .expect("needs changes");
    let status: String = db
        .genealogy
        .query_one(
            "SELECT review_status FROM assertion WHERE person_id = $1 AND assertion_kind = 'gender'",
            &[&person_id],
        )
        .expect("assertion")
        .get(0);
    assert_eq!(status, "verified");
}

#[test]
fn cross_branch_and_revoked_grants_are_denied() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-cross");
    let reviewer = db.account("reviewer-cross");
    let (person_id, person_key) = db.person("cross");
    let (reviewer_person, _) = db.person("reviewer-cross");
    let branch_one = db.branch("cross-one");
    let branch_two = db.branch("cross-two");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch_one);

    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch_one,
        "gender",
        "gender",
        "female",
    );
    assert_eq!(
        list_proposals(
            &mut db.genealogy,
            &reviewer,
            Some(&branch_one),
            None,
            None,
            25
        )
        .map(|page| page.submissions.len()),
        Err(BranchReviewError::NotGranted)
    );
    assert_eq!(
        read_proposal(&mut db.genealogy, &reviewer, proposal_id),
        Err(BranchReviewError::UnknownProposal)
    );
    assert_eq!(
        decide_proposal(&mut db.genealogy, &reviewer, proposal_id, "approve", None),
        Err(BranchReviewError::NotGranted)
    );

    grant_branches(
        &mut db.genealogy,
        &reviewer,
        std::slice::from_ref(&branch_one),
        "admin",
        "branch one",
    )
    .expect("grant branch one");
    assert_eq!(
        list_proposals(
            &mut db.genealogy,
            &reviewer,
            Some(&branch_two),
            None,
            None,
            25
        ),
        Err(BranchReviewError::NotGranted)
    );
    let page = list_proposals(&mut db.genealogy, &reviewer, None, None, None, 25).expect("queue");
    assert_eq!(page.submissions.len(), 1);

    let grant_id = grant_id(&mut db, &reviewer).expect("grant id");
    rag_foundation::onboarding::revoke_grant(&mut db.genealogy, grant_id, "admin", "term ended")
        .expect("revoke");
    let page = list_proposals(&mut db.genealogy, &reviewer, None, None, None, 25)
        .expect("queue after revoke");
    assert!(page.submissions.is_empty());
}

#[test]
fn ambiguous_branch_membership_blocks_proposals() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-ambiguous");
    let (person_id, person_key) = db.person("ambiguous");
    let branch_one = db.branch("ambiguous-one");
    let branch_two = db.branch("ambiguous-two");
    db.link(&owner, person_id);
    db.verified_branch(person_id, &branch_one);
    db.verified_branch(person_id, &branch_two);
    let evidence = db.evidence();
    assert_eq!(
        propose_fact(
            &mut db.genealogy,
            &owner,
            &person_key,
            &branch_one,
            "gender",
            "gender",
            "female",
            &[evidence],
            None,
        ),
        Err(BranchReviewError::BranchAmbiguous)
    );
}

#[test]
fn stale_prior_snapshot_blocks_approval() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-stale");
    let reviewer = db.account("reviewer-stale");
    let (person_id, person_key) = db.person("stale");
    let (reviewer_person, _) = db.person("reviewer-stale");
    let branch = db.branch("stale");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch);
    db.assertion(person_id, "gender", "gender", "male", "verified");
    grant_branches(
        &mut db.genealogy,
        &reviewer,
        std::slice::from_ref(&branch),
        "admin",
        "branch reviewer",
    )
    .expect("grant");

    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );
    db.genealogy
        .execute(
            "UPDATE assertion SET value_text = 'unknown' WHERE person_id = $1 AND assertion_kind = 'gender'",
            &[&person_id],
        )
        .expect("change current fact");
    assert_eq!(
        decide_proposal(
            &mut db.genealogy,
            &reviewer,
            proposal_id,
            "approve",
            Some("stale"),
        ),
        Err(BranchReviewError::Stale)
    );
}

#[test]
fn concurrent_decisions_have_exactly_one_winner() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-concurrent-review");
    let reviewer_one = db.account("reviewer-concurrent-one");
    let reviewer_two = db.account("reviewer-concurrent-two");
    let (person_id, person_key) = db.person("concurrent-review");
    let (reviewer_person_one, _) = db.person("reviewer-concurrent-one");
    let (reviewer_person_two, _) = db.person("reviewer-concurrent-two");
    let branch = db.branch("concurrent-review");
    db.link(&owner, person_id);
    db.link(&reviewer_one, reviewer_person_one);
    db.link(&reviewer_two, reviewer_person_two);
    db.verified_branch(person_id, &branch);
    for reviewer in [&reviewer_one, &reviewer_two] {
        grant_branches(
            &mut db.genealogy,
            reviewer,
            std::slice::from_ref(&branch),
            "admin",
            "branch reviewer",
        )
        .expect("grant");
    }
    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );

    let barrier = Arc::new(Barrier::new(2));
    let dsn_one = db.dsn.clone();
    let dsn_two = db.dsn.clone();
    let fingerprint_one = reviewer_one.identity_fingerprint.clone();
    let fingerprint_two = reviewer_two.identity_fingerprint.clone();
    let one = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut client = Client::connect(&dsn_one, NoTls).expect("reviewer one db");
            barrier.wait();
            let account = IdentityAccount {
                account_id: reviewer_one.account_id,
                identity_fingerprint: fingerprint_one,
            };
            decide_proposal(&mut client, &account, proposal_id, "approve", Some("one"))
        })
    };
    let two = {
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            let mut client = Client::connect(&dsn_two, NoTls).expect("reviewer two db");
            barrier.wait();
            let account = IdentityAccount {
                account_id: reviewer_two.account_id,
                identity_fingerprint: fingerprint_two,
            };
            decide_proposal(&mut client, &account, proposal_id, "approve", Some("two"))
        })
    };
    let outcomes = [one.join().expect("one"), two.join().expect("two")];
    assert_eq!(outcomes.iter().filter(|outcome| outcome.is_ok()).count(), 1);
    assert!(outcomes
        .iter()
        .any(|outcome| matches!(outcome, Err(BranchReviewError::InvalidTransition))));
}

fn grant_id(db: &mut TestDb, account: &IdentityAccount) -> Option<i64> {
    db.genealogy
        .query_opt(
            "SELECT grant_id FROM onboarding_branch_reviewer_grant WHERE account_id = $1",
            &[&account.account_id],
        )
        .ok()
        .flatten()
        .map(|row| row.get(0))
}

#[test]
fn linked_owner_and_replaced_account_cannot_use_a_grant() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-link-bound");
    let reviewer = db.account("reviewer-link-bound");
    let replacement = db.account("reviewer-replacement");
    let (person_id, person_key) = db.person("link-bound");
    let (reviewer_person, _) = db.person("reviewer-link-bound");
    let branch = db.branch("link-bound");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch);
    grant_branches(
        &mut db.genealogy,
        &reviewer,
        std::slice::from_ref(&branch),
        "admin",
        "branch reviewer",
    )
    .expect("grant");
    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );

    db.genealogy
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE person_id = $1",
            &[&reviewer_person],
        )
        .expect("remove reviewer link");
    db.genealogy
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person_id],
        )
        .expect("remove owner link");
    db.link(&reviewer, person_id);
    assert_eq!(
        decide_proposal(
            &mut db.genealogy,
            &reviewer,
            proposal_id,
            "approve",
            Some("owner"),
        ),
        Err(BranchReviewError::OwnerReview)
    );

    db.genealogy
        .execute(
            "DELETE FROM onboarding_person_account_link WHERE person_id = $1",
            &[&person_id],
        )
        .expect("remove reviewer-owner link");
    db.link(&replacement, person_id);
    assert!(effective_grants(&mut db.genealogy, &reviewer)
        .expect("old grants")
        .is_empty());
    assert!(
        list_proposals(&mut db.genealogy, &replacement, None, None, None, 25)
            .expect("replacement queue")
            .submissions
            .is_empty()
    );
}
