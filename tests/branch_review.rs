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
        // Delete dependent rows first: a shared, long-lived test database may
        // currently lack the migration 0020 foreign keys (an earlier schema
        // reset can drop them), so cleanup must not depend on ON DELETE CASCADE.
        // Only rows this test created are matched, always by its unique prefix,
        // so real records are never touched. Proposal and audit events are
        // append-only by design and are never deleted; they hold no file content
        // and no foreign key.
        for statement in [
            // The appointment stores its person by key, not by id, so it has no
            // cascade and must be removed explicitly.
            "DELETE FROM head_of_family_appointment WHERE person_key LIKE $1",
            "DELETE FROM onboarding_person_account_link \
              WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE $1)",
            "DELETE FROM genealogy_fact_proposal \
              WHERE person_id IN (SELECT person_id FROM person WHERE person_key LIKE $1)",
        ] {
            let _ = self.genealogy.execute(statement, &[&pattern]);
        }
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

    let queue = list_proposals(
        &mut db.genealogy,
        &reviewer,
        Some(&branch),
        Some("pending"),
        None,
        25,
    )
    .expect("queue");
    // The alias "pending" is echoed back as the canonical status label.
    assert_eq!(queue.status, "pending_review");
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
fn evidence_must_belong_to_the_cited_document_and_pages() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-evidence");
    let (person_id, person_key) = db.person("evidence");
    let branch = db.branch("evidence");
    db.link(&owner, person_id);
    db.verified_branch(person_id, &branch);

    let evidence = db.evidence();
    let mismatched = ProposalEvidence {
        document_ref: format!("{}/not-the-cited-document.md", db.prefix),
        ..evidence.clone()
    };
    assert_eq!(
        propose_fact(
            &mut db.genealogy,
            &owner,
            &person_key,
            &branch,
            "gender",
            "gender",
            "female",
            &[mismatched],
            None,
        ),
        Err(BranchReviewError::InvalidEvidence)
    );

    let wrong_page = ProposalEvidence {
        page: evidence.page + 1,
        ..evidence.clone()
    };
    assert_eq!(
        propose_fact(
            &mut db.genealogy,
            &owner,
            &person_key,
            &branch,
            "gender",
            "gender",
            "female",
            &[wrong_page],
            None,
        ),
        Err(BranchReviewError::InvalidEvidence)
    );

    // The matching reference is accepted. The branch queue itself is reviewer
    // only, so an owner without a grant sees nothing there by design.
    let proposal_id = propose_fact(
        &mut db.genealogy,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
        &[evidence],
        None,
    )
    .expect("matching evidence");
    assert!(proposal_id > 0);
    assert!(list_proposals(&mut db.genealogy, &owner, Some(&branch), None, None, 25,).is_err());
}

#[test]
fn a_grant_without_an_audited_appointment_confers_no_authority() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-unaudited");
    let reviewer = db.account("reviewer-unaudited");
    let (person_id, person_key) = db.person("unaudited");
    let (reviewer_person, _) = db.person("reviewer-unaudited");
    let branch = db.branch("unaudited");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch);

    // A grant row written out of band carries a valid link and fingerprint but
    // no append-only appointment event, so it must confer no authority.
    db.genealogy
        .execute(
            "INSERT INTO onboarding_branch_reviewer_grant \
                 (account_id, family_branch_id, identity_fingerprint, granted_by, granted_reason) \
             SELECT $1, family_branch_id, $2, 'out-of-band', 'no audited appointment' \
               FROM family_branch WHERE branch_key = $3",
            &[
                &reviewer.account_id,
                &reviewer.identity_fingerprint,
                &branch,
            ],
        )
        .expect("out-of-band grant");

    assert!(effective_grants(&mut db.genealogy, &reviewer)
        .expect("grants")
        .is_empty());

    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "gender",
        "gender",
        "female",
    );
    assert_eq!(
        list_proposals(&mut db.genealogy, &reviewer, Some(&branch), None, None, 25),
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

// --- USR-005D acceptance scenarios -----------------------------------------
//
// These are the server-side scenarios from the branch review acceptance matrix
// that the USR-005B suite does not already cover. Scenarios 1-8, 10, and 12 are
// proven by the tests above and by tests/onboarding_branch_grants.rs; scenario 9
// and the role half of scenario 15 are proven at the HTTP boundary in
// web/e2e/branch-review-security.spec.ts, because deactivation and roles live in
// the identity store, not in the genealogy database.

/// Scenario 11: a Head of Family designation is genealogy status, not permission.
#[test]
fn head_of_family_without_a_grant_has_no_review_authority() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let hof = db.account("hof");
    let (hof_person, hof_key) = db.person("hof");
    let branch = db.branch("hof-third");
    db.link(&hof, hof_person);
    db.verified_branch(hof_person, &branch);
    db.genealogy
        .execute(
            "INSERT INTO head_of_family_appointment \
                 (person_key, branch_key, approved_by, evidence_assertion_ids) \
             VALUES ($1, $2, 'synthetic-admin', '[]'::jsonb)",
            &[&hof_key, &branch],
        )
        .expect("active appointment");

    let owner = db.account("owner-hof");
    let (subject, subject_key) = db.person("hof-subject");
    db.link(&owner, subject);
    db.verified_branch(subject, &branch);
    let proposal_id = proposal(
        &mut db,
        &owner,
        &subject_key,
        &branch,
        "gender",
        "gender",
        "female",
    );

    // The title confers nothing: no grant means no effective authority at all.
    assert!(effective_grants(&mut db.genealogy, &hof)
        .expect("grants")
        .is_empty());
    assert_eq!(
        list_proposals(&mut db.genealogy, &hof, Some(&branch), None, None, 25),
        Err(BranchReviewError::NotGranted)
    );
    assert_eq!(
        read_proposal(&mut db.genealogy, &hof, proposal_id),
        Err(BranchReviewError::UnknownProposal)
    );
    assert_eq!(
        decide_proposal(&mut db.genealogy, &hof, proposal_id, "approve", None),
        Err(BranchReviewError::NotGranted)
    );
}

