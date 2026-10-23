//! Deterministic resolution of parsed relationship candidates to stable keys.
//!
//! A relationship becomes a graph edge only when its target names exactly one
//! person in the same catalog edition. Nothing is guessed: a target that matches
//! no person, or more than one (namesakes), stays unresolved and is reported so
//! a reviewer can see it. Matching is case-insensitive and whitespace-trimmed
//! only, so two source spellings that differ by a diacritic are not conflated.

use std::collections::BTreeMap;

/// One catalog person and every name the catalog may use for it.
#[derive(Debug, Clone)]
pub struct PersonName {
    pub person_key: String,
    pub display_name: String,
    pub aliases: Vec<String>,
}

/// One relationship statement parsed from a person's catalog row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipCandidate {
    /// The person whose row states the relationship.
    pub owner_key: String,
    /// `parent_child` or `spouse`.
    pub kind: String,
    /// The target name exactly as the catalog wrote it.
    pub target: String,
    /// `false` when the source hedges the statement or marks it as an
    /// inference. Such a statement stays an assertion for review and never
    /// becomes an edge, even when its target is unambiguous.
    pub resolvable: bool,
}

/// A resolved edge, ready to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedLink {
    pub owner_key: String,
    pub kind: String,
    pub target: String,
    /// `parent_child` links are parent → child; `spouse` links are ordered by key.
    pub from_key: String,
    pub to_key: String,
}

/// A relationship that could not be resolved, with a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedRelationship {
    pub owner_key: String,
    pub kind: String,
    pub target: String,
    pub reason: String,
}

/// How a target name resolves within one catalog edition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    Unique(String),
    Ambiguous(Vec<String>),
    Unknown,
}

/// A normalized-name index over one catalog edition.
#[derive(Debug, Default)]
pub struct NameIndex {
    by_name: BTreeMap<String, Vec<String>>,
}

fn normalize(name: &str) -> String {
    name.trim().to_lowercase()
}

impl NameIndex {
    pub fn new(people: &[PersonName]) -> Self {
        let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for person in people {
            let mut names = vec![person.display_name.clone()];
            names.extend(person.aliases.iter().cloned());
            for name in names {
                let key = normalize(&name);
                if key.is_empty() {
                    continue;
                }
                let entry = by_name.entry(key).or_default();
                if !entry.contains(&person.person_key) {
                    entry.push(person.person_key.clone());
                }
            }
        }
        for keys in by_name.values_mut() {
            keys.sort();
        }
        Self { by_name }
    }

    pub fn resolve(&self, name: &str) -> Resolution {
        match self.by_name.get(&normalize(name)) {
            Some(keys) if keys.len() == 1 => Resolution::Unique(keys[0].clone()),
            Some(keys) => Resolution::Ambiguous(keys.clone()),
            None => Resolution::Unknown,
        }
    }
}

/// The person key each candidate resolves to, parallel to `candidates`.
///
/// A `None` means the statement stays unresolved: an unknown name, a shared
/// name (namesakes), or a statement that names its own owner. Every endpoint
/// that stated the same relationship gets the same binding, so a verified
/// assertion on either side can support the edge.
pub fn object_bindings(
    candidates: &[RelationshipCandidate],
    index: &NameIndex,
) -> Vec<Option<String>> {
    candidates
        .iter()
        .map(|candidate| match index.resolve(&candidate.target) {
            Resolution::Unique(key) if candidate.resolvable && key != candidate.owner_key => {
                Some(key)
            }
            _ => None,
        })
        .collect()
}

/// Plans one edge per resolvable candidate and reports the rest.
///
/// Duplicate statements for the same pair collapse to one edge, so re-running an
/// import cannot create duplicates even before the database's unique keys apply.
/// A statement that resolves to the owner itself is dropped: the schema forbids
/// self-links and a self-relationship is a source error, not an edge.
pub fn plan_links(
    candidates: &[RelationshipCandidate],
    index: &NameIndex,
) -> (Vec<PlannedLink>, Vec<UnresolvedRelationship>) {
    let mut links: Vec<PlannedLink> = Vec::new();
    let mut unresolved = Vec::new();

    for candidate in candidates {
        if !candidate.resolvable {
            unresolved.push(UnresolvedRelationship {
                owner_key: candidate.owner_key.clone(),
                kind: candidate.kind.clone(),
                target: candidate.target.clone(),
                reason: "the source marks this relationship as inferred or unresolved".to_string(),
            });
            continue;
        }
        match index.resolve(&candidate.target) {
            Resolution::Unique(target_key) => {
                if target_key == candidate.owner_key {
                    unresolved.push(UnresolvedRelationship {
                        owner_key: candidate.owner_key.clone(),
                        kind: candidate.kind.clone(),
                        target: candidate.target.clone(),
                        reason: "the statement names the person itself".to_string(),
                    });
                    continue;
                }
                let (from_key, to_key) = if candidate.kind == "parent_child" {
                    (target_key, candidate.owner_key.clone())
                } else if candidate.owner_key <= target_key {
                    (candidate.owner_key.clone(), target_key)
                } else {
                    (target_key, candidate.owner_key.clone())
                };
                let link = PlannedLink {
                    owner_key: candidate.owner_key.clone(),
                    kind: candidate.kind.clone(),
                    target: candidate.target.clone(),
                    from_key,
                    to_key,
                };
                if !links.iter().any(|existing| {
                    existing.kind == link.kind
                        && existing.from_key == link.from_key
                        && existing.to_key == link.to_key
                }) {
                    links.push(link);
                }
            }
            Resolution::Ambiguous(keys) => unresolved.push(UnresolvedRelationship {
                owner_key: candidate.owner_key.clone(),
                kind: candidate.kind.clone(),
                target: candidate.target.clone(),
                reason: format!(
                    "{} catalog entries share this name; kept separate for review",
                    keys.len()
                ),
            }),
            Resolution::Unknown => unresolved.push(UnresolvedRelationship {
                owner_key: candidate.owner_key.clone(),
                kind: candidate.kind.clone(),
                target: candidate.target.clone(),
                reason: "no catalog entry matches this name".to_string(),
            }),
        }
    }

    (links, unresolved)
}

#[cfg(test)]
#[path = "relationships_tests.rs"]
mod tests;
