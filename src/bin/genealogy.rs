//! Local-only genealogy CLI.
//!
//! Every database operation is guarded by `GENEALOGY_DATABASE_URL`; the command
//! never falls back to the RAG database and never exposes an HTTP route.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use rag_foundation::config::Config;
use rag_foundation::genealogy::migrate::{self, MIGRATIONS_DIR};
use rag_foundation::genealogy::parser::{self, AssertionKind, Evidence, ParseOutcome};
use rag_foundation::genealogy::query;
use rag_foundation::genealogy::reimport::{
    self, ImportAssertion, ImportDocument, ImportLink, ImportPerson,
};
use rag_foundation::genealogy::relationships::{
    self, NameIndex, PersonName, RelationshipCandidate,
};
use rag_foundation::genealogy::report;
use rag_foundation::genealogy::resolve_genealogy_dsn;
use rag_foundation::genealogy::review::{self, ReviewRequest, ReviewStatus};
use rag_foundation::inventory::sha256_hex;
use rag_foundation::storage_pg::create_pool;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("genealogy: {err}");
            ExitCode::from(2)
        }
    }
}

fn run(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("migrate") => migrate_cmd(),
        Some("import") => import_cmd(&args[1..]),
        Some("unresolved") => unresolved_cmd(args.get(1).map(String::as_str)),
        Some("person") => person_cmd(args.get(1).ok_or("usage: person <person-key>")?),
        Some("relationship") => relationship_cmd(
            args.get(1)
                .ok_or("usage: relationship <from-key> <to-key>")?,
            args.get(2)
                .ok_or("usage: relationship <from-key> <to-key>")?,
        ),
        Some("review") => review_cmd(&args[1..]),
        _ => Err(
            "usage: genealogy <migrate|import|unresolved|person|relationship|review> ...".into(),
        ),
    }
}

fn config() -> Result<Config, String> {
    Config::load().map_err(|err| err.to_string())
}

fn guarded_client() -> Result<
    (
        Config,
        r2d2::PooledConnection<r2d2_postgres::PostgresConnectionManager<postgres::NoTls>>,
    ),
    String,
> {
    let cfg = config()?;
    let dsn = resolve_genealogy_dsn(&cfg).map_err(|err| err.to_string())?;
    let pool = create_pool(&dsn).map_err(|err| err.to_string())?;
    let conn = pool.get().map_err(|err| err.to_string())?;
    Ok((cfg, conn))
}

fn migrate_cmd() -> Result<(), String> {
    let cfg = config()?;
    let report =
        migrate::run_migrations(&cfg, Path::new(MIGRATIONS_DIR)).map_err(|err| err.to_string())?;
    println!("applied: {}", report.applied.len());
    for version in &report.applied {
        println!("  {version}");
    }
    println!("already applied: {}", report.already_applied.len());
    Ok(())
}

fn import_cmd(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("--dry-run") => {
            let input = args
                .get(1)
                .ok_or("usage: import --dry-run <catalog> <report>")?;
            let output = args
                .get(2)
                .ok_or("usage: import --dry-run <catalog> <report>")?;
            let report = parser::run_dry_run(Path::new(input), Path::new(output))
                .map_err(|err| err.to_string())?;
            println!(
                "processed={} parsed={} ambiguous={} rejected={} unmatched={} \
                 birth_dates={} death_dates={} unresolved_dates={}",
                report.processed,
                report.parsed,
                report.ambiguous,
                report.rejected,
                report.unmatched,
                report.birth_dates,
                report.death_dates,
                report.unresolved_dates
            );
            Ok(())
        }
        Some("--apply") => {
            let input = args.get(1).ok_or("usage: import --apply <catalog>")?;
            import_apply(Path::new(input))
        }
        _ => Err("usage: import <--dry-run <catalog> <report>|--apply <catalog>>".into()),
    }
}

