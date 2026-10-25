//! Local-only genealogy CLI.
//!
//! Every database operation is guarded by `GENEALOGY_DATABASE_URL`; the command
//! never falls back to the RAG database and never exposes an HTTP route.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use rag_foundation::config::Config;
use rag_foundation::genealogy::authority::{self, ApplyInput, ReverseInput, ScopeSpec};
use rag_foundation::genealogy::migrate::{self, MIGRATIONS_DIR};
use rag_foundation::genealogy::narrative::{self, PassageDecision, PassageVisibility};
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
        Some("narrative") => narrative_cmd(&args[1..]),
        Some("authority-preview") => authority_preview_cmd(&args[1..]),
        Some("authority-apply") => authority_apply_cmd(&args[1..]),
        Some("authority-reverse") => authority_reverse_cmd(&args[1..]),
        Some("authority-history") => authority_history_cmd(args.get(1).map(String::as_str)),
        _ => Err(
            "usage: genealogy <migrate|import|unresolved|person|relationship|review|narrative> ..."
                .into(),
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

/// A single guarded connection for the authority commands, which take a
/// `postgres::Client` because they accept both a client and a transaction.
fn guarded_raw_client() -> Result<postgres::Client, String> {
    let cfg = config()?;
    let dsn = resolve_genealogy_dsn(&cfg).map_err(|err| err.to_string())?;
    postgres::Client::connect(&dsn, postgres::NoTls).map_err(|err| err.to_string())
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

// --- authoritative source decisions ---------------------------------------

/// The hash recorded for a document version, so an operator does not have to
/// copy it out of the extraction screen.
fn stored_sha(conn: &mut postgres::Client, version_id: i64) -> Result<String, String> {
    conn.query_opt(
        "SELECT content_sha256 FROM document_extraction
          WHERE document_version_id = $1 ORDER BY extraction_id DESC LIMIT 1",
        &[&version_id],
    )
    .map_err(|err| err.to_string())?
    .map(|row| row.get(0))
    .ok_or_else(|| format!("no reviewed extraction exists for document version {version_id}"))
}

struct AuthorityOptions {
    version_id: i64,
    scope: String,
    person_key: Option<String>,
    assertion_ids: Vec<i64>,
    remove_assertion_ids: Vec<i64>,
    sha: Option<String>,
    reason: String,
    actor: String,
    preview_hash: String,
    confirm: bool,
    acknowledge_conflicts: bool,
}

fn parse_ids(value: &str) -> Result<Vec<i64>, String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.parse::<i64>()
                .map_err(|_| format!("not an assertion id: {item}"))
        })
        .collect()
}

fn parse_authority_options(args: &[String]) -> Result<AuthorityOptions, String> {
    let version = args
        .first()
        .ok_or("usage: authority-preview <document-version-id> [options]")?;
    let mut options = AuthorityOptions {
        version_id: version
            .parse()
            .map_err(|_| "document version id must be an integer".to_string())?,
        scope: "document".to_string(),
        person_key: None,
        assertion_ids: Vec::new(),
        remove_assertion_ids: Vec::new(),
        sha: None,
        reason: String::new(),
        actor: String::new(),
        preview_hash: String::new(),
        confirm: false,
        acknowledge_conflicts: false,
    };
    let mut index = 1;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = |offset: usize| -> Result<&str, String> {
            args.get(index + offset)
                .map(String::as_str)
                .ok_or_else(|| format!("{flag} requires a value"))
        };
        match flag {
            "--scope" => {
                options.scope = value(1)?.to_string();
                index += 2;
            }
            "--person" => {
                options.person_key = Some(value(1)?.to_string());
                index += 2;
            }
            "--assertions" => {
                options.assertion_ids = parse_ids(value(1)?)?;
                index += 2;
            }
            "--remove" => {
                options.remove_assertion_ids = parse_ids(value(1)?)?;
                index += 2;
            }
            "--sha" => {
                options.sha = Some(value(1)?.to_string());
                index += 2;
            }
            "--reason" => {
                options.reason = value(1)?.to_string();
                index += 2;
            }
            "--actor" => {
                options.actor = value(1)?.to_string();
                index += 2;
            }
            "--preview-hash" => {
                options.preview_hash = value(1)?.to_string();
                index += 2;
            }
            "--confirm" => {
                options.confirm = true;
                index += 1;
            }
            "--acknowledge-conflicts" => {
                options.acknowledge_conflicts = true;
                index += 1;
            }
            other => return Err(format!("unknown authority option: {other}")),
        }
    }
    Ok(options)
}

