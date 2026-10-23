//! Relationship resolution tests.
//!
//! Synthetic names only: the namesakes below are fixtures that reproduce the
//! ambiguity shape, not real family records.

use super::*;

fn person(key: &str, name: &str, aliases: &[&str]) -> PersonName {
    PersonName {
        person_key: key.to_string(),
        display_name: name.to_string(),
        aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
    }
}

fn index() -> NameIndex {
    NameIndex::new(&[
        person("P001", "An", &[]),
        person("P002", "Bình", &["Binh"]),
        // Two entries with the same display name stay two people.
        person("P003", "Huyền", &[]),
        person("P004", "Huyền", &[]),
    ])
}

fn candidate(owner: &str, kind: &str, target: &str) -> RelationshipCandidate {
    RelationshipCandidate {
        owner_key: owner.to_string(),
        kind: kind.to_string(),
        target: target.to_string(),
        resolvable: true,
    }
}

#[test]
fn a_unique_name_resolves_to_one_key() {
    assert_eq!(index().resolve("An"), Resolution::Unique("P001".into()));
    assert_eq!(index().resolve("  an "), Resolution::Unique("P001".into()));
    assert_eq!(index().resolve("Binh"), Resolution::Unique("P002".into()));
}

#[test]
fn namesakes_are_ambiguous_and_never_merged() {
    assert_eq!(
        index().resolve("Huyền"),
        Resolution::Ambiguous(vec!["P003".into(), "P004".into()])
    );
}

#[test]
fn an_unknown_name_is_unresolved() {
    assert_eq!(index().resolve("Nobody"), Resolution::Unknown);
}

#[test]
fn parent_child_edges_point_from_parent_to_child() {
    let (links, unresolved) = plan_links(&[candidate("P002", "parent_child", "An")], &index());
    assert!(unresolved.is_empty());
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].from_key, "P001");
    assert_eq!(links[0].to_key, "P002");
}

#[test]
fn spouse_edges_are_canonical_regardless_of_statement_order() {
    let (forward, _) = plan_links(&[candidate("P002", "spouse", "An")], &index());
    let (reverse, _) = plan_links(&[candidate("P001", "spouse", "Bình")], &index());
    assert_eq!(forward.len(), 1);
    assert_eq!(reverse.len(), 1);
    assert_eq!(forward[0].from_key, reverse[0].from_key);
    assert_eq!(forward[0].to_key, reverse[0].to_key);
}

#[test]
fn ambiguous_targets_stay_unresolved_with_a_reason() {
    let (links, unresolved) = plan_links(&[candidate("P001", "spouse", "Huyền")], &index());
    assert!(
        links.is_empty(),
        "an ambiguous target must not create an edge"
    );
    assert_eq!(unresolved.len(), 1);
    assert!(unresolved[0].reason.contains("share this name"));
}

#[test]
fn duplicate_statements_collapse_to_one_edge() {
    let (links, _) = plan_links(
        &[
            candidate("P002", "parent_child", "An"),
            candidate("P002", "parent_child", "An"),
        ],
        &index(),
    );
    assert_eq!(links.len(), 1);
}

#[test]
fn a_self_relationship_is_reported_not_linked() {
    let (links, unresolved) = plan_links(&[candidate("P001", "spouse", "An")], &index());
    assert!(links.is_empty());
    assert_eq!(unresolved.len(), 1);
    assert!(unresolved[0].reason.contains("itself"));
}

#[test]
fn both_endpoints_bind_to_the_same_counterpart() {
    // The child's row and the parent's row state the same relationship. Both
    // assertions keep an object binding, so either one can support the edge.
    let candidates = [
        candidate("P002", "parent_child", "An"),
        candidate("P001", "parent_child", "Bình"),
    ];
    assert_eq!(
        object_bindings(&candidates, &index()),
        vec![Some("P001".to_string()), Some("P002".to_string())]
    );
}

#[test]
fn ambiguous_unknown_and_self_candidates_have_no_binding() {
    let candidates = [
        candidate("P001", "spouse", "Huyền"),
        candidate("P001", "spouse", "Nobody"),
        candidate("P001", "spouse", "An"),
    ];
    assert_eq!(
        object_bindings(&candidates, &index()),
        vec![None, None, None]
    );
}

#[test]
fn an_inferred_statement_never_becomes_an_edge() {
    // Layout inference names an unambiguous person, so only the candidate's own
    // `resolvable` flag keeps it out of the graph.
    let inferred = RelationshipCandidate {
        resolvable: false,
        ..candidate("P002", "parent_child", "An")
    };
    let (links, unresolved) = plan_links(std::slice::from_ref(&inferred), &index());
    assert!(
        links.is_empty(),
        "an inferred statement must not create an edge"
    );
    assert_eq!(unresolved.len(), 1);
    assert!(unresolved[0].reason.contains("inferred"));
    assert_eq!(object_bindings(&[inferred], &index()), vec![None]);
}