fn import_apply(input: &Path) -> Result<(), String> {
    let (cfg, mut conn) = guarded_client()?;
    migrate::run_migrations(&cfg, Path::new(MIGRATIONS_DIR)).map_err(|err| err.to_string())?;

    let text =
        fs::read_to_string(input).map_err(|err| format!("read {}: {err}", input.display()))?;
    let parse = parser::parse_catalog(&text);
    let document_ref = input.to_string_lossy().to_string();
    let sha256 = sha256_hex(&fs::read(input).map_err(|err| err.to_string())?);

    let mut persons = Vec::new();
    let mut names = Vec::new();
    let mut candidates = Vec::new();
    for entry in &parse.entries {
        if entry.outcome == ParseOutcome::Rejected {
            continue;
        }
        let display_name = entry
            .display_name
            .clone()
            .unwrap_or_else(|| entry.key.clone());
        let aliases: Vec<String> = entry
            .assertions
            .iter()
            .filter(|assertion| assertion.kind == AssertionKind::NameVariant)
            .map(|assertion| assertion.value_text.clone())
            .collect();
        names.push(PersonName {
            person_key: entry.key.clone(),
            display_name: display_name.clone(),
            aliases,
        });
        persons.push(ImportPerson {
            person_key: entry.key.clone(),
            display_name,
        });

        for assertion in &entry.assertions {
            if !matches!(
                assertion.kind,
                AssertionKind::ParentChild | AssertionKind::Spouse
            ) {
                continue;
            }
            candidates.push(RelationshipCandidate {
                owner_key: entry.key.clone(),
                kind: assertion_kind(assertion.kind).to_string(),
                target: assertion.value_text.clone(),
                // A statement the source hedges or marks as an inference stays
                // an assertion for review, but it never becomes a graph edge.
                resolvable: assertion.reason.is_none(),
            });
        }
    }

    // Resolve every relationship target against this edition's own names. A
    // target that matches no entry, or more than one (namesakes), stays
    // unresolved: no edge and no object binding is invented.
    let index = NameIndex::new(&names);
    let (planned_links, unresolved) = relationships::plan_links(&candidates, &index);
    let mut resolved: std::collections::BTreeMap<(String, String, String), String> =
        std::collections::BTreeMap::new();
    for (candidate, binding) in candidates
        .iter()
        .zip(relationships::object_bindings(&candidates, &index))
    {
        if let Some(object_key) = binding {
            resolved.insert(
                (
                    candidate.owner_key.clone(),
                    candidate.kind.clone(),
                    candidate.target.clone(),
                ),
                object_key,
            );
        }
    }

    let mut assertions = Vec::new();
    for entry in &parse.entries {
        if entry.outcome == ParseOutcome::Rejected {
            continue;
        }
        for assertion in &entry.assertions {
            let evidence: Vec<(String, i32)> = assertion
                .evidence
                .iter()
                .filter_map(|item| match item {
                    Evidence::Page(page) => Some((document_ref.clone(), *page as i32)),
                    Evidence::Unresolved(_) => None,
                })
                .collect();
            let kind = assertion_kind(assertion.kind).to_string();
            let object_key = resolved
                .get(&(
                    entry.key.clone(),
                    kind.clone(),
                    assertion.value_text.clone(),
                ))
                .cloned();
            assertions.push(ImportAssertion {
                person_key: entry.key.clone(),
                kind,
                key: assertion.assertion_key.clone(),
                value_text: Some(assertion.value_text.clone()),
                object_key,
                evidence,
                date: assertion.date.clone(),
            });
        }
    }

    let links: Vec<ImportLink> = planned_links
        .iter()
        .map(|link| ImportLink {
            kind: link.kind.clone(),
            from_key: link.from_key.clone(),
            to_key: link.to_key.clone(),
        })
        .collect();

    let document = ImportDocument {
        relative_path: document_ref,
        sha256,
        source_kind: "catalog".to_string(),
        persons,
        assertions,
        links,
    };
    let report =
        reimport::reimport_document(&mut conn, &document).map_err(|err| err.to_string())?;
    println!(
        "inserted={} unchanged={} reopened={} links_inserted={} links_unchanged={} \
         relationships_resolved={} relationships_unresolved={}",
        report.inserted,
        report.unchanged,
        report.reopened,
        report.links_inserted,
        report.links_unchanged,
        planned_links.len(),
        unresolved.len()
    );
    for item in &unresolved {
        println!(
            "unresolved relationship: {} {} -> {:?} ({})",
            item.owner_key, item.kind, item.target, item.reason
        );
    }
    Ok(())
}

