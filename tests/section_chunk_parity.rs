//! RUST-003 fixture parity: compare the Rust section/chunk output against the
//! pinned Go reference.
//!
//! The fixtures are frozen from the pinned Go reference (the generator was
//! removed in RUST-007C) and record the reference's `ParseSections` /
//! `loader.Dispatch` / `FromSections` output verbatim. This test never edits an
//! expected value; a difference is a Rust implementation gap to repair, not a
//! fixture change.

use std::path::PathBuf;

use rag_foundation::chunker::chunk_sections;
use rag_foundation::document::Section;
use rag_foundation::loader::{load_pdf, load_text};
use rag_foundation::section::parse_sections;
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    provenance: std::collections::BTreeMap<String, String>,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    name: String,
    kind: String,
    #[serde(default)]
    input: String,
    #[serde(default)]
    input_file: String,
    source: String,
    language: String,
    chunk_size: usize,
    overlap: usize,
    sections: Vec<Sec>,
    chunks: Vec<Chk>,
    #[serde(default)]
    load_error: String,
    #[serde(default)]
    incomplete_pages: Vec<u32>,
}

#[derive(Deserialize, PartialEq, Eq, Debug, Clone)]
struct Sec {
    title: String,
    content: String,
    page: i64,
}

#[derive(Deserialize, PartialEq, Eq, Debug, Clone)]
struct Chk {
    section: String,
    content: String,
    index: i64,
    page: i64,
}

/// One case's Rust load result.
struct Loaded {
    sections: Vec<Section>,
    err: Option<String>,
    incomplete: Vec<u32>,
}

fn fixture() -> Fixture {
    let raw = std::fs::read_to_string("fixtures/parity/section_chunk_parity.json").expect(
        "read fixtures/parity/section_chunk_parity.json (frozen from the pinned Go reference)",
    );
    serde_json::from_str(&raw).expect("parse parity fixture")
}

fn to_sec(s: &Section) -> Sec {
    Sec {
        title: s.title.clone(),
        content: s.content.clone(),
        page: s.page.map(i64::from).unwrap_or(0),
    }
}

fn inputs() -> PathBuf {
    PathBuf::from("fixtures/parity/inputs")
}

fn load_case(c: &Case) -> Loaded {
    match c.kind.as_str() {
        "markdown" => Loaded {
            sections: parse_sections(&c.input),
            err: None,
            incomplete: Vec::new(),
        },
        "text" => match load_text(inputs().join(&c.input_file)) {
            Ok(sections) => Loaded {
                sections,
                err: None,
                incomplete: Vec::new(),
            },
            Err(e) => Loaded {
                sections: Vec::new(),
                err: Some(e.to_string()),
                incomplete: Vec::new(),
            },
        },
        "pdf" => match load_pdf(inputs().join(&c.input_file)) {
            Ok(x) => Loaded {
                sections: x.sections,
                err: None,
                incomplete: x.incomplete_pages,
            },
            Err(e) => Loaded {
                sections: Vec::new(),
                err: Some(e.to_string()),
                incomplete: Vec::new(),
            },
        },
        other => panic!("unknown case kind {other:?}"),
    }
}

/// The Go reference's outcome category for a case, derived from the fixture.
#[derive(PartialEq, Eq, Debug)]
enum Status {
    /// No error, every page had text.
    Complete,
    /// Readable sections returned alongside an incomplete-extraction error.
    Incomplete,
    /// A hard error with no sections.
    HardError,
}

fn go_status(c: &Case) -> Status {
    if c.load_error.is_empty() {
        Status::Complete
    } else if c.sections.is_empty() {
        Status::HardError
    } else {
        Status::Incomplete
    }
}

fn rust_status(l: &Loaded) -> Status {
    if l.err.is_some() {
        Status::HardError
    } else if !l.incomplete.is_empty() {
        Status::Incomplete
    } else {
        Status::Complete
    }
}

#[test]
fn provenance_is_recorded() {
    let fx = fixture();
    assert_eq!(
        fx.provenance.get("module").map(String::as_str),
        Some("rag-template")
    );
    assert!(!fx
        .provenance
        .get("commit")
        .map(String::as_str)
        .unwrap_or("")
        .is_empty());
    assert!(!fx.cases.is_empty());
}

#[test]
fn sections_match_go_reference() {
    let fx = fixture();
    let mut failures = Vec::new();

    for c in &fx.cases {
        let loaded = load_case(c);
        let got: Vec<Sec> = loaded.sections.iter().map(to_sec).collect();
        if got != c.sections {
            failures.push(format!(
                "case {} ({}): sections differ\n  want: {:?}\n  got:  {:?}",
                c.name, c.kind, c.sections, got
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "section parity failures ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn chunks_match_go_reference() {
    let fx = fixture();
    let mut failures = Vec::new();

    for c in &fx.cases {
        let loaded = load_case(c);
        let got: Vec<Chk> = chunk_sections(
            &loaded.sections,
            &c.source,
            &c.language,
            c.chunk_size,
            c.overlap,
        )
        .into_iter()
        .map(|ch| Chk {
            section: ch.section,
            content: ch.content,
            index: i64::from(ch.chunk_index),
            page: ch.page.map(i64::from).unwrap_or(0),
        })
        .collect();
        if got != c.chunks {
            failures.push(format!(
                "case {} ({}): chunks differ\n  want: {:?}\n  got:  {:?}",
                c.name, c.kind, c.chunks, got
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "chunk parity failures ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The loader status and the explicit incomplete-page list must match the
/// reference: a complete extraction, a partial extraction that names its OCR
/// pages, or a hard error.
#[test]
fn pdf_extraction_status_matches_reference() {
    let fx = fixture();
    let mut failures = Vec::new();

    for c in &fx.cases {
        let loaded = load_case(c);
        if go_status(c) != rust_status(&loaded) {
            failures.push(format!(
                "case {}: status go={:?} rust={:?} (go load_error={:?}, rust err={:?})",
                c.name,
                go_status(c),
                rust_status(&loaded),
                c.load_error,
                loaded.err
            ));
        }
        if loaded.incomplete != c.incomplete_pages {
            failures.push(format!(
                "case {}: incomplete_pages go={:?} rust={:?}",
                c.name, c.incomplete_pages, loaded.incomplete
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "pdf extraction status failures ({}):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// Source/language are inputs the Rust chunker propagates; the Go reference
/// attaches them at ingestion, so this checks propagation rather than a
/// reference-derived value.
#[test]
fn chunk_source_and_language_are_propagated() {
    let fx = fixture();
    for c in &fx.cases {
        let loaded = load_case(c);
        for ch in chunk_sections(
            &loaded.sections,
            &c.source,
            &c.language,
            c.chunk_size,
            c.overlap,
        ) {
            assert_eq!(ch.source, c.source, "case {}", c.name);
            assert_eq!(ch.language, c.language, "case {}", c.name);
        }
    }
}
