//! Evidence-backed read queries for the genealogy CLI (TRAN-002E).
//!
//! Every function is read-only. It returns the stored assertion status and
//! review status verbatim plus the document_ref/page citations. It never
//! synthesizes a relationship: an assertion with no evidence or a non-verified
//! status is reported as such.

use postgres::{Client, Row};

use super::GenealogyError;

/// One evidence citation attached to an assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// The source document reference.
    pub document_ref: String,
    /// The 1-based page number.
    pub page: i32,
}

/// One assertion with its status and citations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionRow {
    /// The assertion id.
    pub assertion_id: i64,
    /// The assertion kind.
    pub kind: String,
    /// The assertion key.
    pub key: String,
    /// The value text.
    pub value_text: Option<String>,
    /// The stored review status.
    pub review_status: String,
    /// The evidence citations.
    pub citations: Vec<Citation>,
}

/// One person and their assertions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonDetail {
    /// The person key.
    pub person_key: String,
    /// The display name, if stored.
    pub display_name: Option<String>,
    /// The person assertions.
    pub assertions: Vec<AssertionRow>,
}

/// One relationship (parent/child or spouse) between two people.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationshipDetail {
    /// The relationship kind (`parent_child` or `spouse`).
    pub relationship_kind: String,
    /// The "from" person key.
    pub from_person_key: String,
    /// The "to" person key.
    pub to_person_key: String,
    /// The assertion status, or `unknown` when no verified assertion backs it.
    pub status: String,
    /// The evidence citations supporting the relationship.
    pub citations: Vec<Citation>,
}

fn load_citations(client: &mut Client, assertion_id: i64) -> Result<Vec<Citation>, GenealogyError> {
    let rows = client
        .query(
            "SELECT document_ref, page FROM assertion_evidence
              WHERE assertion_id = $1 ORDER BY document_ref, page",
            &[&assertion_id],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| Citation {
            document_ref: row.get(0),
            page: row.get(1),
        })
        .collect())
}

fn assertion_rows(
    client: &mut Client,
    person_id: i64,
) -> Result<Vec<AssertionRow>, GenealogyError> {
    let rows = client
        .query(
            "SELECT a.assertion_id, a.assertion_kind, a.assertion_key, a.value_text,
                    a.review_status, ae.document_ref, ae.page
               FROM assertion a
               LEFT JOIN assertion_evidence ae ON ae.assertion_id = a.assertion_id
              WHERE a.person_id = $1
              ORDER BY a.assertion_kind, a.assertion_key, ae.document_ref, ae.page",
            &[&person_id],
        )
        .map_err(GenealogyError::Query)?;
    group_assertion_rows(&rows)
}

/// Groups a joined assertion/evidence result into one row per assertion.
///
/// The query orders by the assertion fields, so each assertion's citations are
/// contiguous. Decoding is fallible so a schema drift cannot panic a report.
fn group_assertion_rows(rows: &[Row]) -> Result<Vec<AssertionRow>, GenealogyError> {
    let mut out: Vec<AssertionRow> = Vec::new();
    for row in rows {
        let assertion_id: i64 = row.try_get(0)?;
        let document_ref: Option<String> = row.try_get(5)?;
        let page: Option<i32> = row.try_get(6)?;
        let citation = match (document_ref, page) {
            (Some(document_ref), Some(page)) => Some(Citation { document_ref, page }),
            _ => None,
        };

        match out.last_mut() {
            Some(last) if last.assertion_id == assertion_id => {
                if let Some(citation) = citation {
                    last.citations.push(citation);
                }
            }
            _ => {
                out.push(AssertionRow {
                    assertion_id,
                    kind: row.try_get(1)?,
                    key: row.try_get(2)?,
                    value_text: row.try_get(3)?,
                    review_status: row.try_get(4)?,
                    citations: citation.into_iter().collect(),
                });
            }
        }
    }
    Ok(out)
}

/// Looks up one person and all assertions by person key.
///
/// Returns `Ok(None)` when no person matches.
pub fn lookup_person(
    client: &mut Client,
    person_key: &str,
) -> Result<Option<PersonDetail>, GenealogyError> {
    let row = client
        .query_opt(
            "SELECT person_id, display_name FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .map_err(GenealogyError::Query)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let person_id: i64 = row.get(0);
    let display_name: Option<String> = row.get(1);
    let assertions = assertion_rows(client, person_id)?;
    Ok(Some(PersonDetail {
        person_key: person_key.to_string(),
        display_name,
        assertions,
    }))
}

/// Looks up the relationship between two person keys.
///
/// The status stays `unknown` unless a backing assertion with status `verified`
/// exists. An unverified or missing assertion never becomes a confirmed link.
pub fn lookup_relationship(
    client: &mut Client,
    from_person_key: &str,
    to_person_key: &str,
) -> Result<RelationshipDetail, GenealogyError> {
    let link = client
        .query_opt(
            "SELECT 'parent_child'
               FROM parent_child_link pcl
               JOIN person p ON p.person_id = pcl.parent_id
               JOIN person c ON c.person_id = pcl.child_id
              WHERE p.person_key = $1 AND c.person_key = $2
             UNION ALL
             SELECT 'spouse'
               FROM spouse_link sl
               JOIN person p ON p.person_id = sl.person_id
               JOIN person s ON s.person_id = sl.spouse_id
              WHERE p.person_key = $1 AND s.person_key = $2
              LIMIT 1",
            &[&from_person_key, &to_person_key],
        )
        .map_err(GenealogyError::Query)?;

    let relationship_kind = link
        .map(|row| row.get::<_, String>(0))
        .unwrap_or_else(|| "unknown".to_string());

    // Find any verified assertion that names the relationship.
    let assertion = client
        .query_opt(
            "SELECT a.assertion_id
               FROM assertion a
               JOIN person p ON p.person_id = a.person_id
               JOIN person o ON o.person_id = a.object_person_id
              WHERE p.person_key = $1 AND o.person_key = $2
                AND a.review_status = 'verified'
              LIMIT 1",
            &[&from_person_key, &to_person_key],
        )
        .map_err(GenealogyError::Query)?;

    match assertion {
        Some(row) => {
            let assertion_id: i64 = row.get(0);
            let citations = load_citations(client, assertion_id)?;
            Ok(RelationshipDetail {
                relationship_kind,
                from_person_key: from_person_key.to_string(),
                to_person_key: to_person_key.to_string(),
                status: "verified".to_string(),
                citations,
            })
        }
        None => Ok(RelationshipDetail {
            relationship_kind,
            from_person_key: from_person_key.to_string(),
            to_person_key: to_person_key.to_string(),
            status: "unknown".to_string(),
            citations: Vec::new(),
        }),
    }
}

/// Returns every assertion that is not yet resolved.
///
/// A resolved assertion is `verified` or `rejected`. Candidates in
/// `unverified`, `needs_review`, or `disputed` state are returned with their
/// evidence citations.
pub fn unresolved_assertions(client: &mut Client) -> Result<Vec<AssertionRow>, GenealogyError> {
    let rows = client
        .query(
            "SELECT a.assertion_id, a.assertion_kind, a.assertion_key, a.value_text,
                    a.review_status, ae.document_ref, ae.page
               FROM assertion a
               LEFT JOIN assertion_evidence ae ON ae.assertion_id = a.assertion_id
              WHERE a.review_status IN ('unverified', 'needs_review', 'disputed')
              ORDER BY a.assertion_id, ae.document_ref, ae.page",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    group_assertion_rows(&rows)
}
