//! Reviewed document-to-genealogy proposals (TRAN-DOC-EXTRACT).
//!
//! An approved, processed family document can be parsed into *draft* proposals.
//! Nothing is written to `person`, `assertion`, or the link tables until a
//! reviewer approves an individual proposal and applies the extraction.
//!
//! The workflow is deliberately separate from document approval:
//!
//! - document review decides whether a file may be indexed;
//! - this workflow decides whether the claims inside it become genealogy
//!   records, one proposal at a time.
//!
//! Applying a proposal never verifies an assertion. Every assertion written
//! here is `unverified` and keeps the document reference, the supporting text,
//! and the document version that produced it.

use postgres::{Client, Transaction};
use serde::Serialize;
use thiserror::Error;

use super::document_card::{parse_card, safe_social_url, CardDraft, RelationshipKind};
use super::person_search::{self, Candidate};
use super::GenealogyError;

/// The most proposals one extraction may hold.
pub const MAX_PROPOSALS: usize = 200;

/// The most candidates shown for one person reference.
pub const MAX_CANDIDATES: usize = 5;

/// One candidate shown to a reviewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct CandidateSummary {
    pub person_key: String,
    pub display_name: String,
    pub evidence_status: String,
    pub match_kind: String,
    pub matched_name: String,
}

/// How a person reference resolved at extraction time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Exactly one recorded person matched at the strongest tier.
    Matched,
    /// Several people tied, or the only match was weak. A reviewer must choose.
    Ambiguous,
    /// Nothing recorded matched. A reviewer may create a person.
    Unresolved,
    /// The reviewer chose to create a new person.
    Create,
}

impl Resolution {
    pub fn as_str(self) -> &'static str {
        match self {
            Resolution::Matched => "matched",
            Resolution::Ambiguous => "ambiguous",
            Resolution::Unresolved => "unresolved",
            Resolution::Create => "create",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "matched" => Resolution::Matched,
            "ambiguous" => Resolution::Ambiguous,
            "create" => Resolution::Create,
            _ => Resolution::Unresolved,
        }
    }
}

/// One stored proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalRow {
    pub proposal_id: i64,
    pub ordinal: i32,
    pub kind: String,
    pub subject_name: String,
    pub target_name: Option<String>,
    pub resolution: String,
    pub resolved_person_key: Option<String>,
    pub candidates: Vec<CandidateSummary>,
    pub value_text: Option<String>,
    pub support_text: String,
    /// The 1-based line in the document that the proposal quotes.
    pub support_line: i32,
    pub status: String,
    pub decided_by: Option<String>,
    pub applied_result: Option<serde_json::Value>,
}

/// One extraction with its proposals.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionRow {
    pub extraction_id: i64,
    pub document_version_id: i64,
    pub corpus: String,
    pub document_source: String,
    pub content_sha256: String,
    pub status: String,
    pub created_by: String,
    pub created_at: String,
    pub applied_by: Option<String>,
    pub applied_at: Option<String>,
}

/// One extraction and every proposal it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractionDetail {
    pub extraction: ExtractionRow,
    pub proposals: Vec<ProposalRow>,
}

/// Input for extracting one document version.
pub struct ExtractionInput<'a> {
    pub document_version_id: i64,
    pub corpus: &'a str,
    pub document_source: &'a str,
    pub content_sha256: &'a str,
    pub text: &'a str,
    pub actor: &'a str,
}

/// One reviewer decision.
pub struct DecisionInput<'a> {
    pub proposal_id: i64,
    pub decision: &'a str,
    /// Select an existing person for a `person` proposal.
    pub person_key: Option<&'a str>,
    /// Create a new person for a `person` proposal.
    pub create_person: bool,
    /// Correct the alias or the URL for `alias` / `social_link`.
    pub value_text: Option<&'a str>,
    pub actor: &'a str,
}

/// One applied proposal and what it produced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AppliedProposal {
    pub proposal_id: i64,
    pub kind: String,
    pub result: serde_json::Value,
}

/// The result of applying an extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApplyReport {
    pub extraction_id: i64,
    pub applied: Vec<AppliedProposal>,
    /// True when the extraction had already been applied.
    pub already_applied: bool,
}

