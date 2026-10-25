//! Read-only genealogy graph projection for the authorized family-tree API.
//!
//! Physical links are *candidates*, not proof. A displayed verified edge must
//! have a matching reviewed relationship assertion; conflicting assertions
//! keep the edge in needs_review. An assertion may be recorded from either
//! endpoint of the pair (a child naming its parent, or a parent naming its
//! child), and the match accepts both. All reads use the separate genealogy DB.

use super::dates::{self, DatePrecision, DateQualifier};
use super::{query::Citation, GenealogyError};
use postgres::Client;
use std::collections::BTreeMap;

/// Bound the first tree response without silently pretending it is complete.
pub const MAX_TREE_PEOPLE: i64 = 500;

/// One verified date on a tree person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeDate {
    /// Partial ISO start value.
    pub start: String,
    /// Partial ISO end value, for a range.
    pub end: Option<String>,
    /// The precision the source supports.
    pub precision: DatePrecision,
    /// The source's certainty.
    pub qualifier: DateQualifier,
}

/// Every verified birth and death claim per person key, in stable order.
type LifespanByPerson = BTreeMap<String, (Vec<TreeDate>, Vec<TreeDate>)>;

/// One cataloged person, with no inferred personal attributes.
///
/// `birth` and `death` come only from verified assertions, so an unconfirmed
/// date never appears as a lifespan. A missing death date never implies that a
/// person is living.
#[derive(Debug, Clone)]
pub struct TreePerson {
    pub person_key: String,
    pub display_name: String,
    pub evidence_status: String,
    pub aliases: Vec<String>,
    pub birth: Option<TreeDate>,
    pub death: Option<TreeDate>,
}

/// One stored link, classified using a matching review assertion.
#[derive(Debug, Clone)]
pub struct TreeLink {
    pub from_key: String,
    pub to_key: String,
    pub kind: String,
    pub evidence_status: String,
    pub citations: Vec<Citation>,
}

/// A bounded projection. When truncated, the UI must say so.
#[derive(Debug, Clone)]
pub struct TreeGraph {
    pub people: Vec<TreePerson>,
    pub links: Vec<TreeLink>,
    pub truncated: bool,
}