fn assertion_kind(kind: AssertionKind) -> &'static str {
    match kind {
        AssertionKind::DisplayName => "display_name",
        AssertionKind::NameVariant => "name_variant",
        AssertionKind::Branch => "branch",
        AssertionKind::ParentChild => "parent_child",
        AssertionKind::Spouse => "spouse",
        AssertionKind::BirthDate => "birth_date",
        AssertionKind::DeathDate => "death_date",
        AssertionKind::DateUnresolved => "date_unresolved",
    }
}

fn unresolved_cmd(path: Option<&str>) -> Result<(), String> {
    let (_cfg, mut conn) = guarded_client()?;
    match path {
        Some(path) => {
            let report = report::write_unresolved_report(&mut conn, Path::new(path))
                .map_err(|err| err.to_string())?;
            println!("unresolved: {}", report.entries.len());
        }
        None => {
            let report =
                report::build_unresolved_report(&mut conn).map_err(|err| err.to_string())?;
            let json = serde_json::to_string_pretty(&report).map_err(|err| err.to_string())?;
            println!("{json}");
        }
    }
    Ok(())
}

fn person_cmd(key: &str) -> Result<(), String> {
    let (_cfg, mut conn) = guarded_client()?;
    match query::lookup_person(&mut conn, key).map_err(|err| err.to_string())? {
        Some(person) => {
            println!("person: {}", person.person_key);
            if let Some(name) = person.display_name {
                println!("display_name: {name}");
            }
            for assertion in person.assertions {
                println!(
                    "assertion {} {} {} status={} citations={}",
                    assertion.assertion_id,
                    assertion.kind,
                    assertion.key,
                    assertion.review_status,
                    assertion.citations.len()
                );
                if let Some(date) = assertion.date {
                    println!(
                        "  date start={} end={:?} precision={} qualifier={} raw={:?}",
                        date.start,
                        date.end,
                        date.precision.as_str(),
                        date.qualifier.as_str(),
                        assertion.value_text
                    );
                }
            }
        }
        None => println!("person: not found"),
    }
    Ok(())
}

fn relationship_cmd(from: &str, to: &str) -> Result<(), String> {
    let (_cfg, mut conn) = guarded_client()?;
    let relationship =
        query::lookup_relationship(&mut conn, from, to).map_err(|err| err.to_string())?;
    println!(
        "{} -> {} kind={} status={} citations={}",
        relationship.from_person_key,
        relationship.to_person_key,
        relationship.relationship_kind,
        relationship.status,
        relationship.citations.len()
    );
    Ok(())
}

fn review_cmd(args: &[String]) -> Result<(), String> {
    let assertion_id = args
        .first()
        .ok_or("usage: review <assertion-id> <status> --reviewer R --evidence E [--note N]")?
        .parse::<i64>()
        .map_err(|_| "assertion id must be an integer".to_string())?;
    let status = ReviewStatus::parse(args.get(1).ok_or("missing review status")?)
        .ok_or_else(|| "unknown review status".to_string())?;
    let mut reviewer = "";
    let mut evidence = "";
    let mut note = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--reviewer" => {
                reviewer = args.get(i + 1).ok_or("--reviewer requires a value")?;
                i += 2;
            }
            "--evidence" => {
                evidence = args.get(i + 1).ok_or("--evidence requires a value")?;
                i += 2;
            }
            "--note" => {
                note = Some(args.get(i + 1).ok_or("--note requires a value")?.as_str());
                i += 2;
            }
            other => return Err(format!("unknown review option: {other}")),
        }
    }
    let (_cfg, mut conn) = guarded_client()?;
    let request = ReviewRequest {
        assertion_id,
        new_status: status,
        reviewer,
        evidence_ref: evidence,
        note,
    };
    let outcome = review::review_assertion(&mut conn, &request).map_err(|err| err.to_string())?;
    println!(
        "assertion {} {} -> {}",
        outcome.assertion_id, outcome.from_status, outcome.to_status
    );
    Ok(())
}