/// Errors raised by the extraction workflow.
#[derive(Debug, Error)]
pub enum ExtractionError {
    #[error("the document states no person record")]
    NoRecords,
    #[error("no extraction exists for this document version")]
    NotFound,
    #[error("the extraction no longer matches the reviewed document version")]
    Stale,
    #[error("the proposal has already been decided")]
    Decided,
    #[error("a person reference still needs a reviewer decision: {0}")]
    UnresolvedPerson(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Genealogy(#[from] GenealogyError),
}

impl From<postgres::Error> for ExtractionError {
    fn from(err: postgres::Error) -> Self {
        ExtractionError::Genealogy(GenealogyError::Query(err))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn stable_key(prefix: &str, value: &str) -> String {
    format!("{prefix}-{:016x}", fnv1a64(value.as_bytes()))
}

/// A person key minted for an explicit create-person decision.
///
/// It is derived from the document version and the normalized name, so a
/// repeated apply of the same extraction lands on the same key.
fn created_person_key(document_version_id: i64, name: &str) -> String {
    let normalized = name
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    stable_key("doc", &format!("{document_version_id}:{normalized}"))
}

fn candidate_summary(candidate: &Candidate) -> CandidateSummary {
    CandidateSummary {
        person_key: candidate.person_key.clone(),
        display_name: candidate.display_name.clone(),
        evidence_status: candidate.evidence_status.clone(),
        match_kind: candidate
            .match_kind
            .map(|kind| kind.as_str().to_string())
            .unwrap_or_else(|| "none".to_string()),
        matched_name: candidate.matched_name.clone().unwrap_or_default(),
    }
}

fn match_reference(
    catalog: &[person_search::PersonNames],
    name: &str,
) -> (Resolution, Option<String>, Vec<CandidateSummary>) {
    let ranked = person_search::rank(catalog, name);
    let top = person_search::top_tier(&ranked);
    let candidates: Vec<CandidateSummary> = top
        .iter()
        .take(MAX_CANDIDATES)
        .map(|candidate| candidate_summary(candidate))
        .collect();
    match top.as_slice() {
        [] => (Resolution::Unresolved, None, candidates),
        [only] if person_search::is_exact_name(only) => (
            Resolution::Matched,
            Some(only.person_key.clone()),
            candidates,
        ),
        // Several people tied, or the only lead is a reordered/partial name:
        // a reviewer confirms the identity explicitly.
        _ => (Resolution::Ambiguous, None, candidates),
    }
}

/// One draft before it is persisted.
struct Draft {
    kind: &'static str,
    subject_name: String,
    target_name: Option<String>,
    resolution: Resolution,
    resolved_person_key: Option<String>,
    candidates: Vec<CandidateSummary>,
    value_text: Option<String>,
    support_text: String,
    support_line: usize,
}

fn build_drafts(card: &CardDraft, catalog: &[person_search::PersonNames]) -> Vec<Draft> {
    let mut drafts: Vec<Draft> = Vec::new();
    let mut seen_people: Vec<String> = Vec::new();

    let mut push_person = |name: &str, support: &str, line: usize, drafts: &mut Vec<Draft>| {
        if seen_people.iter().any(|seen| seen == name) {
            return;
        }
        seen_people.push(name.to_string());
        let (resolution, resolved_person_key, candidates) = match_reference(catalog, name);
        drafts.push(Draft {
            kind: "person",
            subject_name: name.to_string(),
            target_name: None,
            resolution,
            resolved_person_key,
            candidates,
            value_text: None,
            support_text: support.to_string(),
            support_line: line,
        });
    };

    push_person(
        &card.subject,
        &card.subject_support,
        card.subject_line,
        &mut drafts,
    );
    for relationship in &card.relationships {
        push_person(
            &relationship.target,
            &relationship.support,
            relationship.line,
            &mut drafts,
        );
    }

    for relationship in &card.relationships {
        drafts.push(Draft {
            kind: match relationship.kind {
                RelationshipKind::ParentChild => "parent_child",
                RelationshipKind::Spouse => "spouse",
            },
            subject_name: card.subject.clone(),
            target_name: Some(relationship.target.clone()),
            resolution: Resolution::Matched,
            resolved_person_key: None,
            candidates: Vec::new(),
            value_text: None,
            support_text: relationship.support.clone(),
            support_line: relationship.line,
        });
    }

    for alias in &card.aliases {
        drafts.push(Draft {
            kind: "alias",
            subject_name: card.subject.clone(),
            target_name: None,
            resolution: Resolution::Matched,
            resolved_person_key: None,
            candidates: Vec::new(),
            value_text: Some(alias.name.clone()),
            support_text: alias.support.clone(),
            support_line: alias.line,
        });
    }

    for social in &card.socials {
        drafts.push(Draft {
            kind: "social_link",
            subject_name: card.subject.clone(),
            target_name: None,
            resolution: Resolution::Matched,
            resolved_person_key: None,
            candidates: Vec::new(),
            value_text: Some(social.url.clone()),
            support_text: social.support.clone(),
            support_line: social.line,
        });
    }

    drafts.truncate(MAX_PROPOSALS);
    drafts
}

fn load_extraction(
    client: &mut Client,
    version_id: i64,
) -> Result<Option<ExtractionRow>, GenealogyError> {
    let row = client.query_opt(
        "SELECT extraction_id, document_version_id, corpus, document_source, content_sha256,
                status, created_by, created_at::text, applied_by, applied_at::text
           FROM document_extraction
          WHERE document_version_id = $1
          ORDER BY extraction_id DESC
          LIMIT 1",
        &[&version_id],
    )?;
    Ok(row.map(|row| ExtractionRow {
        extraction_id: row.get(0),
        document_version_id: row.get(1),
        corpus: row.get(2),
        document_source: row.get(3),
        content_sha256: row.get(4),
        status: row.get(5),
        created_by: row.get(6),
        created_at: row.get(7),
        applied_by: row.get(8),
        applied_at: row.get(9),
    }))
}

fn load_proposals(
    tx: &mut Transaction<'_>,
    extraction_id: i64,
    for_update: bool,
) -> Result<Vec<ProposalRow>, GenealogyError> {
    let sql = format!(
        "SELECT proposal_id, ordinal, kind, subject_name, target_name, resolution,
                resolved_person_key, candidates_json::text, value_text, support_text,
                support_line, status, decided_by, applied_result::text
           FROM document_proposal
          WHERE extraction_id = $1
          ORDER BY ordinal{}",
        if for_update { " FOR UPDATE" } else { "" }
    );
    let rows = tx.query(&sql, &[&extraction_id])?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let candidates_json: String = row.get(7);
            let applied_json: Option<String> = row.get(13);
            ProposalRow {
                proposal_id: row.get(0),
                ordinal: row.get(1),
                kind: row.get(2),
                subject_name: row.get(3),
                target_name: row.get(4),
                resolution: row.get(5),
                resolved_person_key: row.get(6),
                candidates: serde_json::from_str(&candidates_json).unwrap_or_default(),
                value_text: row.get(8),
                support_text: row.get(9),
                support_line: row.get(10),
                status: row.get(11),
                decided_by: row.get(12),
                applied_result: applied_json.and_then(|value| serde_json::from_str(&value).ok()),
            }
        })
        .collect())
}

/// Reads the current extraction for a document version.
pub fn detail(
    client: &mut Client,
    version_id: i64,
) -> Result<Option<ExtractionDetail>, GenealogyError> {
    let Some(extraction) = load_extraction(client, version_id)? else {
        return Ok(None);
    };
    let mut tx = client.transaction()?;
    let proposals = load_proposals(&mut tx, extraction.extraction_id, false)?;
    tx.commit()?;
    Ok(Some(ExtractionDetail {
        extraction,
        proposals,
    }))
}

/// Extracts one document version into draft proposals.
///
/// Repeating the call for the same version and content hash returns the stored
/// extraction unchanged, so a reviewer never loses decisions.
pub fn extract(
    client: &mut Client,
    input: ExtractionInput<'_>,
) -> Result<ExtractionDetail, ExtractionError> {
    let Some(card) = parse_card(input.text) else {
        return Err(ExtractionError::NoRecords);
    };
    if let Some(existing) = detail(client, input.document_version_id)? {
        if existing.extraction.content_sha256 == input.content_sha256 {
            return Ok(existing);
        }
        // A replacement document must not inherit an old decision.
        return Err(ExtractionError::Stale);
    }
    let catalog = person_search::load_catalog(client)?.people;
    let drafts = build_drafts(&card, &catalog);

    let mut tx = client.transaction()?;
    let inserted = tx.query_opt(
        "INSERT INTO document_extraction
                 (document_version_id, corpus, document_source, content_sha256, created_by)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (document_version_id, content_sha256) DO NOTHING
             RETURNING extraction_id, created_at::text",
        &[
            &input.document_version_id,
            &input.corpus,
            &input.document_source,
            &input.content_sha256,
            &input.actor,
        ],
    )?;
    let Some(row) = inserted else {
        tx.rollback()?;
        return detail(client, input.document_version_id)
            .map_err(ExtractionError::Genealogy)?
            .ok_or(ExtractionError::NotFound);
    };
    let extraction_id: i64 = row.get(0);
    let created_at: String = row.get(1);
    for (index, draft) in drafts.iter().enumerate() {
        tx.execute(
            "INSERT INTO document_proposal
                 (extraction_id, ordinal, kind, subject_name, target_name, resolution,
                  resolved_person_key, candidates_json, value_text, support_text, support_line)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8::text::jsonb, $9, $10, $11)",
            &[
                &extraction_id,
                &((index + 1) as i32),
                &draft.kind,
                &draft.subject_name,
                &draft.target_name,
                &draft.resolution.as_str(),
                &draft.resolved_person_key,
                &serde_json::to_string(&draft.candidates)
                    .map_err(|err| ExtractionError::Invalid(err.to_string()))?,
                &draft.value_text,
                &draft.support_text,
                &(draft.support_line as i32),
            ],
        )?;
    }
    tx.execute(
        "INSERT INTO document_extraction_event (extraction_id, action, actor, detail)
         VALUES ($1, 'extracted', $2, $3::text::jsonb)",
        &[
            &extraction_id,
            &input.actor,
            &serde_json::json!({
                "proposals": drafts.len(),
                "document_source": input.document_source,
                "content_sha256": input.content_sha256,
            })
            .to_string(),
        ],
    )?;
    let proposals = load_proposals(&mut tx, extraction_id, false)?;
    tx.commit()?;
    Ok(ExtractionDetail {
        extraction: ExtractionRow {
            extraction_id,
            document_version_id: input.document_version_id,
            corpus: input.corpus.to_string(),
            document_source: input.document_source.to_string(),
            content_sha256: input.content_sha256.to_string(),
            status: "draft".to_string(),
            created_by: input.actor.to_string(),
            created_at,
            applied_by: None,
            applied_at: None,
        },
        proposals,
    })
}