/// Lists graph nodes and links without inventing relatives or applying LLM output.
///
/// Only linked people within the bounded set appear in edge results. A stored
/// link alone never produces a verified relationship. No genealogy DB fallback
/// or RAG query is involved; callers must authorize before opening this DB.
pub fn load_tree(client: &mut Client) -> Result<TreeGraph, GenealogyError> {
    let rows = client.query(
        "SELECT p.person_key,
                COALESCE(NULLIF(BTRIM(p.display_name), ''), 'Unnamed person'),
                CASE
                  WHEN COALESCE(BOOL_OR(a.review_status IN ('needs_review', 'disputed')), false)
                    THEN 'needs_review'
                  WHEN COALESCE(BOOL_OR(a.review_status = 'verified'), false)
                    THEN 'verified'
                  ELSE 'unverified'
                END AS evidence_status,
                (SELECT COALESCE(ARRAY_AGG(DISTINCT name ORDER BY name), ARRAY[]::text[])
                   FROM (
                     SELECT a.value_text AS name
                       FROM assertion a
                      WHERE a.person_id = p.person_id
                        AND a.assertion_kind = 'name_variant'
                        AND a.value_text IS NOT NULL
                     UNION
                     SELECT v.name_variant
                       FROM person_name_variant v
                      WHERE v.person_id = p.person_id
                   ) recorded_names) AS aliases
           FROM person p
           LEFT JOIN assertion a ON a.person_id = p.person_id
          GROUP BY p.person_id
          ORDER BY p.person_key
          LIMIT $1",
        &[&(MAX_TREE_PEOPLE + 1)],
    )?;
    let truncated = rows.len() as i64 > MAX_TREE_PEOPLE;
    let mut people: Vec<TreePerson> = rows
        .iter()
        .take(MAX_TREE_PEOPLE as usize)
        .map(|row| {
            Ok(TreePerson {
                person_key: row.try_get(0)?,
                display_name: row.try_get(1)?,
                evidence_status: row.try_get(2)?,
                aliases: row.try_get(3)?,
                birth: None,
                death: None,
            })
        })
        .collect::<Result<Vec<_>, postgres::Error>>()?;

    if people.is_empty() {
        return Ok(TreeGraph {
            people,
            links: Vec::new(),
            truncated,
        });
    }

    let keys: Vec<String> = people.iter().map(|p| p.person_key.clone()).collect();

    // Lifespans are a verified-only projection. An unverified or disputed date
    // is deliberately absent from the card, so a candidate never reads as fact.
    let mut lifespans = load_verified_dates(client, &keys)?;
    for person in &mut people {
        if let Some((births, deaths)) = lifespans.remove(&person.person_key) {
            person.birth = unambiguous(births);
            person.death = unambiguous(deaths);
        }
    }
    // The link and assertion must match the same person pair and relationship
    // kind. A statement may be recorded from either endpoint (a child naming a
    // parent, or a parent naming a child; either spouse naming the other), so
    // both orientations count. Source metadata is returned only for verified,
    // non-conflicting edges.
    let rows = client.query(
        "WITH links AS (
            SELECT 'parent_child'::text AS kind,
                   parent.person_key AS from_key, child.person_key AS to_key,
                   pcl.parent_id AS from_id, pcl.child_id AS to_id
              FROM parent_child_link pcl
              JOIN person parent ON parent.person_id = pcl.parent_id
              JOIN person child ON child.person_id = pcl.child_id
             WHERE parent.person_key = ANY($1::text[])
               AND child.person_key = ANY($1::text[])
            UNION
            SELECT 'spouse'::text AS kind,
                   p.person_key AS from_key, s.person_key AS to_key,
                   sl.person_id AS from_id, sl.spouse_id AS to_id
              FROM spouse_link sl
              JOIN person p ON p.person_id = sl.person_id
              JOIN person s ON s.person_id = sl.spouse_id
             WHERE p.person_key = ANY($1::text[])
               AND s.person_key = ANY($1::text[])
        )
        SELECT l.kind, l.from_key, l.to_key,
               CASE
                 WHEN EXISTS (
                   SELECT 1 FROM assertion a
                    WHERE a.assertion_kind = l.kind
                      AND (
                        (a.person_id = l.from_id
                         AND (a.object_person_id = l.to_id OR a.value_text = l.to_key))
                        OR
                        (a.person_id = l.to_id
                         AND (a.object_person_id = l.from_id OR a.value_text = l.from_key))
                      )
                      AND a.review_status IN ('needs_review', 'disputed')
                 ) THEN 'needs_review'
                 WHEN EXISTS (
                   SELECT 1 FROM assertion a
                    WHERE a.assertion_kind = l.kind
                      AND (
                        (a.person_id = l.from_id
                         AND (a.object_person_id = l.to_id OR a.value_text = l.to_key))
                        OR
                        (a.person_id = l.to_id
                         AND (a.object_person_id = l.from_id OR a.value_text = l.from_key))
                      )
                      AND a.review_status = 'verified'
                 ) THEN 'verified'
                 ELSE 'unverified'
               END AS evidence_status,
               evidence.document_ref, evidence.page, evidence.locator, evidence.excerpt
          FROM links l
          LEFT JOIN LATERAL (
            SELECT DISTINCT ae.document_ref, ae.page, ae.locator, ae.excerpt
              FROM assertion a
              JOIN assertion_evidence ae
                ON ae.assertion_id = a.assertion_id AND ae.withdrawn_at IS NULL
             WHERE a.assertion_kind = l.kind
               AND (
                 (a.person_id = l.from_id
                  AND (a.object_person_id = l.to_id OR a.value_text = l.to_key))
                 OR
                 (a.person_id = l.to_id
                  AND (a.object_person_id = l.from_id OR a.value_text = l.from_key))
               )
               AND a.review_status = 'verified'
          ) evidence ON true
         ORDER BY l.kind, l.from_key, l.to_key, evidence.document_ref, evidence.page,
                  evidence.locator",
        &[&keys],
    )?;

    let mut links: Vec<TreeLink> = Vec::new();
    for row in rows {
        let kind: String = row.try_get(0)?;
        let from_key: String = row.try_get(1)?;
        let to_key: String = row.try_get(2)?;
        let status: String = row.try_get(3)?;
        let document_ref: Option<String> = row.try_get(4)?;
        let page: Option<i32> = row.try_get(5)?;
        let locator: Option<String> = row.try_get(6)?;
        let excerpt: Option<String> = row.try_get(7)?;
        let citation = match (document_ref, page) {
            (Some(document_ref), Some(page)) if status == "verified" => Some(Citation {
                document_ref,
                page,
                locator,
                excerpt,
            }),
            _ => None,
        };
        if let Some(last) = links.last_mut() {
            if last.kind == kind && last.from_key == from_key && last.to_key == to_key {
                if let Some(citation) = citation {
                    last.citations.push(citation);
                }
                continue;
            }
        }
        links.push(TreeLink {
            from_key,
            to_key,
            kind,
            evidence_status: status,
            citations: citation.into_iter().collect(),
        });
    }
    Ok(TreeGraph {
        people,
        links,
        truncated,
    })
}