/// Scenario 13: a disputed fact is only re-verified by an explicit decision.
#[test]
fn a_disputed_fact_is_not_verified_by_submission_alone() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-disputed");
    let reviewer = db.account("reviewer-disputed");
    let (person_id, person_key) = db.person("disputed");
    let (reviewer_person, _) = db.person("reviewer-disputed");
    let branch = db.branch("disputed");
    db.link(&owner, person_id);
    db.link(&reviewer, reviewer_person);
    db.verified_branch(person_id, &branch);
    db.assertion(person_id, "gender", "gender", "male", "disputed");
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

    // Submission records the disputed snapshot and changes nothing.
    let after_submission =
        read_proposal(&mut db.genealogy, &reviewer, proposal_id).expect("detail after submission");
    assert_eq!(
        after_submission.summary.prior_review_status.as_deref(),
        Some("disputed")
    );
    let (value, status): (String, String) = db
        .genealogy
        .query_one(
            "SELECT value_text, review_status FROM assertion \
              WHERE person_id = $1 AND assertion_kind = 'gender'",
            &[&person_id],
        )
        .map(|row| (row.get(0), row.get(1)))
        .expect("authoritative fact");
    assert_eq!((value.as_str(), status.as_str()), ("male", "disputed"));

    // Only an independent, audited approval promotes the disputed fact.
    let decision = decide_proposal(
        &mut db.genealogy,
        &reviewer,
        proposal_id,
        "approve",
        Some("independently re-verified against the cited page"),
    )
    .expect("approve disputed fact");
    assert_eq!(decision.status, "approved");
    let (value, status): (String, String) = db
        .genealogy
        .query_one(
            "SELECT value_text, review_status FROM assertion \
              WHERE person_id = $1 AND assertion_kind = 'gender'",
            &[&person_id],
        )
        .map(|row| (row.get(0), row.get(1)))
        .expect("authoritative fact after approval");
    assert_eq!((value.as_str(), status.as_str()), ("female", "verified"));
    let after = read_proposal(&mut db.genealogy, &reviewer, proposal_id).expect("detail");
    assert!(after.events.iter().any(|event| event.event == "approved"));
}

/// Scenario 14: review authority follows the person's branch, not the reviewer's
/// wish to see another branch. Cross-branch relationships stay out of scope.
#[test]
fn proposals_are_scoped_to_the_person_branch_not_the_reviewer() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-scope");
    let reviewer_one = db.account("reviewer-one");
    let reviewer_two = db.account("reviewer-two");
    let (person, person_key) = db.person("scope-one");
    let (one_person, _) = db.person("scope-reviewer-one");
    let (two_person, _) = db.person("scope-reviewer-two");
    let branch_one = db.branch("scope-one");
    let branch_two = db.branch("scope-two");
    db.link(&owner, person);
    db.link(&reviewer_one, one_person);
    db.link(&reviewer_two, two_person);
    db.verified_branch(person, &branch_one);
    let proposal_id = proposal(
        &mut db,
        &owner,
        &person_key,
        &branch_one,
        "gender",
        "gender",
        "female",
    );

    for (reviewer, branch) in [(&reviewer_one, &branch_one), (&reviewer_two, &branch_two)] {
        grant_branches(
            &mut db.genealogy,
            reviewer,
            std::slice::from_ref(branch),
            "admin",
            "branch reviewer",
        )
        .expect("grant");
    }

    // The branch-one reviewer can read the proposal; the branch-two reviewer
    // cannot see or decide it at all, and their own queue stays empty.
    assert!(read_proposal(&mut db.genealogy, &reviewer_one, proposal_id).is_ok());
    assert_eq!(
        read_proposal(&mut db.genealogy, &reviewer_two, proposal_id),
        Err(BranchReviewError::UnknownProposal)
    );
    assert_eq!(
        decide_proposal(
            &mut db.genealogy,
            &reviewer_two,
            proposal_id,
            "approve",
            None
        ),
        Err(BranchReviewError::NotGranted)
    );
    assert!(list_proposals(
        &mut db.genealogy,
        &reviewer_two,
        Some(&branch_two),
        None,
        None,
        25
    )
    .expect("branch two queue")
    .submissions
    .is_empty());
}