/// Records one reviewer decision.
pub fn decide(
    client: &mut Client,
    input: DecisionInput<'_>,
) -> Result<ExtractionDetail, ExtractionError> {
    let decision = input.decision.trim().to_ascii_lowercase();
    if !matches!(decision.as_str(), "approve" | "reject") {
        return Err(ExtractionError::Invalid(
            "the decision must be approve or reject".to_string(),
        ));
    }
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT extraction_id, kind, status, subject_name, resolution
               FROM document_proposal WHERE proposal_id = $1 FOR UPDATE",
            &[&input.proposal_id],
        )?
        .ok_or(ExtractionError::Invalid("unknown proposal".to_string()))?;
    let extraction_id: i64 = row.get(0);
    let kind: String = row.get(1);
    let status: String = row.get(2);
    let subject_name: String = row.get(3);
    let current_resolution: String = row.get(4);
    if status != "proposed" && status != "approved" {
        return Err(ExtractionError::Decided);
    }

    let mut resolution: Option<String> = None;
    let mut resolved_person_key: Option<String> = None;
    let mut value_text: Option<String> = None;

    if decision == "approve" {
        match kind.as_str() {
            "person" => {
                if input.create_person {
                    resolution = Some(Resolution::Create.as_str().to_string());
                } else if let Some(key) = input.person_key.map(str::trim).filter(|k| !k.is_empty())
                {
                    let exists: Option<i64> = tx
                        .query_opt(
                            "SELECT person_id FROM person WHERE person_key = $1",
                            &[&key],
                        )?
                        .map(|row| row.get(0));
                    if exists.is_none() {
                        return Err(ExtractionError::Invalid(
                            "the selected person does not exist".to_string(),
                        ));
                    }
                    resolution = Some(Resolution::Matched.as_str().to_string());
                    resolved_person_key = Some(key.to_string());
                } else if current_resolution != "matched" {
                    // A namesake or an unmatched reference must be resolved
                    // explicitly; approving it as-is would guess an identity.
                    return Err(ExtractionError::Invalid(
                        "choose an existing person or create one before approving".to_string(),
                    ));
                }
            }
            "alias" => {
                if let Some(value) = input.value_text.map(str::trim).filter(|v| !v.is_empty()) {
                    if value.chars().count() > 160 {
                        return Err(ExtractionError::Invalid(
                            "the alias is too long".to_string(),
                        ));
                    }
                    value_text = Some(value.to_string());
                }
            }
            "social_link" => {
                if let Some(value) = input.value_text.map(str::trim).filter(|v| !v.is_empty()) {
                    if safe_social_url(value).is_none() {
                        return Err(ExtractionError::Invalid(
                            "a social link must be an http(s) URL".to_string(),
                        ));
                    }
                    value_text = Some(value.to_string());
                }
            }
            _ => {}
        }
    }

    tx.execute(
        "UPDATE document_proposal
            SET status = $2,
                decided_by = $3,
                decided_at = now(),
                resolution = COALESCE($4, resolution),
                resolved_person_key = COALESCE($5, resolved_person_key),
                value_text = COALESCE($6, value_text)
          WHERE proposal_id = $1",
        &[
            &input.proposal_id,
            &(if decision == "approve" {
                "approved"
            } else {
                "rejected"
            }),
            &input.actor,
            &resolution,
            &resolved_person_key,
            &value_text,
        ],
    )?;
    tx.execute(
        "INSERT INTO document_extraction_event (extraction_id, action, actor, detail)
         VALUES ($1, 'decided', $2, $3::text::jsonb)",
        &[
            &extraction_id,
            &input.actor,
            &serde_json::json!({
                "proposal_id": input.proposal_id,
                "decision": decision,
                "kind": kind,
                "subject_name": subject_name,
                "person_key": resolved_person_key,
            })
            .to_string(),
        ],
    )?;
    tx.commit()?;
    let version_id = client
        .query_one(
            "SELECT document_version_id FROM document_extraction WHERE extraction_id = $1",
            &[&extraction_id],
        )?
        .get(0);
    detail(client, version_id)
        .map_err(ExtractionError::Genealogy)?
        .ok_or(ExtractionError::NotFound)
}