/// Loads each person's verified birth and death date.
///
/// Only `verified` assertions are read, and only their first (deterministically
/// ordered) value per kind. A date still under review is not projected onto the
/// card, and a missing death date is never treated as evidence of living status.
fn load_verified_dates(
    client: &mut Client,
    keys: &[String],
) -> Result<LifespanByPerson, GenealogyError> {
    let rows = client.query(
        "SELECT p.person_key, a.assertion_kind, a.value_date_start, a.value_date_end,
                a.value_date_precision, a.value_date_qualifier
           FROM assertion a
           JOIN person p ON p.person_id = a.person_id
          WHERE p.person_key = ANY($1)
            AND a.assertion_kind IN ('birth_date', 'death_date')
            AND a.review_status = 'verified'
            AND a.value_date_start IS NOT NULL
          ORDER BY p.person_key, a.assertion_kind, a.value_date_start, a.assertion_id",
        &[&keys.to_vec()],
    )?;

    let mut out: LifespanByPerson = BTreeMap::new();
    for row in rows {
        let person_key: String = row.try_get(0)?;
        let kind: String = row.try_get(1)?;
        let start: String = row.try_get(2)?;
        let end: Option<String> = row.try_get(3)?;
        let precision: Option<String> = row.try_get(4)?;
        let qualifier: Option<String> = row.try_get(5)?;
        let (Some(precision), Some(qualifier)) = (
            precision.as_deref().and_then(DatePrecision::parse),
            qualifier.as_deref().and_then(DateQualifier::parse),
        ) else {
            continue;
        };
        let entry = out.entry(person_key).or_default();
        let slot = if kind == "birth_date" {
            &mut entry.0
        } else {
            &mut entry.1
        };
        let date = TreeDate {
            start,
            end,
            precision,
            qualifier,
        };
        if !slot.contains(&date) {
            slot.push(date);
        }
    }
    Ok(out)
}

/// One date, or nothing when the verified claims genuinely disagree.
///
/// Overlapping claims are refinements of one another (`1877` and `1877-03`), so
/// the more precise one is shown. Disjoint claims are an unsettled disagreement,
/// so the card omits the date and the person detail keeps every claim with its
/// review status.
fn unambiguous(dates: Vec<TreeDate>) -> Option<TreeDate> {
    let claims: Vec<(String, Option<String>, DatePrecision)> = dates
        .iter()
        .map(|date| (date.start.clone(), date.end.clone(), date.precision))
        .collect();
    dates::representative_claim(&claims).map(|index| dates[index].clone())
}