/// Scenario 16 (API): branch review never returns member-private profile data.
#[test]
fn branch_review_reads_never_return_private_profile_values() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let owner = db.account("owner-private");
    let reviewer = db.account("reviewer-private");
    let (person_id, person_key) = db.person("private");
    let (reviewer_person, _) = db.person("reviewer-private");
    let branch = db.branch("private");
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

    // Private profile values exist for the same person and must never surface.
    // The value guard only accepts an approved revision, so the fixture walks
    // the real draft -> submitted -> approved path before writing each value.
    let marker = format!("{}-private-marker", db.prefix);
    for (field, value) in [
        ("current_location", format!("{marker}-home")),
        ("contact_email", format!("{marker}-email")),
        ("phone", format!("{marker}-phone")),
    ] {
        let revision_id: i64 = db
            .genealogy
            .query_one(
                "INSERT INTO onboarding_profile_revision \
                     (person_id, field_key, prior_value_text, \
                      prior_approved_revision_number, new_value_text, changed_by, \
                      revision_number, status, visibility) \
                 VALUES ($1, $2, NULL, 0, $3, 'synthetic-owner', 1, 'draft', \
                         'owner_private') \
                 RETURNING revision_id",
                &[&person_id, &field, &value],
            )
            .expect("profile revision")
            .get(0);
        db.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'pending_review', submitted_at = now() \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("submit profile revision");
        db.genealogy
            .execute(
                "UPDATE onboarding_profile_revision \
                    SET status = 'approved', reviewed_at = now(), \
                        reviewed_by = 'synthetic-reviewer', review_note = 'synthetic' \
                  WHERE revision_id = $1",
                &[&revision_id],
            )
            .expect("approve profile revision");
        db.genealogy
            .execute(
                "INSERT INTO onboarding_profile_value \
                     (person_id, field_key, value_text, visibility, updated_by, \
                      approved_revision_number, approved_revision_id) \
                 VALUES ($1, $2, $3, 'owner_private', 'synthetic-owner', 1, $4)",
                &[&person_id, &field, &value, &revision_id],
            )
            .expect("profile value");
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
    let queue =
        list_proposals(&mut db.genealogy, &reviewer, Some(&branch), None, None, 25).expect("queue");
    let detail = read_proposal(&mut db.genealogy, &reviewer, proposal_id).expect("detail");

    // Debug rendering is the strictest structural check available here: the
    // response types have no private fields to select at all. The HTTP-level
    // form of the same guarantee, against real JSON, is asserted in
    // web/e2e/branch-review-security.spec.ts with a seeded private value.
    for rendered in [format!("{queue:?}"), format!("{detail:?}")] {
        assert!(
            !rendered.contains(&marker),
            "branch review leaked a private profile value: {rendered}"
        );
    }
}

/// Scenario 16 (RAG): proposed shared facts never enter the retrieval corpus.
#[test]
fn proposal_content_never_enters_the_rag_corpus() {
    let Some(mut db) = TestDb::new() else {
        return;
    };
    let rag_dsn = match std::env::var("DATABASE_URL") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => {
            eprintln!("DATABASE_URL unset; skipping the RAG corpus assertion");
            return;
        }
    };
    let owner = db.account("owner-rag");
    let (person_id, person_key) = db.person("rag");
    let branch = db.branch("rag");
    db.link(&owner, person_id);
    db.verified_branch(person_id, &branch);

    let marker = format!("{}-rag-marker", db.prefix);
    proposal(
        &mut db,
        &owner,
        &person_key,
        &branch,
        "name",
        "name",
        &marker,
    );

    let mut rag = Client::connect(&rag_dsn, NoTls).expect("rag database");
    let has_documents: bool = rag
        .query_one("SELECT to_regclass('public.documents') IS NOT NULL", &[])
        .expect("documents table probe")
        .get(0);
    if !has_documents {
        eprintln!("documents table absent; skipping the RAG corpus assertion");
        return;
    }
    let count: i64 = rag
        .query_one(
            "SELECT count(*) FROM documents WHERE content LIKE $1",
            &[&format!("%{marker}%")],
        )
        .expect("corpus probe")
        .get(0);
    assert_eq!(
        count, 0,
        "a proposed shared fact must never be readable from the RAG corpus"
    );
}
