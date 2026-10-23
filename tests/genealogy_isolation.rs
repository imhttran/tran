//! TDI-004 regression tests for the per-suite genealogy isolation helper.
//!
//! These tests exercise `tests/support/genealogy_isolation.rs` directly:
//! (a) the guard refuses a non-`_test`/`_disposable` database name;
//! (b) `create_isolated_target` produces a uniquely named owned target per call;
//! (c) `cleanup` drops only the owned target, leaving a second target intact;
//! (d) a migrated suite does not reset state owned by another target.
//!
//! When `GENEALOGY_DATABASE_URL` is unset, every test skips cleanly. Structured
//! `{"Action":"pass",...}` events are emitted only after every assertion passes.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use genealogy_isolation::{base_dsn, create_isolated_target, validate_dsn};

/// The guard must refuse a database name that is neither `_test` nor
/// `_disposable`. We pass a clearly non-disposable DSN to the pure guard and
/// assert that it panics before any DDL or DML runs. This test does not touch
/// the environment, so it is safe under parallel execution.
#[test]
fn guard_refuses_non_disposable_database() {
    let bad = "postgres://rag:rag@127.0.0.1:5432/genealogy_prod?sslmode=disable";
    let outcome = std::panic::catch_unwind(|| validate_dsn(bad, None));

    let payload = outcome.expect_err("validate_dsn must panic for a non-disposable database");
    let message = panic_message(&payload);
    assert!(
        message.contains("refusing to write to non-disposable genealogy database"),
        "guard message must explain the refusal: {message}"
    );

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"guard_refuses_non_disposable_database\"}}"
    );
}

/// The guard must also refuse a disposable-looking DSN that points at the RAG
/// `DATABASE_URL` target.
#[test]
fn guard_refuses_the_rag_database_target() {
    let dsn = "postgres://rag:rag@127.0.0.1:5432/genealogy_test?sslmode=disable";
    let rag = "postgres://rag:rag@127.0.0.1:5432/genealogy_test?sslmode=disable";
    let outcome = std::panic::catch_unwind(|| validate_dsn(dsn, Some(rag)));

    let payload = outcome.expect_err("validate_dsn must refuse the RAG database target");
    let message = panic_message(&payload);
    assert!(
        message.contains("must not target the RAG database"),
        "guard message must explain the RAG refusal: {message}"
    );

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"guard_refuses_the_rag_database_target\"}}"
    );
}

/// Two `create_isolated_target` calls for the same suite label must produce
/// different `owned_name` values, and both must be cleaned up.
#[test]
fn isolated_targets_are_unique_per_call() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping isolation uniqueness test");
        return;
    };

    let first = create_isolated_target(&base, "isolation_unique");
    let second = create_isolated_target(&base, "isolation_unique");
    assert_ne!(
        first.owned_name, second.owned_name,
        "two isolated targets must have distinct owned names"
    );

    first.cleanup();
    second.cleanup();

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"isolated_targets_are_unique_per_call\"}}"
    );
}

/// Cleanup of one owned target must not remove the second target's state.
#[test]
fn cleanup_drops_only_the_owned_target() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping isolation cleanup test");
        return;
    };

    let owned = create_isolated_target(&base, "isolation_cleanup_a");
    let survivor = create_isolated_target(&base, "isolation_cleanup_b");
    assert_ne!(
        owned.dsn, survivor.dsn,
        "each target must own a distinct database, not a shared DSN"
    );
    assert_ne!(owned.owned_name, survivor.owned_name);

    owned.apply_migrations();
    survivor.apply_migrations();

    {
        let mut owned_client = owned.connect();
        owned_client
            .batch_execute(
                "INSERT INTO person (person_key, display_name) VALUES ('owned-target', 'Owned');",
            )
            .expect("insert into owned target");
    }
    {
        let mut survivor_client = survivor.connect();
        survivor_client
            .batch_execute(
                "INSERT INTO person (person_key, display_name) VALUES ('survivor-target', 'Survivor');",
            )
            .expect("insert into survivor target");
    }

    owned.cleanup();

    // The surviving target still holds its row.
    let mut survivor_client = survivor.connect();
    let count: i64 = survivor_client
        .query_one(
            "SELECT count(*) FROM person WHERE person_key = 'survivor-target'",
            &[],
        )
        .expect("count survivor rows")
        .get(0);
    assert_eq!(count, 1, "cleanup must not touch the other owned target");
    drop(survivor_client);

    survivor.cleanup();

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"cleanup_drops_only_the_owned_target\"}}"
    );
}