fn authority_preview_cmd(args: &[String]) -> Result<(), String> {
    let options = parse_authority_options(args)?;
    let mut conn = guarded_raw_client()?;
    let sha = match options.sha {
        Some(sha) => sha,
        None => stored_sha(&mut conn, options.version_id)?,
    };
    let spec = ScopeSpec {
        scope: &options.scope,
        person_key: options.person_key.as_deref(),
        assertion_ids: &options.assertion_ids,
        remove_assertion_ids: &options.remove_assertion_ids,
    };
    let preview = authority::preview(&mut conn, options.version_id, &sha, &spec)
        .map_err(|err| err.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&preview).map_err(|err| err.to_string())?
    );
    Ok(())
}

fn authority_apply_cmd(args: &[String]) -> Result<(), String> {
    let options = parse_authority_options(args)?;
    let mut conn = guarded_raw_client()?;
    let sha = match options.sha {
        Some(sha) => sha,
        None => stored_sha(&mut conn, options.version_id)?,
    };
    let spec = ScopeSpec {
        scope: &options.scope,
        person_key: options.person_key.as_deref(),
        assertion_ids: &options.assertion_ids,
        remove_assertion_ids: &options.remove_assertion_ids,
    };
    let report = authority::apply(
        &mut conn,
        ApplyInput {
            document_version_id: options.version_id,
            content_sha256: &sha,
            spec,
            reason: &options.reason,
            actor: &options.actor,
            preview_hash: &options.preview_hash,
            confirm: options.confirm,
            acknowledge_conflicts: options.acknowledge_conflicts,
        },
    )
    .map_err(|err| err.to_string())?;
    println!(
        "decision={} status={} applied={} superseded={} removed={}",
        report.decision_id, report.status, report.applied, report.superseded, report.removed
    );
    Ok(())
}

fn authority_reverse_cmd(args: &[String]) -> Result<(), String> {
    let decision_id = args
        .first()
        .ok_or("usage: authority-reverse <decision-id> --reason R --actor A")?
        .parse::<i64>()
        .map_err(|_| "decision id must be an integer".to_string())?;
    let mut reason = String::new();
    let mut actor = String::new();
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--reason" => {
                reason = args
                    .get(index + 1)
                    .ok_or("--reason requires a value")?
                    .clone();
                index += 2;
            }
            "--actor" => {
                actor = args
                    .get(index + 1)
                    .ok_or("--actor requires a value")?
                    .clone();
                index += 2;
            }
            other => return Err(format!("unknown reversal option: {other}")),
        }
    }
    let mut conn = guarded_raw_client()?;
    let report = authority::reverse(
        &mut conn,
        ReverseInput {
            decision_id,
            reason: &reason,
            actor: &actor,
        },
    )
    .map_err(|err| err.to_string())?;
    println!(
        "decision={} status={} restored={} superseded={}",
        report.decision_id, report.status, report.applied, report.superseded
    );
    Ok(())
}