/// Records a reviewed reference spelling as a searchable alias.
///
/// `None` means the alias was already recorded, so a repeat apply is a no-op.
#[allow(clippy::too_many_arguments)]
fn ensure_alias(
    tx: &mut Transaction<'_>,
    person_id: i64,
    alias: &str,
    document_ref: &str,
    support: &str,
    support_line: i32,
    document_version_id: i64,
) -> Result<Option<i64>, GenealogyError> {
    let existing: Option<i64> = tx
        .query_opt(
            "SELECT assertion_id FROM assertion
              WHERE person_id = $1 AND assertion_kind = 'name_variant'
                AND lower(value_text) = lower($2)
              LIMIT 1",
            &[&person_id, &alias],
        )?
        .map(|row| row.get(0));
    if existing.is_some() {
        return Ok(None);
    }
    let assertion_id = ensure_assertion(
        tx,
        person_id,
        "name_variant",
        &stable_key("variant", &alias.to_lowercase()),
        alias,
        None,
        document_ref,
        support,
        support_line,
        document_version_id,
    )?;
    Ok(Some(assertion_id))
}

#[allow(clippy::too_many_arguments)]
fn add_evidence(
    tx: &mut Transaction<'_>,
    assertion_id: i64,
    document_ref: &str,
    support: &str,
    support_line: i32,
    document_version_id: i64,
) -> Result<(), GenealogyError> {
    // `page` is still required by the schema, but a text document has no real
    // page. The line number is what the reader sees, and the document version
    // lets a withdrawal take this evidence out of current answers.
    let locator = format!("line {support_line}");
    tx.execute(
        "INSERT INTO assertion_evidence
             (assertion_id, document_ref, page, locator, excerpt, document_version_id)
         VALUES ($1, $2, 1, $3, $4, $5)
         ON CONFLICT (assertion_id, document_ref, page) DO UPDATE
            SET locator = EXCLUDED.locator,
                excerpt = EXCLUDED.excerpt,
                document_version_id = EXCLUDED.document_version_id,
                withdrawn_at = NULL",
        &[
            &assertion_id,
            &document_ref,
            &locator,
            &support,
            &document_version_id,
        ],
    )?;
    Ok(())
}

