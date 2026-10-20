//! Read-only genealogy graph projection for the authorized family-tree API.
//!
//! Physical links are *candidates*, not proof. A displayed verified edge must
//! have a matching reviewed relationship assertion; conflicting assertions
//! keep the edge in needs_review. All reads use the separate genealogy DB.

use postgres::Client;
use super::{query::Citation, GenealogyError};

/// Bound the first tree response without silently pretending it is complete.
pub const MAX_TREE_PEOPLE: i64 = 500;

/// One cataloged person, with no inferred personal attributes.
#[derive(Debug, Clone)]
pub struct TreePerson {
    pub person_key: String,
    pub display_name: String,
    pub evidence_status: String,
    pub aliases: Vec<String>,
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
                (SELECT COALESCE(ARRAY_AGG(v.name_variant ORDER BY v.name_variant), ARRAY[]::text[])
                   FROM person_name_variant v WHERE v.person_id = p.person_id) AS aliases
           FROM person p
           LEFT JOIN assertion a ON a.person_id = p.person_id
          GROUP BY p.person_id
          ORDER BY p.person_key
          LIMIT $1",
        &[&(MAX_TREE_PEOPLE + 1)],
    )?;
    let truncated = rows.len() as i64 > MAX_TREE_PEOPLE;
    let people: Vec<TreePerson> = rows
        .iter()
        .take(MAX_TREE_PEOPLE as usize)
        .map(|row| {
            Ok(TreePerson {
                person_key: row.try_get(0)?,
                display_name: row.try_get(1)?,
                evidence_status: row.try_get(2)?,
                aliases: row.try_get(3)?,
            })
        })
        .collect::<Result<Vec<_>, postgres::Error>>()?;

    if people.is_empty() {
        return Ok(TreeGraph { people, links: Vec::new(), truncated });
    }

    let keys: Vec<String> = people.iter().map(|p| p.person_key.clone()).collect();
    // The link and assertion must match the same person pair and relationship
    // kind. Source metadata is returned only for verified, non-conflicting edges.
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
                    WHERE a.person_id = l.from_id AND a.assertion_kind = l.kind
                      AND (a.object_person_id = l.to_id OR a.value_text = l.to_key)
                      AND a.review_status IN ('needs_review', 'disputed')
                 ) THEN 'needs_review'
                 WHEN EXISTS (
                   SELECT 1 FROM assertion a
                    WHERE a.person_id = l.from_id AND a.assertion_kind = l.kind
                      AND (a.object_person_id = l.to_id OR a.value_text = l.to_key)
                      AND a.review_status = 'verified'
                 ) THEN 'verified'
                 ELSE 'unverified'
               END AS evidence_status,
               evidence.document_ref, evidence.page
          FROM links l
          LEFT JOIN LATERAL (
            SELECT DISTINCT ae.document_ref, ae.page
              FROM assertion a
              JOIN assertion_evidence ae ON ae.assertion_id = a.assertion_id
             WHERE a.person_id = l.from_id AND a.assertion_kind = l.kind
               AND (a.object_person_id = l.to_id OR a.value_text = l.to_key)
               AND a.review_status = 'verified'
          ) evidence ON true
         ORDER BY l.kind, l.from_key, l.to_key, evidence.document_ref, evidence.page",
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
        let citation = match (document_ref, page) {
            (Some(document_ref), Some(page)) if status == "verified" =>
                Some(Citation { document_ref, page }),
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
    Ok(TreeGraph { people, links, truncated })
}
