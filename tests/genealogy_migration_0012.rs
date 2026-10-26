//! Migration 0012 tests: additive upgrade, backfill, constraints, and guards.
//!
//! Uses a dedicated disposable target derived from `GENEALOGY_DATABASE_URL`
//! through `tests/support/genealogy_isolation.rs`. No configured genealogy or
//! RAG database is modified; each test drops only the target it created.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::fs;

use genealogy_isolation::{base_dsn, create_empty_isolated_target, IsolatedTarget};
use postgres::Client;

const PRE_0012: [&str; 11] = [
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
];

fn apply(client: &mut Client, paths: &[&str]) {
    for path in paths {
        let sql = fs::read_to_string(path).unwrap_or_else(|err| panic!("read {path}: {err}"));
        client
            .batch_execute(&sql)
            .unwrap_or_else(|err| panic!("apply {path}: {err}"));
    }
}

fn prepare(label: &str) -> Option<IsolatedTarget> {
    let base = base_dsn()?;
    Some(create_empty_isolated_target(&base, label))
}

fn seed_person(client: &mut Client, key: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2) RETURNING person_id",
            &[&key, &"Synthetic migration person"],
        )
        .expect("seed person")
        .get(0)
}

#[test]
fn legacy_profile_revisions_are_preserved_and_numbered() {
    let Some(target) = prepare("migration_0012_legacy") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping migration 0012 test");
        return;
    };
    let mut client = target.connect();
    apply(&mut client, &PRE_0012);

    let person_id = seed_person(&mut client, "migration-0012-legacy");
    client
        .execute(
            "INSERT INTO onboarding_profile_revision \
                 (person_id, field_key, prior_value_text, new_value_text, changed_by) \
             VALUES ($1, 'biography', NULL, 'legacy biography', 'legacy-owner')",
            &[&person_id],
        )
        .expect("insert legacy revision");
    client
        .execute(
            "INSERT INTO onboarding_profile_value \
                 (person_id, field_key, value_text, updated_by) \
             VALUES ($1, 'display_name', 'Legacy Name', 'legacy-owner')",
            &[&person_id],
        )
        .expect("insert legacy approved value");

    apply(
        &mut client,
        &["genealogy/migrations/0012_profile_revisions.sql"],
    );

    let row = client
        .query_one(
            "SELECT revision_number, status, visibility, new_value_text, changed_by, \
                    submitted_at IS NOT NULL, reviewed_at IS NOT NULL, reviewed_by, review_note \
               FROM onboarding_profile_revision WHERE person_id = $1",
            &[&person_id],
        )
        .expect("read backfilled revision");
    assert_eq!(row.get::<_, i64>(0), 1);
    assert_eq!(row.get::<_, String>(1), "approved");
    assert_eq!(row.get::<_, String>(2), "owner_private");
    assert_eq!(row.get::<_, String>(3), "legacy biography");
    assert_eq!(row.get::<_, String>(4), "legacy-owner");
    assert!(
        !row.get::<_, bool>(5),
        "legacy rows must not fabricate a submission timestamp"
    );
    assert!(
        !row.get::<_, bool>(6),
        "legacy rows must not fabricate a review timestamp"
    );
    assert_eq!(
        row.get::<_, Option<String>>(7),
        None,
        "legacy rows must not fabricate a reviewer"
    );
    assert!(row
        .get::<_, String>(8)
        .contains("review provenance unavailable"));

    // Existing approved values gain the owner-private default without changing
    // their text.
    let value = client
        .query_one(
            "SELECT value_text, visibility FROM onboarding_profile_value WHERE person_id = $1",
            &[&person_id],
        )
        .expect("read legacy value");
    assert_eq!(value.get::<_, String>(0), "Legacy Name");
    assert_eq!(value.get::<_, String>(1), "owner_private");

    drop(client);
    target.cleanup();
}