fn authority_history_cmd(person_key: Option<&str>) -> Result<(), String> {
    let mut conn = guarded_raw_client()?;
    let decisions =
        authority::list_decisions(&mut conn, person_key).map_err(|err| err.to_string())?;
    println!(
        "{}",
        serde_json::to_string_pretty(&decisions).map_err(|err| err.to_string())?
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

/// `genealogy narrative ...`: dry run, import, and review archive passages.
fn narrative_cmd(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("dry-run") => narrative_dry_run(&args[1..]),
        Some("import") => narrative_import(&args[1..]),
        Some("review") => narrative_review(&args[1..]),
        Some("link") => narrative_link(&args[1..]),
        Some("passages") => narrative_passages(&args[1..]),
        _ => {
            Err("usage: genealogy narrative <dry-run|import|review|link|passages> ...".to_string())
        }
    }
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn narrative_source(args: &[String]) -> Result<(String, String, String), String> {
    let source = args
        .first()
        .filter(|arg| !arg.starts_with("--"))
        .ok_or("usage: narrative <verb> <source.md> --language vi [--kind translation]")?;
    let language = flag_value(args, "--language").unwrap_or_else(|| "vi".to_string());
    if language != "vi" && language != "en" {
        return Err("--language must be vi or en".to_string());
    }
    let kind = flag_value(args, "--kind").unwrap_or_else(|| "translation".to_string());
    Ok((source.clone(), language, kind))
}

fn read_narrative(source: &str) -> Result<(narrative::NarrativeParse, String), String> {
    let bytes = fs::read(source).map_err(|err| format!("read {source}: {err}"))?;
    let text =
        String::from_utf8(bytes.clone()).map_err(|err| format!("{source} is not UTF-8: {err}"))?;
    Ok((
        narrative::parse_narrative(&text, source),
        sha256_hex(&bytes),
    ))
}

fn narrative_dry_run(args: &[String]) -> Result<(), String> {
    let (source, language, _kind) = narrative_source(args)?;
    let (parse, _sha) = read_narrative(&source)?;
    let (_cfg, mut conn) = guarded_client()?;
    let people = narrative::linkable_people(&mut conn).map_err(|err| err.to_string())?;
    if people.is_empty() {
        eprintln!(
            "narrative dry run: the genealogy database has no catalog people; \
             every heading will be reported as unmatched"
        );
    }

    let mut resolved = 0usize;
    let mut ambiguous = 0usize;
    let mut unmatched = 0usize;
    let mut report = String::from("# Narrative extraction dry run\n\n");
    report.push_str("This report is local and never committed. Bodies are not included.\n\n");
    report.push_str(&format!("- source: {source}\n"));
    report.push_str(&format!("- language: {language}\n"));
    report.push_str(&format!("- passages: {}\n", parse.passages.len()));
    report.push_str(&format!("- pages: {}\n", parse.page_count()));
    report.push_str(&format!("- unpaged_sections: {}\n", parse.unpaged_sections));
    for passage in &parse.passages {
        let links = narrative::propose_links(passage, &people);
        let resolved_here = links
            .iter()
            .filter(|link| link.status == narrative::LinkStatus::Resolved)
            .count();
        let ambiguous_here = links
            .iter()
            .filter(|link| link.status == narrative::LinkStatus::Ambiguous)
            .count();
        resolved += resolved_here;
        ambiguous += ambiguous_here;
        if passage.section_title.is_some() && links.is_empty() {
            unmatched += 1;
        }
        report.push_str(&format!(
            "- p.{}{} | {} | resolved {} | review {} | links {}\n",
            passage.page_start,
            if passage.page_end > passage.page_start {
                format!("-{}", passage.page_end)
            } else {
                String::new()
            },
            passage.section_title.as_deref().unwrap_or("(untitled)"),
            resolved_here,
            ambiguous_here,
            links
                .iter()
                .map(|link| link.person_key.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    report.push_str(&format!(
        "\n## Totals\n\n- resolved_links: {resolved}\n- ambiguous_links: {ambiguous}\n- unmatched_headings: {unmatched}\n"
    ));

    match flag_value(args, "--report") {
        Some(path) => {
            fs::write(&path, report).map_err(|err| format!("write {path}: {err}"))?;
        }
        None => print!("{report}"),
    }
    println!(
        "passages={} pages={} unresolved_sections={} resolved_links={} ambiguous_links={} unmatched_headings={}",
        parse.passages.len(),
        parse.page_count(),
        parse.unpaged_sections,
        resolved,
        ambiguous,
        unmatched
    );
    Ok(())
}

fn narrative_import(args: &[String]) -> Result<(), String> {
    let (source, language, kind) = narrative_source(args)?;
    let (parse, sha) = read_narrative(&source)?;
    let (cfg, mut conn) = guarded_client()?;
    migrate::run_migrations(&cfg, Path::new(MIGRATIONS_DIR)).map_err(|err| err.to_string())?;
    let people = narrative::linkable_people(&mut conn).map_err(|err| err.to_string())?;

    let mut tx = conn.transaction().map_err(|err| err.to_string())?;
    let report =
        narrative::import_narrative(&mut tx, &source, &sha, &kind, &language, &parse, &people)
            .map_err(|err| err.to_string())?;
    tx.commit().map_err(|err| err.to_string())?;
    println!(
        "inserted={} updated={} unchanged={} revisions={} links_inserted={} links_kept={}",
        report.inserted,
        report.updated,
        report.unchanged,
        report.revisions,
        report.links_inserted,
        report.links_kept
    );
    Ok(())
}

fn narrative_review(args: &[String]) -> Result<(), String> {
    let passage_id: i64 = args
        .first()
        .ok_or("usage: narrative review <passage-id> <pending|approved|rejected> --reviewer R [--visibility family]")?
        .parse()
        .map_err(|_| "passage id must be an integer".to_string())?;
    let decision = args
        .get(1)
        .and_then(|value| PassageDecision::parse(value))
        .ok_or("review status must be pending, approved, or rejected")?;
    let reviewer = flag_value(args, "--reviewer").ok_or("--reviewer is required")?;
    let note = flag_value(args, "--note");
    let visibility = match flag_value(args, "--visibility") {
        Some(value) => Some(
            PassageVisibility::parse(&value).ok_or("--visibility must be family or admin_only")?,
        ),
        None => None,
    };
    let (_cfg, mut conn) = guarded_client()?;
    narrative::review_passage(
        &mut conn,
        passage_id,
        decision,
        visibility,
        &reviewer,
        note.as_deref(),
    )
    .map_err(|err| err.to_string())?;
    println!(
        "passage {passage_id} -> {}{}",
        decision.as_str(),
        visibility
            .map(|value| format!(" ({})", value.as_str()))
            .unwrap_or_default()
    );
    Ok(())
}

fn narrative_link(args: &[String]) -> Result<(), String> {
    let passage_id: i64 = args
        .first()
        .ok_or("usage: narrative link <passage-id> <person-key> <confirm|reject> --reviewer R")?
        .parse()
        .map_err(|_| "passage id must be an integer".to_string())?;
    let person_key = args.get(1).ok_or("person key is required")?;
    let decision = args.get(2).ok_or("confirm or reject is required")?;
    let accept = match decision.as_str() {
        "confirm" | "confirmed" => true,
        "reject" | "rejected" => false,
        _ => return Err("decision must be confirm or reject".to_string()),
    };
    let reviewer = flag_value(args, "--reviewer").ok_or("--reviewer is required")?;
    let note = flag_value(args, "--note");
    let (_cfg, mut conn) = guarded_client()?;
    narrative::review_link(
        &mut conn,
        passage_id,
        person_key,
        accept,
        &reviewer,
        note.as_deref(),
    )
    .map_err(|err| err.to_string())?;
    println!(
        "passage {passage_id} link {person_key} -> {}",
        if accept { "confirmed" } else { "rejected" }
    );
    Ok(())
}

fn narrative_passages(args: &[String]) -> Result<(), String> {
    let person_key =
        flag_value(args, "--person").ok_or("usage: narrative passages --person <person-key>")?;
    let (_cfg, mut conn) = guarded_client()?;
    let rows = conn
        .query(
            "SELECT p.passage_id, p.page_start, p.page_end, p.language, p.review_status,
                    p.visibility, COALESCE(p.section_title, ''),
                    COALESCE(string_agg(DISTINCT l.link_status || ':' || pe.person_key, ', '), '')
               FROM source_narrative_passage p
               JOIN source_narrative_link l ON l.passage_id = p.passage_id
               JOIN person pe ON pe.person_id = l.person_id
              WHERE pe.person_key = $1
              GROUP BY p.passage_id
              ORDER BY p.page_start, p.passage_id",
            &[&person_key],
        )
        .map_err(|err| err.to_string())?;
    for row in rows {
        let id: i64 = row.get(0);
        let start: i32 = row.get(1);
        let end: i32 = row.get(2);
        let language: String = row.get(3);
        let status: String = row.get(4);
        let visibility: String = row.get(5);
        let title: String = row.get(6);
        let links: String = row.get(7);
        println!(
            "passage {id} p.{start}{} [{language}] {status}/{visibility} {title} links={links}",
            if end > start {
                format!("-{end}")
            } else {
                String::new()
            }
        );
    }
    Ok(())
}