/// Withdraws every evidence row produced by one document version.
///
/// The rows stay for the audit trail; reads filter them out, so a withdrawn
/// document immediately stops supporting current answers.
pub fn withdraw_document_evidence(
    client: &mut Client,
    document_version_id: i64,
    document_ref: &str,
) -> Result<u64, GenealogyError> {
    // Evidence written before migration 0024 has no version id, so the source
    // reference is matched as well. The id is filled in while withdrawing, so a
    // later removal is a pure id lookup.
    let updated = client.execute(
        "UPDATE assertion_evidence
            SET withdrawn_at = now(),
                document_version_id = COALESCE(document_version_id, $1)
          WHERE withdrawn_at IS NULL
            AND (document_version_id = $1 OR document_ref = $2)",
        &[&document_version_id, &document_ref],
    )?;
    Ok(updated)
}

#[allow(clippy::too_many_arguments)]
fn ensure_assertion(
    tx: &mut Transaction<'_>,
    person_id: i64,
    kind: &str,
    key: &str,
    value: &str,
    object_person_id: Option<i64>,
    document_ref: &str,
    support: &str,
    support_line: i32,
    document_version_id: i64,
) -> Result<i64, GenealogyError> {
    let existing = tx.query_opt(
        "SELECT assertion_id FROM assertion
          WHERE person_id = $1 AND assertion_kind = $2 AND assertion_key = $3",
        &[&person_id, &kind, &key],
    )?;
    let assertion_id: i64 = match existing {
        Some(row) => row.get(0),
        None => tx
            .query_one(
                "INSERT INTO assertion
                     (person_id, assertion_kind, assertion_key, value_text,
                      object_person_id, review_status)
                 VALUES ($1, $2, $3, $4, $5, 'unverified')
                 RETURNING assertion_id",
                &[&person_id, &kind, &key, &value, &object_person_id],
            )?
            .get(0),
    };
    add_evidence(
        tx,
        assertion_id,
        document_ref,
        support,
        support_line,
        document_version_id,
    )?;
    Ok(assertion_id)
}