#[test]
fn revision_constraints_and_guards_are_enforced() {
    let Some(target) = prepare("migration_0012_constraints") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping migration 0012 schema test");
        return;
    };
    let mut client = target.connect();
    apply(
        &mut client,
        &[
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
        ],
    );

    let person_id = seed_person(&mut client, "migration-0012-constraints");
    let revision_id: i64 = client
        .query_one(
            "INSERT INTO onboarding_profile_revision \
                 (person_id, field_key, prior_value_text, new_value_text, changed_by, \
                  revision_number, status, visibility) \
             VALUES ($1, 'biography', NULL, 'draft biography', 'owner-a', 1, 'draft', 'owner_private') \
             RETURNING revision_id",
            &[&person_id],
        )
        .expect("insert draft")
        .get(0);

    // One open draft/pending revision per field.
    let duplicate_open = client.execute(
        "INSERT INTO onboarding_profile_revision \
             (person_id, field_key, prior_value_text, new_value_text, changed_by, \
              revision_number, status, visibility) \
         VALUES ($1, 'biography', NULL, 'second draft', 'owner-a', 2, 'draft', 'owner_private')",
        &[&person_id],
    );
    assert!(duplicate_open.is_err(), "a second open revision must fail");

    // Version numbers are unique per field.
    client
        .execute(
            "UPDATE onboarding_profile_revision \
                SET status = 'pending_review', submitted_at = now() \
              WHERE revision_id = $1",
            &[&revision_id],
        )
        .expect("submit first revision");
    client
        .execute(
            "UPDATE onboarding_profile_revision SET status = 'rejected', \
                    reviewed_at = now(), reviewed_by = 'reviewer-a', \
                    review_note = 'synthetic rejection' \
              WHERE revision_id = $1",
            &[&revision_id],
        )
        .expect("close first revision");
    let duplicate_version = client.execute(
        "INSERT INTO onboarding_profile_revision \
             (person_id, field_key, prior_value_text, new_value_text, changed_by, \
              revision_number, status, visibility) \
         VALUES ($1, 'biography', NULL, 'same version', 'owner-a', 1, 'draft', 'owner_private')",
        &[&person_id],
    );
    assert!(
        duplicate_version.is_err(),
        "duplicate revision number must fail"
    );

    // Invalid state and visibility values are rejected by the schema.
    let invalid_status = client.execute(
        "INSERT INTO onboarding_profile_revision \
             (person_id, field_key, new_value_text, changed_by, revision_number, status, visibility) \
         VALUES ($1, 'biography', 'bad state', 'owner-a', 2, 'bogus', 'owner_private')",
        &[&person_id],
    );
    assert!(invalid_status.is_err(), "invalid status must fail");
    let invalid_visibility = client.execute(
        "INSERT INTO onboarding_profile_revision \
             (person_id, field_key, new_value_text, changed_by, revision_number, status, visibility) \
         VALUES ($1, 'biography', 'bad visibility', 'owner-a', 3, 'draft', 'public')",
        &[&person_id],
    );
    assert!(invalid_visibility.is_err(), "invalid visibility must fail");

    // Create a fresh submission and prove submitted content and self-review are
    // rejected at the database boundary.
    let submitted_id: i64 = client
        .query_one(
            "INSERT INTO onboarding_profile_revision \
                 (person_id, field_key, prior_value_text, new_value_text, changed_by, \
                  revision_number, status, visibility) \
             VALUES ($1, 'biography', NULL, 'submitted biography', 'owner-b', 4, 'draft', 'owner_private') \
             RETURNING revision_id",
            &[&person_id],
        )
        .expect("insert submitted draft")
        .get(0);
    client
        .execute(
            "UPDATE onboarding_profile_revision \
                SET status = 'pending_review', submitted_at = now() \
              WHERE revision_id = $1",
            &[&submitted_id],
        )
        .expect("submit revision");
    let changed_content = client.execute(
        "UPDATE onboarding_profile_revision SET new_value_text = 'changed' \
          WHERE revision_id = $1",
        &[&submitted_id],
    );
    assert!(changed_content.is_err(), "submitted content is immutable");
    let self_review = client.execute(
        "UPDATE onboarding_profile_revision \
            SET status = 'approved', reviewed_at = now(), reviewed_by = 'owner-b' \
          WHERE revision_id = $1",
        &[&submitted_id],
    );
    assert!(self_review.is_err(), "self-review must be rejected");
    client
        .execute(
            "UPDATE onboarding_profile_revision \
                SET status = 'approved', reviewed_at = now(), reviewed_by = 'reviewer-b' \
              WHERE revision_id = $1",
            &[&submitted_id],
        )
        .expect("independent approval");
    let terminal_update = client.execute(
        "UPDATE onboarding_profile_revision SET review_note = 'late edit' \
          WHERE revision_id = $1",
        &[&submitted_id],
    );
    assert!(terminal_update.is_err(), "terminal revision is immutable");
    let reopened = client.execute(
        "UPDATE onboarding_profile_revision SET status = 'draft' \
          WHERE revision_id = $1",
        &[&submitted_id],
    );
    assert!(
        reopened.is_err(),
        "a terminal revision cannot be reopened by direct SQL"
    );

    // An approved value update must point at the matching approved revision and
    // advance the version; a direct text overwrite is rejected.
    client
        .execute(
            "INSERT INTO onboarding_profile_value \
                 (person_id, field_key, value_text, updated_by, \
                  approved_revision_number, approved_revision_id) \
             VALUES ($1, 'biography', 'submitted biography', 'reviewer-b', 4, $2)",
            &[&person_id, &submitted_id],
        )
        .expect("insert value from approved revision");
    let direct_value_update = client.execute(
        "UPDATE onboarding_profile_value SET value_text = 'direct overwrite' \
          WHERE person_id = $1 AND field_key = 'biography'",
        &[&person_id],
    );
    assert!(
        direct_value_update.is_err(),
        "direct approved-value updates must not bypass the revision"
    );

    // A new row cannot be minted directly as approved.
    let direct_approval = client.execute(
        "INSERT INTO onboarding_profile_revision \
             (person_id, field_key, new_value_text, changed_by, revision_number, \
              status, visibility, reviewed_at, reviewed_by) \
         VALUES ($1, 'biography', 'direct approval', 'owner-c', 5, \
                 'approved', 'owner_private', now(), 'reviewer-c')",
        &[&person_id],
    );
    assert!(
        direct_approval.is_err(),
        "new revisions must begin as drafts"
    );

    // Review event history is append-only for updates.
    let event_id: i64 = client
        .query_one(
            "INSERT INTO onboarding_profile_review_event \
                 (revision_id, person_id, event, actor) \
             VALUES ($1, $2, 'pending_review', 'owner-b') \
             RETURNING event_id",
            &[&submitted_id, &person_id],
        )
        .expect("insert event")
        .get(0);
    let event_update = client.execute(
        "UPDATE onboarding_profile_review_event SET note = 'rewritten' WHERE event_id = $1",
        &[&event_id],
    );
    assert!(event_update.is_err(), "review events are append-only");
    let revision_delete = client.execute(
        "DELETE FROM onboarding_profile_revision WHERE revision_id = $1",
        &[&submitted_id],
    );
    assert!(
        revision_delete.is_err(),
        "terminal revision history cannot be deleted directly"
    );
    let event_delete = client.execute(
        "DELETE FROM onboarding_profile_review_event WHERE event_id = $1",
        &[&event_id],
    );
    assert!(
        event_delete.is_err(),
        "review events cannot be deleted directly"
    );

    // Deleting the owning person must still cascade through revision and event
    // history; the delete guards only reject direct child deletes.
    let cascade_person = seed_person(&mut client, "migration-0012-cascade");
    let cascade_revision: i64 = client
        .query_one(
            "INSERT INTO onboarding_profile_revision \
                 (person_id, field_key, new_value_text, changed_by, revision_number, \
                  status, visibility) \
             VALUES ($1, 'biography', 'cascade biography', 'owner-d', 1, \
                     'draft', 'owner_private') \
             RETURNING revision_id",
            &[&cascade_person],
        )
        .expect("insert cascade revision")
        .get(0);
    client
        .execute(
            "INSERT INTO onboarding_profile_review_event \
                 (revision_id, person_id, event, actor) \
             VALUES ($1, $2, 'draft', 'owner-d')",
            &[&cascade_revision, &cascade_person],
        )
        .expect("insert cascade event");
    client
        .execute(
            "DELETE FROM person WHERE person_id = $1",
            &[&cascade_person],
        )
        .expect("person deletion must cascade revision history");
    let remaining: i64 = client
        .query_one(
            "SELECT count(*) FROM onboarding_profile_revision WHERE person_id = $1",
            &[&cascade_person],
        )
        .expect("count cascade revisions")
        .get(0);
    assert_eq!(remaining, 0);

    // Re-applying the migration must not rewrite a current lifecycle row. The
    // legacy backfill is scoped to status IS NULL, and the version backfill is
    // collision-safe for already-numbered rows.
    apply(
        &mut client,
        &["genealogy/migrations/0012_profile_revisions.sql"],
    );
    let after_reapply = client
        .query_one(
            "SELECT status, review_note, revision_number, changed_by \
               FROM onboarding_profile_revision WHERE revision_id = $1",
            &[&submitted_id],
        )
        .expect("read revision after reapply");
    assert_eq!(after_reapply.get::<_, String>(0), "approved");
    assert_eq!(after_reapply.get::<_, Option<String>>(1), None);
    assert_eq!(after_reapply.get::<_, i64>(2), 4);
    assert_eq!(after_reapply.get::<_, String>(3), "owner-b");

    drop(client);
    target.cleanup();
}