/// A migrated suite writes to its owned target only; state owned by a second
/// (independently migrated) target must remain intact.
#[test]
fn migrated_suite_does_not_reset_other_target_state() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping isolation cross-suite test");
        return;
    };

    let suite_a = create_isolated_target(&base, "isolation_suite_a");
    suite_a.apply_migrations();
    {
        let mut client = suite_a.connect();
        client
            .batch_execute(
                "INSERT INTO person (person_key, display_name) VALUES ('suite-a', 'Suite A');",
            )
            .expect("suite A writes its own row");
    }

    let suite_b = create_isolated_target(&base, "isolation_suite_b");
    suite_b.apply_migrations();
    assert_ne!(
        suite_a.dsn, suite_b.dsn,
        "each suite must target its own database, not a shared DSN"
    );
    {
        let mut client = suite_b.connect();
        client
            .batch_execute(
                "INSERT INTO person (person_key, display_name) VALUES ('suite-b', 'Suite B');",
            )
            .expect("suite B writes its own row");
    }

    // Suite A's row is still present after suite B migrated and wrote.
    {
        let mut client = suite_a.connect();
        let count: i64 = client
            .query_one(
                "SELECT count(*) FROM person WHERE person_key = 'suite-a'",
                &[],
            )
            .expect("count suite A rows")
            .get(0);
        assert_eq!(count, 1, "suite B must not reset suite A state");
    }

    suite_a.cleanup();
    suite_b.cleanup();

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"migrated_suite_does_not_reset_other_target_state\"}}"
    );
}

fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// A long suite label must not push the owned database name past PostgreSQL's
/// 63-byte identifier limit. `pg_database.datname` is a `name`, so an
/// over-long value fails with SQLSTATE 42622 rather than being truncated; the
/// label must absorb the truncation and the unique token must survive it.
#[test]
fn isolated_target_name_fits_the_identifier_limit() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping identifier length test");
        return;
    };

    let label = "appointments_self_approval_with_a_deliberately_long_label_name";
    let first = create_isolated_target(&base, label);
    let second = create_isolated_target(&base, label);

    assert!(
        first.owned_name.len() <= 63,
        "owned name exceeds the identifier limit: {} ({} bytes)",
        first.owned_name,
        first.owned_name.len()
    );
    assert!(
        first.owned_name.starts_with("genealogy_"),
        "unexpected name prefix: {}",
        first.owned_name
    );
    assert!(
        first.owned_name.ends_with("_test"),
        "the disposable-target suffix must survive truncation: {}",
        first.owned_name
    );
    assert_ne!(
        first.owned_name, second.owned_name,
        "truncating the label must not collapse the unique token"
    );

    first.cleanup();
    second.cleanup();

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"isolated_target_name_fits_the_identifier_limit\"}}"
    );
}

/// A failing assertion must still release the owned database. `IsolatedTarget`
/// drops it from `Drop`, so a panic between creation and `cleanup()` cannot leak
/// a database into the disposable cluster.
#[test]
fn dropping_an_isolated_target_drops_the_owned_database() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping drop-cleanup test");
        return;
    };

    let target = create_isolated_target(&base, "isolation_drop");
    let dsn = target.dsn.clone();
    drop(target);

    let reconnected = postgres::Client::connect(&dsn, postgres::NoTls);
    assert!(
        reconnected.is_err(),
        "Drop must drop the owned database, but it is still reachable"
    );

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"dropping_an_isolated_target_drops_the_owned_database\"}}"
    );
}

/// When no maintenance database is reachable the helper must fail loudly instead
/// of running the suite against a shared database. Port 1 refuses connections, so
/// neither `postgres` nor `template1` can be used.
#[test]
#[should_panic(expected = "CREATEDB")]
fn create_isolated_target_fails_loudly_without_createdb() {
    let base = "postgres://nobody@127.0.0.1:1/unreachable_test?sslmode=disable";
    let _ = create_isolated_target(base, "unreachable");
}

/// A panic between creating a target and calling `cleanup()` must still release
/// the owned database: `IsolatedTarget` drops it from `Drop`. This is the
/// failing-assertion path, which is how a real suite leaks state.
#[test]
fn a_panic_after_creation_still_drops_the_owned_database() {
    let Some(base) = base_dsn() else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping panic-cleanup test");
        return;
    };

    let slot: std::sync::Arc<std::sync::Mutex<Option<String>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let writer = std::sync::Arc::clone(&slot);
    let outcome = std::panic::catch_unwind(move || {
        let target = create_isolated_target(&base, "isolation_panic");
        *writer.lock().expect("record dsn") = Some(target.dsn.clone());
        panic!("simulated failing assertion");
    });

    assert!(outcome.is_err(), "the simulated assertion must panic");
    let dsn = slot
        .lock()
        .expect("read recorded dsn")
        .clone()
        .expect("target was created before the panic");
    assert!(
        postgres::Client::connect(&dsn, postgres::NoTls).is_err(),
        "a panic must not leak the owned database"
    );

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"a_panic_after_creation_still_drops_the_owned_database\"}}"
    );
}

/// The helper's migration list must track the repository exactly, so a new
/// migration file cannot silently be left out of every isolated target.
#[test]
fn migration_list_matches_the_repository() {
    let mut listed: Vec<String> = genealogy_isolation::GENEALOGY_MIGRATIONS
        .iter()
        .map(|path| path.to_string())
        .collect();
    listed.sort();

    let mut on_disk: Vec<String> = std::fs::read_dir("genealogy/migrations")
        .expect("read genealogy/migrations")
        .map(|entry| entry.expect("migration dir entry").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("sql"))
        .map(|path| path.to_string_lossy().replace('\\', "/"))
        .collect();
    on_disk.sort();

    assert_eq!(
        listed, on_disk,
        "GENEALOGY_MIGRATIONS must list every migration file exactly once"
    );

    println!(
        "{{\"Action\":\"pass\",\"Package\":\"genealogy\",\"Test\":\"migration_list_matches_the_repository\"}}"
    );
}