fn person_id_for(tx: &mut Transaction<'_>, key: &str) -> Result<Option<i64>, GenealogyError> {
    Ok(tx
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&key],
        )?
        .map(|row| row.get(0)))
}

/// Applies every approved proposal in one transaction.
///
/// The caller must have revalidated the RAG document version first and pass its
/// current content hash. A mismatch means the document was replaced or
/// withdrawn and the extraction is stale.
pub fn apply(
    client: &mut Client,
    document_version_id: i64,
    content_sha256: &str,
    actor: &str,
) -> Result<ApplyReport, ExtractionError> {
    let mut tx = client.transaction()?;
    let row = tx
        .query_opt(
            "SELECT extraction_id, content_sha256, status, document_source
               FROM document_extraction
              WHERE document_version_id = $1
              ORDER BY extraction_id DESC
              LIMIT 1
              FOR UPDATE",
            &[&document_version_id],
        )?
        .ok_or(ExtractionError::NotFound)?;
    let extraction_id: i64 = row.get(0);
    let stored_sha: String = row.get(1);
    let status: String = row.get(2);
    let document_ref: String = row.get(3);
    if stored_sha != content_sha256 {
        return Err(ExtractionError::Stale);
    }

    let proposals = load_proposals(&mut tx, extraction_id, true)?;
    let approved_present = proposals
        .iter()
        .any(|proposal| proposal.status == "approved");
    if !approved_present {
        if status == "applied" {
            // Nothing new to apply: report what the extraction already wrote.
            let applied = proposals
                .iter()
                .filter(|proposal| proposal.status == "applied")
                .map(|proposal| AppliedProposal {
                    proposal_id: proposal.proposal_id,
                    kind: proposal.kind.clone(),
                    result: proposal
                        .applied_result
                        .clone()
                        .unwrap_or(serde_json::Value::Null),
                })
                .collect();
            tx.commit()?;
            return Ok(ApplyReport {
                extraction_id,
                applied,
                already_applied: true,
            });
        }
        return Err(ExtractionError::Invalid(
            "no proposal has been approved yet".to_string(),
        ));
    }

    // Resolve every approved person reference before writing anything.
    let mut person_keys: Vec<(String, String)> = Vec::new();
    for proposal in proposals.iter().filter(|proposal| {
        proposal.kind == "person" && matches!(proposal.status.as_str(), "approved" | "applied")
    }) {
        let key = if proposal.status == "applied" {
            // An earlier apply already created or matched this person.
            applied_person_key(proposal)?
        } else {
            match Resolution::parse(&proposal.resolution) {
                Resolution::Matched => proposal.resolved_person_key.clone().ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(proposal.subject_name.clone())
                })?,
                Resolution::Create => {
                    created_person_key(document_version_id, &proposal.subject_name)
                }
                Resolution::Ambiguous | Resolution::Unresolved => {
                    return Err(ExtractionError::UnresolvedPerson(
                        proposal.subject_name.clone(),
                    ))
                }
            }
        };
        person_keys.push((proposal.subject_name.clone(), key));
    }

    let mut applied: Vec<AppliedProposal> = Vec::new();

    for proposal in &proposals {
        if proposal.status != "approved" {
            continue;
        }
        match proposal.kind.as_str() {
            "person" => {
                let key = person_keys
                    .iter()
                    .find(|(name, _)| name == &proposal.subject_name)
                    .map(|(_, key)| key.clone())
                    .ok_or_else(|| {
                        ExtractionError::UnresolvedPerson(proposal.subject_name.clone())
                    })?;
                let (person_id, created) = match person_id_for(&mut tx, &key)? {
                    Some(person_id) => (person_id, false),
                    None => {
                        let person_id: i64 = tx
                            .query_one(
                                "INSERT INTO person (person_key, display_name)
                                 VALUES ($1, $2)
                                 RETURNING person_id",
                                &[&key, &proposal.subject_name],
                            )?
                            .get(0);
                        ensure_assertion(
                            &mut tx,
                            person_id,
                            "display_name",
                            "name",
                            &proposal.subject_name,
                            None,
                            &document_ref,
                            &proposal.support_text,
                            proposal.support_line,
                            document_version_id,
                        )?;
                        (person_id, true)
                    }
                };
                // A confirmed reference that is spelled differently from the
                // recorded name (for example the English name order) becomes a
                // searchable alias, so the identity is explicit next time.
                let display: Option<String> = tx
                    .query_opt(
                        "SELECT NULLIF(BTRIM(display_name), '')
                           FROM person WHERE person_id = $1",
                        &[&person_id],
                    )?
                    .and_then(|row| row.get::<_, Option<String>>(0));
                let alias_recorded = match display.as_deref() {
                    Some(display)
                        if person_search::fold(display)
                            != person_search::fold(&proposal.subject_name) =>
                    {
                        ensure_alias(
                            &mut tx,
                            person_id,
                            &proposal.subject_name,
                            &document_ref,
                            &proposal.support_text,
                            proposal.support_line,
                            document_version_id,
                        )?
                    }
                    _ => None,
                };
                applied.push(AppliedProposal {
                    proposal_id: proposal.proposal_id,
                    kind: proposal.kind.clone(),
                    result: serde_json::json!({
                        "person_key": key,
                        "person_id": person_id,
                        "created": created,
                        "alias_recorded": if alias_recorded.is_some() {
                            Some(proposal.subject_name.clone())
                        } else {
                            None
                        },
                        "alias_assertion_id": alias_recorded,
                    }),
                });
            }
            "parent_child" => {
                let child_key = resolved_key(&person_keys, &proposal.subject_name)?;
                let parent_key = resolved_key(
                    &person_keys,
                    proposal.target_name.as_deref().unwrap_or_default(),
                )?;
                if child_key == parent_key {
                    return Err(ExtractionError::Invalid(
                        "a person cannot be their own parent".to_string(),
                    ));
                }
                let child_id = person_id_for(&mut tx, &child_key)?.ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(proposal.subject_name.clone())
                })?;
                let parent_id = person_id_for(&mut tx, &parent_key)?.ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(
                        proposal.target_name.clone().unwrap_or_default(),
                    )
                })?;
                let inserted = tx.execute(
                    "INSERT INTO parent_child_link (parent_id, child_id)
                     VALUES ($1, $2)
                     ON CONFLICT (parent_id, child_id) DO NOTHING",
                    &[&parent_id, &child_id],
                )?;
                let parent_name: String = tx
                    .query_one(
                        "SELECT COALESCE(NULLIF(BTRIM(display_name), ''), 'Unnamed person')
                           FROM person WHERE person_id = $1",
                        &[&parent_id],
                    )?
                    .get(0);
                let assertion_id = ensure_assertion(
                    &mut tx,
                    child_id,
                    "parent_child",
                    &stable_key("parent_child", &parent_key),
                    &parent_name,
                    Some(parent_id),
                    &document_ref,
                    &proposal.support_text,
                    proposal.support_line,
                    document_version_id,
                )?;
                // Repair a row written by an earlier extraction that stored the
                // raw key as the value. Never touch a reviewed value.
                tx.execute(
                    "UPDATE assertion SET value_text = $3
                      WHERE assertion_id = $1 AND review_status = 'unverified'
                        AND value_text = $2",
                    &[&assertion_id, &parent_key, &parent_name],
                )?;
                applied.push(AppliedProposal {
                    proposal_id: proposal.proposal_id,
                    kind: proposal.kind.clone(),
                    result: serde_json::json!({
                        "child_key": child_key,
                        "parent_key": parent_key,
                        "link_created": inserted > 0,
                        "assertion_id": assertion_id,
                    }),
                });
            }
            "spouse" => {
                let first_key = resolved_key(&person_keys, &proposal.subject_name)?;
                let second_key = resolved_key(
                    &person_keys,
                    proposal.target_name.as_deref().unwrap_or_default(),
                )?;
                if first_key == second_key {
                    return Err(ExtractionError::Invalid(
                        "a person cannot be their own spouse".to_string(),
                    ));
                }
                let first_id = person_id_for(&mut tx, &first_key)?.ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(proposal.subject_name.clone())
                })?;
                let second_id = person_id_for(&mut tx, &second_key)?.ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(
                        proposal.target_name.clone().unwrap_or_default(),
                    )
                })?;
                let inserted = tx.execute(
                    "INSERT INTO spouse_link (person_id, spouse_id, source_order)
                     SELECT $1, $2, 1
                      WHERE NOT EXISTS (
                        SELECT 1 FROM spouse_link existing
                         WHERE (existing.person_id = $1 AND existing.spouse_id = $2)
                            OR (existing.person_id = $2 AND existing.spouse_id = $1)
                      )
                     ON CONFLICT (person_id, spouse_id, source_order) DO NOTHING",
                    &[&first_id, &second_id],
                )?;
                let spouse_name: String = tx
                    .query_one(
                        "SELECT COALESCE(NULLIF(BTRIM(display_name), ''), 'Unnamed person')
                           FROM person WHERE person_id = $1",
                        &[&second_id],
                    )?
                    .get(0);
                let assertion_id = ensure_assertion(
                    &mut tx,
                    first_id,
                    "spouse",
                    &stable_key("spouse", &second_key),
                    &spouse_name,
                    Some(second_id),
                    &document_ref,
                    &proposal.support_text,
                    proposal.support_line,
                    document_version_id,
                )?;
                tx.execute(
                    "UPDATE assertion SET value_text = $3
                      WHERE assertion_id = $1 AND review_status = 'unverified'
                        AND value_text = $2",
                    &[&assertion_id, &second_key, &spouse_name],
                )?;
                applied.push(AppliedProposal {
                    proposal_id: proposal.proposal_id,
                    kind: proposal.kind.clone(),
                    result: serde_json::json!({
                        "person_key": first_key,
                        "spouse_key": second_key,
                        "link_created": inserted > 0,
                        "assertion_id": assertion_id,
                    }),
                });
            }
            "alias" => {
                let key = resolved_key(&person_keys, &proposal.subject_name)?;
                let person_id = person_id_for(&mut tx, &key)?.ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(proposal.subject_name.clone())
                })?;
                let alias = proposal
                    .value_text
                    .clone()
                    .ok_or_else(|| ExtractionError::Invalid("alias value missing".to_string()))?;
                let assertion_id = ensure_assertion(
                    &mut tx,
                    person_id,
                    "name_variant",
                    &stable_key("variant", &alias.to_lowercase()),
                    &alias,
                    None,
                    &document_ref,
                    &proposal.support_text,
                    proposal.support_line,
                    document_version_id,
                )?;
                applied.push(AppliedProposal {
                    proposal_id: proposal.proposal_id,
                    kind: proposal.kind.clone(),
                    result: serde_json::json!({
                        "person_key": key,
                        "alias": alias,
                        "assertion_id": assertion_id,
                    }),
                });
            }
            "social_link" => {
                let key = resolved_key(&person_keys, &proposal.subject_name)?;
                let person_id = person_id_for(&mut tx, &key)?.ok_or_else(|| {
                    ExtractionError::UnresolvedPerson(proposal.subject_name.clone())
                })?;
                let raw = proposal.value_text.clone().ok_or_else(|| {
                    ExtractionError::Invalid("social link value missing".to_string())
                })?;
                let url = safe_social_url(&raw).ok_or_else(|| {
                    ExtractionError::Invalid("a social link must be an http(s) URL".to_string())
                })?;
                let assertion_id = ensure_assertion(
                    &mut tx,
                    person_id,
                    "social_link",
                    &stable_key("link", &url),
                    &url,
                    None,
                    &document_ref,
                    &proposal.support_text,
                    proposal.support_line,
                    document_version_id,
                )?;
                applied.push(AppliedProposal {
                    proposal_id: proposal.proposal_id,
                    kind: proposal.kind.clone(),
                    result: serde_json::json!({
                        "person_key": key,
                        "url": url,
                        "assertion_id": assertion_id,
                    }),
                });
            }
            _ => {}
        }
    }

    for applied_proposal in &applied {
        tx.execute(
            "UPDATE document_proposal
                SET status = 'applied', applied_at = now(), applied_result = $2::text::jsonb
              WHERE proposal_id = $1",
            &[
                &applied_proposal.proposal_id,
                &serde_json::to_string(&applied_proposal.result)
                    .map_err(|err| ExtractionError::Invalid(err.to_string()))?,
            ],
        )?;
    }
    tx.execute(
        "UPDATE document_extraction
            SET status = 'applied', applied_by = $2, applied_at = now()
          WHERE extraction_id = $1",
        &[&extraction_id, &actor],
    )?;
    tx.execute(
        "INSERT INTO document_extraction_event (extraction_id, action, actor, detail)
         VALUES ($1, 'applied', $2, $3::text::jsonb)",
        &[
            &extraction_id,
            &actor,
            &serde_json::json!({
                "applied": applied.len(),
                "proposal_ids": applied.iter().map(|item| item.proposal_id).collect::<Vec<_>>(),
            })
            .to_string(),
        ],
    )?;
    tx.commit()?;
    Ok(ApplyReport {
        extraction_id,
        applied,
        // A later approval can still be applied to an extraction that was
        // already applied once; this flag then means it had been applied before.
        already_applied: status == "applied",
    })
}

/// The person key an already-applied `person` proposal produced.
fn applied_person_key(proposal: &ProposalRow) -> Result<String, ExtractionError> {
    if let Some(key) = proposal.resolved_person_key.clone() {
        return Ok(key);
    }
    if let Some(key) = proposal
        .applied_result
        .as_ref()
        .and_then(|value| value.get("person_key"))
        .and_then(|value| value.as_str())
    {
        return Ok(key.to_string());
    }
    Err(ExtractionError::UnresolvedPerson(
        proposal.subject_name.clone(),
    ))
}

fn resolved_key(person_keys: &[(String, String)], name: &str) -> Result<String, ExtractionError> {
    person_keys
        .iter()
        .find(|(candidate, _)| candidate == name)
        .map(|(_, key)| key.clone())
        .ok_or_else(|| ExtractionError::UnresolvedPerson(name.to_string()))
}
