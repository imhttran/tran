//! Catalog parser tests (TRAN-002C).
//!
//! Every fixture here is synthetic and contains no real family name, date, or
//! page text.

use super::*;

const SYNTHETIC_UNIFORM: &str = "\
1. Nguyễn Văn An\n   Trang: 3\n2. Lê Thị Bình\n   Alias: Bình Lê\n   Nhánh: Trưởng\n   Trang: 12\n";

const SYNTHETIC_MIXED: &str = "\
# Nguyễn Văn An\nTrang: 3\n\n## Lê Thị Bình\nAlias: Bình Lê\nNhánh: Trưởng\nPage: 12\n\n# Lê Thị Bình\nPage: 20\n";

const SYNTHETIC_BAD_PAGES: &str = "\
1. Trần Văn Cường\nTrang: 0\n2. Phạm Văn Dũng\nTrang: 57\n3. Hoàng Thị Em\nTrang: 56\n";

const SYNTHETIC_NO_PAGE: &str = "\
1. Vũ Văn Phong\nNhánh: Hai\n";

const SYNTHETIC_UNSOURCED_REL: &str = "\
1. Đỗ Văn Giang\nVợ: cùng tên với chị gái\nTrang: 5\n";

#[test]
fn every_entry_maps_to_exactly_one_outcome() {
    for text in [
        SYNTHETIC_UNIFORM,
        SYNTHETIC_MIXED,
        SYNTHETIC_BAD_PAGES,
        SYNTHETIC_NO_PAGE,
        SYNTHETIC_UNSOURCED_REL,
    ] {
        let parse = parse_catalog(text);
        let classified = parse.parsed_count() + parse.ambiguous_count() + parse.rejected_count();
        assert_eq!(
            classified,
            parse.entries.len(),
            "each entry must be classified exactly once"
        );
    }
}

#[test]
fn two_parses_are_deterministic() {
    for text in [SYNTHETIC_UNIFORM, SYNTHETIC_MIXED, SYNTHETIC_BAD_PAGES] {
        let first = parse_catalog(text);
        let second = parse_catalog(text);
        assert_eq!(first.keys(), second.keys(), "keys must be stable");
        assert_eq!(first, second, "assertions must be identical and ordered");
    }
}

#[test]
fn no_assertion_is_verified() {
    let parse = parse_catalog(SYNTHETIC_UNIFORM);
    for entry in &parse.entries {
        for assertion in &entry.assertions {
            assert_eq!(assertion.review_status, ReviewStatus::Candidate);
        }
    }
}

#[test]
fn same_name_produces_distinct_candidates() {
    let parse = parse_catalog(SYNTHETIC_MIXED);
    let binh: Vec<_> = parse
        .entries
        .iter()
        .filter(|e| e.display_name.as_deref() == Some("Lê Thị Bình"))
        .collect();
    assert_eq!(binh.len(), 2, "both same-name entries are kept");
    assert_ne!(binh[0].key, binh[1].key, "keys must be distinct");
    assert!(binh.iter().all(|e| e.outcome == ParseOutcome::Ambiguous));
    assert!(binh.iter().all(|e| e.reason == Some(ReasonCode::SameName)));
}

#[test]
fn name_variants_preserve_vietnamese_spelling() {
    let parse = parse_catalog(SYNTHETIC_UNIFORM);
    let variants: Vec<_> = parse
        .entries
        .iter()
        .flat_map(|e| e.assertions.iter())
        .filter(|a| a.kind == AssertionKind::NameVariant)
        .map(|a| a.value_text.clone())
        .collect();
    assert!(variants.contains(&"Bình Lê".to_string()));
}

#[test]
fn page_zero_and_fifty_seven_are_rejected_and_fifty_six_accepted() {
    let parse = parse_catalog(SYNTHETIC_BAD_PAGES);
    assert_eq!(parse.entries[0].outcome, ParseOutcome::Rejected);
    assert_eq!(parse.entries[0].reason, Some(ReasonCode::PageOutOfRange));
    assert_eq!(parse.entries[1].outcome, ParseOutcome::Rejected);
    assert_eq!(parse.entries[2].outcome, ParseOutcome::Parsed);
}

#[test]
fn missing_page_is_an_explicit_unresolved_locator() {
    let parse = parse_catalog(SYNTHETIC_NO_PAGE);
    let entry = &parse.entries[0];
    assert_eq!(entry.outcome, ParseOutcome::Parsed);
    let has_unresolved = entry.assertions.iter().any(|a| {
        a.evidence
            .iter()
            .any(|ev| matches!(ev, Evidence::Unresolved(_)))
    });
    assert!(has_unresolved, "no page must produce an unresolved locator");
}

#[test]
fn unsourced_relationship_is_candidate_with_reason() {
    let parse = parse_catalog(SYNTHETIC_UNSOURCED_REL);
    let rel = parse.entries[0]
        .assertions
        .iter()
        .find(|a| a.kind == AssertionKind::Spouse)
        .expect("spouse assertion exists");
    assert_eq!(rel.review_status, ReviewStatus::Candidate);
    assert_eq!(rel.reason, Some(ReasonCode::UnsourcedRelationship));
}

#[test]
fn dry_run_writes_report_to_temp_path() {
    let dir = tempfile::tempdir().expect("temp dir");
    let input = dir.path().join("catalog.md");
    let output = dir.path().join("out/report.md");
    std::fs::write(&input, SYNTHETIC_MIXED).expect("write catalog");
    let report = run_dry_run(&input, &output).expect("dry run");
    assert_eq!(
        report.parsed + report.ambiguous + report.rejected,
        report.processed
    );
    assert!(output.exists(), "report file must be written");
}
