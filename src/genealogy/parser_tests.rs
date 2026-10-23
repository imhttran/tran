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

const SYNTHETIC_TABLE: &str = "\
| ID | Name | Description | Page |\n| --- | --- | --- | --- |\n| P001 | Nguyễn Văn An | Spouse: Lê Thị Bình [p. 3] | 3 |\n| P002 | Lê Thị Bình | Child of Nguyễn Văn An [p. 4] | 4 |\n";

#[test]
fn table_rows_are_entries_with_names_and_pages() {
    let parse = parse_catalog(SYNTHETIC_TABLE);
    assert_eq!(parse.entries.len(), 2, "one entry per catalog table row");
    assert_eq!(parse.parsed_count(), 2);
    assert_eq!(
        parse.entries[0].display_name.as_deref(),
        Some("Nguyễn Văn An")
    );
    assert_eq!(
        parse.entries[1].display_name.as_deref(),
        Some("Lê Thị Bình")
    );
    assert_ne!(parse.entries[0].key, parse.entries[1].key);
}

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

#[test]
fn expanding_unicode_before_a_page_marker_does_not_panic() {
    // `İ` lowercases to `i` plus a combining dot, which is one byte longer.
    // Reusing that offset to slice the original text used to panic.
    let (pages, _) = extract_pages("İİTrang: 7");
    assert_eq!(pages, vec![7]);
}

#[test]
fn relationship_keywords_only_match_the_field_label() {
    let parse =
        parse_catalog("1. An\nNotes: second marriage of An\nChandler: not a parent\nVợ: Bình\n");
    let relationships: Vec<&CandidateAssertion> = parse.entries[0]
        .assertions
        .iter()
        .filter(|assertion| {
            matches!(
                assertion.kind,
                AssertionKind::ParentChild | AssertionKind::Spouse
            )
        })
        .collect();

    assert_eq!(
        relationships.len(),
        1,
        "only the `Vợ` label is a relationship"
    );
    assert_eq!(relationships[0].kind, AssertionKind::Spouse);
    assert_eq!(relationships[0].value_text, "Bình");
}

const SYNTHETIC_TABLE_RELATIONSHIPS: &str = "\
| ID | Name | Description | Page |
| --- | --- | --- | --- |
| P001 | Nguyễn Văn An | Generation 1; Spouse: Lê Thị Bình [p. 3]; Spouse: Phạm Thị Cúc [p. 3] | 3 |
| P002 | Lê Thị Bình | Spouse of Nguyễn Văn An [p. 4] | 4 |
| P003 | Đỗ Văn Dũng | Child of Nguyễn Văn An and Lê Thị Bình (source family grouping) [p. 5] | 5 |
| P004 | Vũ Thị Em | Child of Nguyễn Văn An and unnamed fourth wife [p. 6] | 6 |
| P005 | Hoàng Văn Giang | Child of Nguyễn Văn An (LAYOUT INFERENCE) [p. 7] | 7 |
| P006 | Bùi Thị Hoa | Spouse-column match to Nguyễn Văn An [p. 8] | 8 |
| P007 | Ngô Văn Khoa | Eighth child of Nguyễn Văn An [p. 9] | 9 |
| P008 | Đinh Thị Lan | Probable child of Nguyễn Văn An and Lê Thị Bình [p. 10] | 10 |
";

fn relationships_of(
    parse: &CatalogParse,
    index: usize,
) -> Vec<(AssertionKind, String, Option<ReasonCode>)> {
    parse.entries[index]
        .assertions
        .iter()
        .filter(|a| matches!(a.kind, AssertionKind::ParentChild | AssertionKind::Spouse))
        .map(|a| (a.kind, a.value_text.clone(), a.reason.clone()))
        .collect()
}

#[test]
fn table_row_yields_one_candidate_per_relationship() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    let spouses = relationships_of(&parse, 0);
    assert_eq!(
        spouses.len(),
        2,
        "two Spouse statements stay two candidates"
    );
    assert_eq!(
        spouses,
        vec![
            (AssertionKind::Spouse, "Lê Thị Bình".to_string(), None),
            (AssertionKind::Spouse, "Phạm Thị Cúc".to_string(), None),
        ]
    );
}

#[test]
fn table_child_of_two_parents_is_two_parent_candidates() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    let parents = relationships_of(&parse, 2);
    // Candidates are sorted by value so the order is deterministic.
    assert_eq!(
        parents,
        vec![
            (AssertionKind::ParentChild, "Lê Thị Bình".to_string(), None),
            (
                AssertionKind::ParentChild,
                "Nguyễn Văn An".to_string(),
                None
            ),
        ]
    );
}

#[test]
fn table_unnamed_parent_is_kept_unresolved() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    let parents = relationships_of(&parse, 3);
    assert_eq!(parents.len(), 2);
    assert!(parents.iter().any(|(kind, value, reason)| {
        *kind == AssertionKind::ParentChild
            && value.starts_with("unnamed fourth wife")
            && *reason == Some(ReasonCode::UnclearParentage)
    }));
}

#[test]
fn table_layout_inference_is_unsourced() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    let parents = relationships_of(&parse, 4);
    assert_eq!(
        parents,
        vec![(
            AssertionKind::ParentChild,
            "Nguyễn Văn An".to_string(),
            Some(ReasonCode::UnsourcedRelationship)
        )]
    );
}

#[test]
fn table_spouse_of_and_spouse_column_forms_are_recognised() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    assert_eq!(
        relationships_of(&parse, 1),
        vec![(AssertionKind::Spouse, "Nguyễn Văn An".to_string(), None)]
    );
    assert_eq!(
        relationships_of(&parse, 5),
        vec![(AssertionKind::Spouse, "Nguyễn Văn An".to_string(), None)]
    );
}

#[test]
fn a_late_birth_order_ordinal_still_states_parentage() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    assert_eq!(
        relationships_of(&parse, 6),
        vec![(
            AssertionKind::ParentChild,
            "Nguyễn Văn An".to_string(),
            None
        )]
    );
}

#[test]
fn a_hedged_parentage_stays_unresolved_for_review() {
    let parse = parse_catalog(SYNTHETIC_TABLE_RELATIONSHIPS);
    let parents = relationships_of(&parse, 7);
    assert_eq!(parents.len(), 2);
    assert!(
        parents
            .iter()
            .all(|(kind, _, reason)| *kind == AssertionKind::ParentChild
                && *reason == Some(ReasonCode::UnsourcedRelationship)),
        "a hedged statement is kept but marked unsourced: {parents:?}"
    );
}
