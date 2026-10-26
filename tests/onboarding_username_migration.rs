//! Username migration uses synthetic rows in an isolated database.
#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

#[test]
fn username_migration_normalizes_existing_rows_and_is_repeatable() {
    let Some(base) = genealogy_isolation::base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping username migration test");
        return;
    };
    let target = genealogy_isolation::create_isolated_target(&base, "username_migration");
    target.apply_migrations();
    let mut client = target.connect();
    client.batch_execute(
        "ALTER TABLE onboarding_username_reservation DROP CONSTRAINT onboarding_username_normalized;
         INSERT INTO person (person_key, display_name)
         VALUES ('synthetic-username-migration', 'Synthetic Person');
         INSERT INTO onboarding_username_reservation
             (person_id, username, username_key, status, reserved_by)
         SELECT person_id, 'ĐTHỒNG', 'đthồng', 'reserved', 'synthetic-operator'
           FROM person WHERE person_key = 'synthetic-username-migration';"
    ).expect("seed legacy username");
    let migration = include_str!("../genealogy/migrations/0030_lowercase_usernames.sql");
    for _ in 0..2 {
        client
            .batch_execute(migration)
            .expect("apply username migration");
        let row = client
            .query_one(
                "SELECT username, username_key FROM onboarding_username_reservation",
                &[],
            )
            .expect("read reservation");
        assert_eq!(row.get::<_, String>(0), "dthong");
        assert_eq!(row.get::<_, String>(1), "dthong");
    }
    assert!(client
        .execute(
            "UPDATE onboarding_username_reservation SET username = 'ĐTHỒNG'",
            &[]
        )
        .is_err());
    drop(client);
    target.cleanup();
}
