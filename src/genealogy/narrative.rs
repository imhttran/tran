//! Source-linked historical narratives (TRAN-002I).
//!
//! The importer reads a Markdown transcription of the family history and turns
//! each page-and-section block into a verbatim passage with its PDF page range.
//! It never translates, never regenerates OCR, and never edits the source
//! wording: the body is the text between the section heading and the next
//! heading, with paragraph breaks preserved.
//!
//! Passages are separate from structured genealogy facts. Approving a passage
//! publishes narrative text for a person; it never promotes a relationship, a
//! date, or a name to `verified`, and it never touches a member's own profile
//! fields (`biography`, `my_story`, `family_memories`).
//!
//! Person links are deliberately conservative:
//!
//! - a section heading that names exactly one catalog person is a structural
//!   match (`resolved`);
//! - an explicit catalog identifier such as `P007` is a structural match;
//! - any other full-name mention is only a candidate (`ambiguous`) and must be
//!   confirmed by a reviewer before the passage can reach a family profile.
//!
//! A bare given name is never matched, and a name that maps to several people is
//! never resolved by ordering.

use std::collections::{BTreeMap, BTreeSet};

use postgres::{Client, Transaction};

use super::query as genealogy_query;
use super::GenealogyError;
use crate::inventory::sha256_hex;

/// One passage extracted from a narrative source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedPassage {
    /// Stable identity of the source slot (document, page, section).
    pub slot_key: String,
    /// Identity of this exact revision.
    pub passage_key: String,
    /// The section heading as written, without the Markdown markers.
    pub section_title: Option<String>,
    /// The first PDF page the passage appears on.
    pub page_start: i32,
    /// The last PDF page the passage appears on.
    pub page_end: i32,
    /// One-based position within the source.
    pub ordinal: usize,
    /// The verbatim body, with paragraph breaks preserved.
    pub body: String,
    /// The SHA-256 of the body.
    pub content_sha256: String,
}

impl ExtractedPassage {
    /// The revision key for this content in this slot.
    ///
    /// A slot keeps its base key for its first revision. Changed text gets the
    /// content hash appended, so an approved passage is never overwritten.
    fn revision_key(slot_key: &str, content_sha256: &str, first_revision: bool) -> String {
        if first_revision {
            slot_key.to_string()
        } else {
            format!("{slot_key}:{}", &content_sha256[..16])
        }
    }
}

/// The result of parsing one narrative source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NarrativeParse {
    /// Passages in source order.
    pub passages: Vec<ExtractedPassage>,
    /// Section headings with no page marker, so they could not be cited.
    pub unpaged_sections: usize,
}

impl NarrativeParse {
    /// The number of distinct pages covered.
    pub fn page_count(&self) -> usize {
        let pages: BTreeSet<i32> = self
            .passages
            .iter()
            .flat_map(|passage| passage.page_start..=passage.page_end)
            .collect();
        pages.len()
    }
}

/// Parses a Markdown narrative into page-and-section passages.
///
/// Recognised page boundaries are `## Trang PDF N`, `## PDF page N`,
/// `## Page N`, `## Trang N`, and a line that is only `[p. N]`; the prefixed
/// forms are matched case-insensitively. Section headings are `###` or deeper.
pub fn parse_narrative(text: &str, source_ref: &str) -> NarrativeParse {
    let mut parse = NarrativeParse::default();
    let mut state = ParseState::default();

    for line in text.lines() {
        let trimmed = line.trim();

        if let Some(page) = page_marker(trimmed) {
            state.finish_block();
            state.current_page = Some(page);
            // A section continues across a page boundary; only the page changes.
            continue;
        }

        if let Some(title) = section_heading(trimmed) {
            state.finish_block();
            state.close_section(&mut parse, source_ref);
            state.section = Some(SectionBuilder {
                title: Some(title),
                page_start: state.current_page,
                page_end: state.current_page,
                parts: Vec::new(),
            });
            continue;
        }

        if trimmed.is_empty() {
            state.finish_block();
            continue;
        }

        if state.section.is_none() {
            // Front matter and other unpaged prose still count as an unpaged
            // section so the dry run can report it.
            state.section = Some(SectionBuilder {
                title: None,
                page_start: state.current_page,
                page_end: state.current_page,
                parts: Vec::new(),
            });
        }
        state.block.push(trimmed.to_string());
    }

    state.finish_block();
    state.close_section(&mut parse, source_ref);
    parse
}

#[derive(Default)]
struct ParseState {
    current_page: Option<i32>,
    section: Option<SectionBuilder>,
    block: Vec<String>,
    ordinal: usize,
    seen_slots: BTreeMap<String, usize>,
}

impl ParseState {
    /// Closes the paragraph being collected and records the current page.
    fn finish_block(&mut self) {
        if self.block.is_empty() {
            return;
        }
        let paragraph = self.block.join("\n");
        self.block.clear();
        if let Some(section) = self.section.as_mut() {
            section.push(paragraph);
            if let Some(page) = self.current_page {
                section.page_end = Some(page);
            }
        }
    }

    /// Turns the current section into a passage, when it has a page and text.
    fn close_section(&mut self, parse: &mut NarrativeParse, source_ref: &str) {
        let Some(done) = self.section.take() else {
            return;
        };
        let (Some(page_start), Some(page_end)) = (done.page_start, done.page_end) else {
            if !done.parts.is_empty() {
                parse.unpaged_sections += 1;
            }
            return;
        };
        let body = done.body();
        if body.trim().is_empty() {
            return;
        }

        self.ordinal += 1;
        let occurrence = self
            .seen_slots
            .entry(format!(
                "{source_ref}|{page_start}|{}",
                done.title.as_deref().unwrap_or("")
            ))
            .and_modify(|count| *count += 1)
            .or_insert(1);
        let slot_key = sha256_hex(
            format!(
                "{source_ref}|{page_start}|{}|{occurrence}",
                done.title.as_deref().unwrap_or("")
            )
            .as_bytes(),
        );
        let content_sha256 = sha256_hex(body.as_bytes());
        parse.passages.push(ExtractedPassage {
            slot_key: slot_key.clone(),
            passage_key: ExtractedPassage::revision_key(&slot_key, &content_sha256, true),
            section_title: done.title,
            page_start,
            page_end,
            ordinal: self.ordinal,
            body,
            content_sha256,
        });
    }
}

struct SectionBuilder {
    title: Option<String>,
    page_start: Option<i32>,
    page_end: Option<i32>,
    parts: Vec<String>,
}

impl SectionBuilder {
    /// Joins paragraphs with a blank line, preserving the source breaks.
    fn body(&self) -> String {
        self.parts.join("\n\n")
    }

    fn push(&mut self, paragraph: String) {
        if !paragraph.trim().is_empty() {
            self.parts.push(paragraph);
        }
    }
}

fn page_marker(line: &str) -> Option<i32> {
    let stripped = line.trim_start_matches('#').trim();
    let lower = stripped.to_lowercase();
    for prefix in ["trang pdf ", "pdf page ", "page ", "trang ", "p. ", "p "] {
        if let Some(rest) = lower.strip_prefix(prefix) {
            let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
            if let Ok(page) = digits.parse::<i32>() {
                if page >= 1 {
                    return Some(page);
                }
            }
        }
    }
    // An inline `[p. 12]` alone on a line, without a heading marker.
    if let Some(rest) = stripped.strip_prefix('[') {
        let inner = rest.trim_end_matches(']').trim();
        if let Some(digits) = inner.strip_prefix("p. ") {
            if let Ok(page) = digits.trim().parse::<i32>() {
                return Some(page);
            }
        }
    }
    None
}

/// A section heading is `###` or deeper. `##` is reserved for page markers and
/// document-level titles.
fn section_heading(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|ch| *ch == '#').count();
    if hashes < 3 {
        return None;
    }
    let title = line[hashes..].trim();
    if title.is_empty() {
        None
    } else {
        Some(title.to_string())
    }
}

/// A person the linker can consider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkablePerson {
    pub person_id: i64,
    pub person_key: String,
    pub display_name: String,
    pub aliases: Vec<String>,
}

/// One proposed person link for a passage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposedLink {
    pub person_id: i64,
    pub person_key: String,
    pub status: LinkStatus,
    pub match_kind: MatchKind,
    pub evidence_note: String,
}

/// How strongly a passage names a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    /// Structural evidence: a section heading or an explicit identifier.
    Resolved,
    /// A name mention that a reviewer must confirm.
    Ambiguous,
}

impl LinkStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            LinkStatus::Resolved => "resolved",
            LinkStatus::Ambiguous => "ambiguous",
        }
    }
}

/// What produced the link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchKind {
    SectionHeading,
    ExplicitId,
    NameMention,
}

impl MatchKind {
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::SectionHeading => "section_heading",
            MatchKind::ExplicitId => "explicit_id",
            MatchKind::NameMention => "name_mention",
        }
    }
}

/// Links one passage to the catalog people it names.
///
/// A heading or an explicit identifier resolves; a full-name mention in the body
/// only proposes. Given names and partial names are ignored.
pub fn propose_links(passage: &ExtractedPassage, people: &[LinkablePerson]) -> Vec<ProposedLink> {
    let mut proposed: BTreeMap<i64, ProposedLink> = BTreeMap::new();
    let body_tokens = tokens(&passage.body);
    let title_tokens = passage
        .section_title
        .as_deref()
        .map(tokens)
        .unwrap_or_default();

    // 1. Explicit catalog identifiers in the heading or body.
    //
    // A catalog key contains hyphens (`ce-0001-0123…`), so it must be matched as
    // a token sequence rather than as one token.
    for person in people {
        if matches_key(&body_tokens, &person.person_key)
            || matches_key(&title_tokens, &person.person_key)
        {
            proposed.insert(
                person.person_id,
                ProposedLink {
                    person_id: person.person_id,
                    person_key: person.person_key.clone(),
                    status: LinkStatus::Resolved,
                    match_kind: MatchKind::ExplicitId,
                    evidence_note: format!("explicit identifier {}", person.person_key),
                },
            );
        }
    }

    // 2. The section heading names one person: structural evidence.
    if let Some(title) = passage.section_title.as_deref() {
        let title_tokens = tokens(title);
        let matches: Vec<&LinkablePerson> = people
            .iter()
            .filter(|person| mentions(&title_tokens, person))
            .collect();
        if matches.len() == 1 {
            let person = matches[0];
            proposed.insert(
                person.person_id,
                ProposedLink {
                    person_id: person.person_id,
                    person_key: person.person_key.clone(),
                    status: LinkStatus::Resolved,
                    match_kind: MatchKind::SectionHeading,
                    evidence_note: "section heading names this person".to_string(),
                },
            );
        } else {
            for person in matches {
                proposed.insert(
                    person.person_id,
                    ProposedLink {
                        person_id: person.person_id,
                        person_key: person.person_key.clone(),
                        status: LinkStatus::Ambiguous,
                        match_kind: MatchKind::SectionHeading,
                        evidence_note: "the section heading matches more than one catalog person"
                            .to_string(),
                    },
                );
            }
        }
    }

    // 3. Full-name mentions in the body are candidates only.
    for person in people {
        if proposed.contains_key(&person.person_id) {
            continue;
        }
        if mentions(&body_tokens, person) {
            proposed.insert(
                person.person_id,
                ProposedLink {
                    person_id: person.person_id,
                    person_key: person.person_key.clone(),
                    status: LinkStatus::Ambiguous,
                    match_kind: MatchKind::NameMention,
                    evidence_note: "full-name mention; identity not confirmed by the source"
                        .to_string(),
                },
            );
        }
    }

    proposed.into_values().collect()
}

/// Whether the tokens name this person by display name or by a multi-word alias.
fn mentions(tokens_in_text: &[String], person: &LinkablePerson) -> bool {
    // A one-word name is not an identity: given names repeat across generations
    // in this corpus, so only a full name or a multi-word alias is considered.
    let mut names: Vec<String> = Vec::new();
    if person.display_name.split_whitespace().count() >= 2 {
        names.push(person.display_name.clone());
    }
    names.extend(
        person
            .aliases
            .iter()
            .filter(|alias| alias.split_whitespace().count() >= 2)
            .cloned(),
    );
    names
        .iter()
        .any(|name| contains_sequence(tokens_in_text, &tokens(name)))
}

/// Diacritic-insensitive lowercase word tokens.
pub fn tokens(text: &str) -> Vec<String> {
    use unicode_normalization::UnicodeNormalization;
    text.nfd()
        .filter(|ch| !matches!(*ch, '\u{0300}'..='\u{036f}'))
        .collect::<String>()
        .to_lowercase()
        .replace('đ', "d")
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn contains_sequence(haystack: &[String], needle: &[String]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn matches_key(tokens_in_text: &[String], key: &str) -> bool {
    let key_tokens = tokens(key);
    !key_tokens.is_empty() && contains_sequence(tokens_in_text, &key_tokens)
}

// ---------------------------------------------------------------------------
// Import
// ---------------------------------------------------------------------------

/// Counters for one narrative import.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub inserted: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub revisions: usize,
    pub links_inserted: usize,
    pub links_kept: usize,
}

/// What an import should do with one source slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportDecision {
    /// No row exists for this slot.
    Insert,
    /// A pending row exists and the text changed; refresh it in place.
    Update,
    /// An identical revision already exists.
    Unchanged,
    /// An approved or rejected revision exists and the text changed; add a
    /// pending revision and leave the reviewed wording alone.
    Revision,
}

/// Decides how to handle a slot given the existing rows.
pub fn decide(
    existing: &[(String, String, String)], // (passage_key, content_sha256, review_status)
    content_sha256: &str,
) -> ImportDecision {
    if existing.iter().any(|(_, sha, _)| sha == content_sha256) {
        return ImportDecision::Unchanged;
    }
    if existing.iter().any(|(_, _, status)| status == "pending") {
        return ImportDecision::Update;
    }
    if existing.is_empty() {
        ImportDecision::Insert
    } else {
        ImportDecision::Revision
    }
}

/// Imports one narrative source inside a transaction.
pub fn import_narrative(
    tx: &mut Transaction<'_>,
    document_ref: &str,
    sha256: &str,
    source_kind: &str,
    language: &str,
    parse: &NarrativeParse,
    people: &[LinkablePerson],
) -> Result<ImportReport, GenealogyError> {
    let document_id: i64 = tx
        .query_one(
            "INSERT INTO source_document (relative_path, sha256, source_kind, language)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (relative_path) DO UPDATE
                SET sha256 = EXCLUDED.sha256,
                    source_kind = EXCLUDED.source_kind,
                    language = EXCLUDED.language
             RETURNING source_document_id",
            &[&document_ref, &sha256, &source_kind, &language],
        )
        .map_err(GenealogyError::Query)?
        .get(0);

    let mut report = ImportReport::default();

    for passage in &parse.passages {
        let rows = tx
            .query(
                "SELECT passage_key, content_sha256, review_status
                   FROM source_narrative_passage
                  WHERE slot_key = $1",
                &[&passage.slot_key],
            )
            .map_err(GenealogyError::Query)?;
        let existing: Vec<(String, String, String)> = rows
            .iter()
            .map(|row| (row.get(0), row.get(1), row.get(2)))
            .collect();

        let decision = decide(&existing, &passage.content_sha256);
        // `ordinal` is an INTEGER column; the driver does not coerce i64 here.
        let ordinal = passage.ordinal as i32;
        let passage_id = match decision {
            ImportDecision::Unchanged => {
                report.unchanged += 1;
                let key: String = existing
                    .iter()
                    .find(|(_, sha, _)| sha == &passage.content_sha256)
                    .map(|(key, _, _)| key.clone())
                    .unwrap_or_else(|| passage.passage_key.clone());
                passage_id_for_key(tx, &key)?
            }
            ImportDecision::Update => {
                report.updated += 1;
                // The text changed, so the revision key must follow it: a key
                // derived from an earlier body would collide if that body came
                // back later. The slot's first row keeps the base key.
                let id: i64 = tx
                    .query_one(
                        "UPDATE source_narrative_passage
                            SET body = $2, content_sha256 = $3, page_end = $4,
                                ordinal = $5, section_title = $6,
                                passage_key = CASE
                                    WHEN passage_key = slot_key THEN slot_key
                                    ELSE slot_key || ':' || left($3, 16)
                                END,
                                updated_at = now()
                          WHERE slot_key = $1 AND review_status = 'pending'
                          RETURNING passage_id",
                        &[
                            &passage.slot_key,
                            &passage.body,
                            &passage.content_sha256,
                            &passage.page_end,
                            &ordinal,
                            &passage.section_title,
                        ],
                    )
                    .map_err(GenealogyError::Query)?
                    .get(0);
                // Link decisions were made about the previous wording, so they
                // are re-derived from the replacement text below. The audit
                // events that recorded them are append-only and remain.
                tx.execute(
                    "DELETE FROM source_narrative_link WHERE passage_id = $1",
                    &[&id],
                )
                .map_err(GenealogyError::Query)?;
                id
            }
            ImportDecision::Insert | ImportDecision::Revision => {
                if decision == ImportDecision::Insert {
                    report.inserted += 1;
                } else {
                    report.revisions += 1;
                }
                let key = if existing.is_empty() {
                    passage.passage_key.clone()
                } else {
                    ExtractedPassage::revision_key(
                        &passage.slot_key,
                        &passage.content_sha256,
                        false,
                    )
                };
                tx.query_one(
                    "INSERT INTO source_narrative_passage
                        (slot_key, passage_key, source_document_id, language,
                         section_title, page_start, page_end, ordinal, body,
                         content_sha256)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                     RETURNING passage_id",
                    &[
                        &passage.slot_key,
                        &key,
                        &document_id,
                        &language,
                        &passage.section_title,
                        &passage.page_start,
                        &passage.page_end,
                        &ordinal,
                        &passage.body,
                        &passage.content_sha256,
                    ],
                )
                .map_err(GenealogyError::Query)?
                .get(0)
            }
        };

        for link in propose_links(passage, people) {
            // A reviewed link is never downgraded by a later import.
            let inserted = tx
                .execute(
                    "INSERT INTO source_narrative_link
                        (passage_id, person_id, link_status, match_kind, evidence_note)
                     VALUES ($1, $2, $3, $4, $5)
                     ON CONFLICT (passage_id, person_id) DO NOTHING",
                    &[
                        &passage_id,
                        &link.person_id,
                        &link.status.as_str(),
                        &link.match_kind.as_str(),
                        &link.evidence_note,
                    ],
                )
                .map_err(GenealogyError::Query)?;
            if inserted > 0 {
                report.links_inserted += 1;
            } else {
                report.links_kept += 1;
            }
        }
    }

    Ok(report)
}

fn passage_id_for_key(tx: &mut Transaction<'_>, key: &str) -> Result<i64, GenealogyError> {
    tx.query_one(
        "SELECT passage_id FROM source_narrative_passage WHERE passage_key = $1",
        &[&key],
    )
    .map_err(GenealogyError::Query)
    .map(|row| row.get(0))
}

/// Loads the catalog people a linker can consider.
pub fn linkable_people(client: &mut Client) -> Result<Vec<LinkablePerson>, GenealogyError> {
    let rows = client
        .query(
            "SELECT p.person_id, p.person_key, p.display_name,
                    COALESCE(ARRAY_AGG(v.name_variant ORDER BY v.name_variant)
                             FILTER (WHERE v.name_variant IS NOT NULL), ARRAY[]::text[])
               FROM person p
               LEFT JOIN person_name_variant v ON v.person_id = p.person_id
              GROUP BY p.person_id, p.person_key, p.display_name
              ORDER BY p.person_key",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| LinkablePerson {
            person_id: row.get(0),
            person_key: row.get(1),
            display_name: row.get(2),
            aliases: row.get(3),
        })
        .collect())
}

// ---------------------------------------------------------------------------
// Review
// ---------------------------------------------------------------------------

/// One passage review decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassageDecision {
    Pending,
    Approved,
    Rejected,
}

impl PassageDecision {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => PassageDecision::Pending,
            "approved" | "approve" => PassageDecision::Approved,
            "rejected" | "reject" => PassageDecision::Rejected,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PassageDecision::Pending => "pending",
            PassageDecision::Approved => "approved",
            PassageDecision::Rejected => "rejected",
        }
    }
}

/// One passage visibility decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassageVisibility {
    AdminOnly,
    Family,
}

impl PassageVisibility {
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "admin_only" | "admin" => PassageVisibility::AdminOnly,
            "family" => PassageVisibility::Family,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PassageVisibility::AdminOnly => "admin_only",
            PassageVisibility::Family => "family",
        }
    }
}

/// Reviews one passage: its text and its audience.
///
/// This wrapper keeps the CLI behaviour: no staleness expectation.
pub fn review_passage(
    client: &mut Client,
    passage_id: i64,
    decision: PassageDecision,
    visibility: Option<PassageVisibility>,
    reviewer: &str,
    note: Option<&str>,
) -> Result<(), GenealogyError> {
    review_passage_checked(
        client, passage_id, decision, visibility, reviewer, note, None, None,
    )
}

/// Reviews one passage with an optional staleness guard.
///
/// The row is locked in the same transaction, then `expected_status` and
/// `expected_version` are compared with the locked row. `expected_version` is
/// the passage's `updated_at::text`, exactly as the detail read returns it. A
/// mismatch is a [`GenealogyError::Review`] and nothing is written.
#[allow(clippy::too_many_arguments)]
pub fn review_passage_checked(
    client: &mut Client,
    passage_id: i64,
    decision: PassageDecision,
    visibility: Option<PassageVisibility>,
    reviewer: &str,
    note: Option<&str>,
    expected_status: Option<&str>,
    expected_version: Option<&str>,
) -> Result<(), GenealogyError> {
    if reviewer.trim().is_empty() {
        return Err(GenealogyError::Review("reviewer is required".into()));
    }

    let mut tx = client.transaction().map_err(GenealogyError::Query)?;
    let current = tx
        .query_opt(
            "SELECT review_status, updated_at::text
               FROM source_narrative_passage
              WHERE passage_id = $1 FOR UPDATE",
            &[&passage_id],
        )
        .map_err(GenealogyError::Query)?;
    let Some(row) = current else {
        return Err(GenealogyError::Review(
            "no such narrative passage".to_string(),
        ));
    };
    let from_status: String = row.get(0);
    let current_version: String = row.get(1);

    if let Some(expected) = expected_status {
        if expected != from_status {
            return Err(GenealogyError::Review(
                "the passage review status changed since it was opened".to_string(),
            ));
        }
    }
    if let Some(expected) = expected_version {
        if expected != current_version {
            return Err(GenealogyError::Review(
                "the passage changed since it was opened".to_string(),
            ));
        }
    }

    // Publishing a passage under a person's name needs at least one link the
    // archive has accepted. The check runs inside this transaction, after the
    // passage row is locked, so a concurrent link rejection cannot slip through.
    if decision == PassageDecision::Approved && visibility == Some(PassageVisibility::Family) {
        let accepted: i64 = tx
            .query_one(
                "SELECT count(*) FROM source_narrative_link
                  WHERE passage_id = $1 AND link_status IN ('resolved', 'confirmed')",
                &[&passage_id],
            )
            .map_err(GenealogyError::Query)?
            .get(0);
        if accepted == 0 {
            return Err(GenealogyError::Review(
                "a family-visible passage needs at least one resolved or confirmed person link"
                    .into(),
            ));
        }
    }

    tx.execute(
        "UPDATE source_narrative_passage
            SET review_status = $2,
                visibility = COALESCE($3, visibility),
                reviewed_by = $4,
                reviewed_at = now(),
                review_note = $5,
                updated_at = now()
          WHERE passage_id = $1",
        &[
            &passage_id,
            &decision.as_str(),
            &visibility.map(PassageVisibility::as_str),
            &reviewer,
            &note,
        ],
    )
    .map_err(GenealogyError::Query)?;

    tx.execute(
        "INSERT INTO source_narrative_review_event
            (passage_id, actor, action, from_status, to_status, visibility, note)
         VALUES ($1, $2, 'passage_review', $3, $4, $5, $6)",
        &[
            &passage_id,
            &reviewer,
            &from_status,
            &decision.as_str(),
            &visibility.map(PassageVisibility::as_str),
            &note,
        ],
    )
    .map_err(GenealogyError::Query)?;

    tx.commit().map_err(GenealogyError::Query)?;
    Ok(())
}

/// Confirms or rejects one person link.
///
/// This wrapper keeps the CLI behaviour: no staleness expectation.
pub fn review_link(
    client: &mut Client,
    passage_id: i64,
    person_key: &str,
    accept: bool,
    reviewer: &str,
    note: Option<&str>,
) -> Result<(), GenealogyError> {
    review_link_checked(client, passage_id, person_key, accept, reviewer, note, None)
}

/// Confirms or rejects one person link with an optional staleness guard.
///
/// The link row is locked in the same transaction, then `expected_link_status`
/// is compared with the locked row. A mismatch is a [`GenealogyError::Review`]
/// and nothing is written. Link review never changes the passage status.
pub fn review_link_checked(
    client: &mut Client,
    passage_id: i64,
    person_key: &str,
    accept: bool,
    reviewer: &str,
    note: Option<&str>,
    expected_link_status: Option<&str>,
) -> Result<(), GenealogyError> {
    if reviewer.trim().is_empty() {
        return Err(GenealogyError::Review("reviewer is required".into()));
    }
    let mut tx = client.transaction().map_err(GenealogyError::Query)?;
    let person_id: Option<i64> = tx
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key],
        )
        .map_err(GenealogyError::Query)?
        .map(|row| row.get(0));
    let Some(person_id) = person_id else {
        return Err(GenealogyError::Review("no such person".to_string()));
    };
    let from_status: Option<String> = tx
        .query_opt(
            "SELECT link_status FROM source_narrative_link
              WHERE passage_id = $1 AND person_id = $2 FOR UPDATE",
            &[&passage_id, &person_id],
        )
        .map_err(GenealogyError::Query)?
        .map(|row| row.get(0));
    let Some(from_status) = from_status else {
        return Err(GenealogyError::Review("no such narrative link".to_string()));
    };

    if let Some(expected) = expected_link_status {
        if expected != from_status {
            return Err(GenealogyError::Review(
                "the person link changed since it was opened".to_string(),
            ));
        }
    }

    let to_status = if accept { "confirmed" } else { "rejected" };
    tx.execute(
        "UPDATE source_narrative_link
            SET link_status = $3, reviewed_by = $4, reviewed_at = now()
          WHERE passage_id = $1 AND person_id = $2",
        &[&passage_id, &person_id, &to_status, &reviewer],
    )
    .map_err(GenealogyError::Query)?;
    tx.execute(
        "INSERT INTO source_narrative_review_event
            (passage_id, person_id, actor, action, from_status, to_status, note)
         VALUES ($1, $2, $3, 'link_review', $4, $5, $6)",
        &[
            &passage_id,
            &person_id,
            &reviewer,
            &from_status,
            &to_status,
            &note,
        ],
    )
    .map_err(GenealogyError::Query)?;
    tx.commit().map_err(GenealogyError::Query)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// One approved, family-visible passage for a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamilyPassage {
    pub passage_id: i64,
    pub language: String,
    /// The source document's relative path, kept beside the wording so a
    /// citation can name the document and not only its page.
    pub document_ref: String,
    pub section_title: Option<String>,
    pub page_start: i32,
    pub page_end: i32,
    pub body: String,
}

/// The one predicate for published family history.
const PUBLISHED_HISTORY_CTE: &str = "\
WITH published_history AS (
    SELECT l.person_id, p.passage_id
      FROM source_narrative_passage p
      JOIN source_narrative_link l ON l.passage_id = p.passage_id
     WHERE l.link_status IN ('resolved', 'confirmed')
       AND p.review_status = 'approved'
       AND p.visibility = 'family'
       AND p.passage_id = (
           SELECT newest.passage_id
             FROM source_narrative_passage newest
            WHERE newest.slot_key = p.slot_key
              AND newest.review_status = 'approved'
            ORDER BY newest.reviewed_at DESC NULLS LAST, newest.passage_id DESC
            LIMIT 1
       )
)";

/// Reads the approved archive history for one person.
pub fn family_history(
    tx: &mut Transaction<'_>,
    person_id: i64,
) -> Result<Vec<FamilyPassage>, GenealogyError> {
    let sql = format!(
        "{PUBLISHED_HISTORY_CTE}
         SELECT p.passage_id, p.language, sd.relative_path, p.section_title,
                p.page_start, p.page_end, p.body
           FROM published_history h
           JOIN source_narrative_passage p ON p.passage_id = h.passage_id
           JOIN source_document sd ON sd.source_document_id = p.source_document_id
          WHERE h.person_id = $1
          ORDER BY p.page_start, p.page_end, p.passage_id"
    );
    let rows = tx
        .query(&sql, &[&person_id])
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| FamilyPassage {
            passage_id: row.get(0),
            language: row.get(1),
            document_ref: row.get(2),
            section_title: row.get(3),
            page_start: row.get(4),
            page_end: row.get(5),
            body: row.get(6),
        })
        .collect())
}

/// Counts published archive passages per person key.
pub fn published_history_counts(
    client: &mut Client,
    person_keys: &[String],
) -> Result<BTreeMap<String, i64>, GenealogyError> {
    if person_keys.is_empty() {
        return Ok(BTreeMap::new());
    }
    let sql = format!(
        "{PUBLISHED_HISTORY_CTE}
         SELECT pe.person_key, count(*)::bigint
           FROM published_history h
           JOIN person pe ON pe.person_id = h.person_id
          WHERE pe.person_key = ANY($1::text[])
          GROUP BY pe.person_key"
    );
    let rows = client
        .query(&sql, &[&person_keys])
        .map_err(GenealogyError::Query)?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

// ---------------------------------------------------------------------------
// Admin review read models (PH-202)
// ---------------------------------------------------------------------------

/// One person link candidate on a passage, for the admin review screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewLink {
    /// Stable person key.
    pub person_key: String,
    /// Display name from the catalog.
    pub display_name: String,
    /// `resolved`, `ambiguous`, `confirmed`, or `rejected`.
    pub link_status: String,
    /// `section_heading`, `explicit_id`, or `name_mention`.
    pub match_kind: String,
    /// The proposal's evidence note, when recorded.
    pub evidence_note: Option<String>,
    /// The reviewer who decided the link, when one did.
    pub reviewed_by: Option<String>,
    /// The decision timestamp as PostgreSQL text, when one exists.
    pub reviewed_at: Option<String>,
}

/// One passage in the admin review read models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPassage {
    pub passage_id: i64,
    pub language: String,
    pub section_title: Option<String>,
    /// The source document's relative path.
    pub document_ref: String,
    pub page_start: i32,
    pub page_end: i32,
    pub review_status: String,
    pub visibility: String,
    /// The complete verbatim body, paragraph breaks preserved.
    pub body: String,
    /// A short, single-line excerpt for list rows.
    pub excerpt: String,
    /// `updated_at::text`, exactly as PostgreSQL returns it. This is the
    /// passage version the review conflict check compares.
    pub updated_at: String,
}

/// Genealogy-only context for one linked person, never private identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPersonContext {
    pub person_key: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    /// A verified birth label, when one is recorded.
    pub birth: Option<String>,
    /// A verified death label, when one is recorded.
    pub death: Option<String>,
    /// Recorded parent names.
    pub parents: Vec<String>,
    /// Recorded spouse names.
    pub spouses: Vec<String>,
    /// Recorded child names.
    pub children: Vec<String>,
}

/// One append-only review event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEvent {
    pub event_id: i64,
    /// The linked person, for a link review; `None` for a passage review.
    pub person_key: Option<String>,
    pub actor: String,
    pub action: String,
    pub from_status: Option<String>,
    pub to_status: String,
    pub visibility: Option<String>,
    pub note: Option<String>,
    pub created_at: String,
}

/// The complete admin review of one passage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewPassageDetail {
    pub passage: ReviewPassage,
    /// Every link candidate with its status.
    pub links: Vec<ReviewLink>,
    /// Genealogy context for every linked person.
    pub people: Vec<ReviewPersonContext>,
    /// The append-only review events for this passage, oldest first.
    pub events: Vec<ReviewEvent>,
}

/// The applied admin review filters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewFilters {
    /// `pending`, `approved`, `rejected`, or `all`.
    pub status: String,
    /// `vi`, `en`, or `None` for every language.
    pub language: Option<String>,
    /// An exact person key, when filtering by person.
    pub person_key: Option<String>,
    /// Only passages with no link candidate at all.
    pub unmatched_only: bool,
    /// `all`, `needs_confirmation`, `no_accepted`, `has_accepted`, or
    /// `no_candidates`.
    pub link: String,
    /// `admin_only`, `family`, or `None` for every visibility.
    pub visibility: Option<String>,
    /// Exact source document relative path.
    pub document_ref: Option<String>,
    /// Inclusive PDF page overlap start.
    pub page_from: Option<i32>,
    /// Inclusive PDF page overlap end.
    pub page_to: Option<i32>,
    /// Accent-insensitive search over the section title and passage body.
    pub search: Option<String>,
    /// Zero-based row offset. The API applies this after filtering.
    pub offset: i64,
    /// The bounded row limit that was applied.
    pub limit: i64,
}

impl Default for ReviewFilters {
    fn default() -> Self {
        Self {
            status: "pending".to_string(),
            language: None,
            person_key: None,
            unmatched_only: false,
            link: "all".to_string(),
            visibility: None,
            document_ref: None,
            page_from: None,
            page_to: None,
            search: None,
            offset: 0,
            limit: REVIEW_QUEUE_DEFAULT_LIMIT,
        }
    }
}

/// One passage and its link candidates in the admin review queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewQueueEntry {
    pub passage: ReviewPassage,
    pub links: Vec<ReviewLink>,
}

/// One page of the admin review queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewQueue {
    pub entries: Vec<ReviewQueueEntry>,
    /// True when more rows match than the requested page returned.
    pub truncated: bool,
    /// The number of rows that match every filter, before pagination.
    pub total: i64,
    /// Source documents that have at least one narrative passage.
    pub documents: Vec<String>,
    pub filters: ReviewFilters,
}

/// The default and maximum queue limit.
pub const REVIEW_QUEUE_DEFAULT_LIMIT: i64 = 100;
/// The largest accepted queue limit.
pub const REVIEW_QUEUE_MAX_LIMIT: i64 = 200;

/// The short single-line excerpt for a list row.
fn excerpt_of(body: &str) -> String {
    const MAX: usize = 160;
    let single = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if single.chars().count() > MAX {
        let truncated: String = single.chars().take(MAX).collect();
        format!("{truncated}…")
    } else {
        single
    }
}

fn review_passage_from_row(row: &postgres::Row) -> ReviewPassage {
    let body: String = row.get(8);
    ReviewPassage {
        passage_id: row.get(0),
        language: row.get(1),
        section_title: row.get(2),
        document_ref: row.get(3),
        page_start: row.get(4),
        page_end: row.get(5),
        review_status: row.get(6),
        visibility: row.get(7),
        excerpt: excerpt_of(&body),
        body,
        updated_at: row.get(9),
    }
}

/// Lists passages for the admin review queue.
///
/// All link candidates, including rejected links, are returned with each
/// passage. Filters are applied in SQL before pagination: status, language,
/// exact person key, unmatched-only, link state, visibility, source document,
/// an inclusive PDF page overlap, and an accent-insensitive search over linked
/// person names, section titles, and passage text. Ordering is deterministic and
/// both the offset and limit are bounded.
pub fn review_queue(
    client: &mut Client,
    filters: &ReviewFilters,
) -> Result<ReviewQueue, GenealogyError> {
    let applied = ReviewFilters {
        status: filters.status.clone(),
        language: filters.language.clone(),
        person_key: filters.person_key.clone(),
        unmatched_only: filters.unmatched_only,
        link: filters.link.clone(),
        visibility: filters.visibility.clone(),
        document_ref: filters.document_ref.clone(),
        page_from: filters.page_from,
        page_to: filters.page_to,
        search: filters
            .search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string),
        offset: filters.offset.max(0),
        limit: filters.limit.clamp(1, REVIEW_QUEUE_MAX_LIMIT),
    };
    let documents = narrative_documents(client)?;
    let person_id: Option<i64> = match applied.person_key.as_deref() {
        Some(key) => client
            .query_opt(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&key],
            )
            .map_err(GenealogyError::Query)?
            .map(|row| row.get(0)),
        None => None,
    };
    // A supplied person key that does not exist is an empty result, not an
    // absent filter. Passing NULL to the SQL below would silently remove the
    // person filter and return every passage.
    if applied.person_key.is_some() && person_id.is_none() {
        return Ok(ReviewQueue {
            entries: Vec::new(),
            truncated: false,
            total: 0,
            documents,
            filters: applied,
        });
    }
    let status = match applied.status.as_str() {
        "all" => None,
        other => Some(other.to_string()),
    };
    let predicate = r#"
        ($1::text IS NULL OR p.review_status = $1)
        AND ($2::text IS NULL OR p.language = $2)
        AND ($3::bigint IS NULL OR EXISTS (
              SELECT 1 FROM source_narrative_link l
               WHERE l.passage_id = p.passage_id AND l.person_id = $3))
        AND (NOT $4::boolean OR NOT EXISTS (
              SELECT 1 FROM source_narrative_link l
               WHERE l.passage_id = p.passage_id))
        AND (
            $5::text = 'all'
            OR ($5::text = 'needs_confirmation' AND EXISTS (
                  SELECT 1 FROM source_narrative_link l
                   WHERE l.passage_id = p.passage_id AND l.link_status = 'ambiguous'))
            OR ($5::text = 'no_accepted' AND NOT EXISTS (
                  SELECT 1 FROM source_narrative_link l
                   WHERE l.passage_id = p.passage_id
                     AND l.link_status IN ('resolved', 'confirmed')))
            OR ($5::text = 'has_accepted' AND EXISTS (
                  SELECT 1 FROM source_narrative_link l
                   WHERE l.passage_id = p.passage_id
                     AND l.link_status IN ('resolved', 'confirmed')))
            OR ($5::text = 'no_candidates' AND NOT EXISTS (
                  SELECT 1 FROM source_narrative_link l
                   WHERE l.passage_id = p.passage_id))
        )
        AND ($6::text IS NULL OR p.visibility = $6)
        AND ($7::text IS NULL OR sd.relative_path = $7)
        AND ($8::int IS NULL OR p.page_end >= $8)
        AND ($9::int IS NULL OR p.page_start <= $9)
        AND (
            $10::text IS NULL
            OR narrative_search_normalize(coalesce(p.section_title, ''))
                 LIKE '%' || narrative_search_normalize($10) || '%'
            OR narrative_search_normalize(p.body)
                 LIKE '%' || narrative_search_normalize($10) || '%'
        )
    "#;
    let count_sql = format!(
        "SELECT count(*)::bigint
           FROM source_narrative_passage p
           JOIN source_document sd ON sd.source_document_id = p.source_document_id
          WHERE {predicate}"
    );
    let mut params: Vec<&(dyn postgres::types::ToSql + Sync)> = vec![
        &status,
        &applied.language,
        &person_id,
        &applied.unmatched_only,
        &applied.link,
        &applied.visibility,
        &applied.document_ref,
        &applied.page_from,
        &applied.page_to,
        &applied.search,
    ];
    let total: i64 = client
        .query_one(&count_sql, &params)
        .map_err(GenealogyError::Query)?
        .get(0);
    let page_limit = applied.limit + 1;
    let offset = applied.offset;
    params.push(&page_limit);
    params.push(&offset);
    let sql = format!(
        "SELECT p.passage_id, p.language, p.section_title, sd.relative_path,
                p.page_start, p.page_end, p.review_status, p.visibility, p.body,
                p.updated_at::text
           FROM source_narrative_passage p
           JOIN source_document sd ON sd.source_document_id = p.source_document_id
          WHERE {predicate}
          ORDER BY p.review_status, p.page_start, p.page_end, p.passage_id
          LIMIT $11 OFFSET $12"
    );
    let rows = client.query(&sql, &params).map_err(GenealogyError::Query)?;
    let truncated = rows.len() as i64 > applied.limit;
    let rows = &rows[..rows.len().min(applied.limit as usize)];
    let passages: Vec<ReviewPassage> = rows.iter().map(review_passage_from_row).collect();
    let ids: Vec<i64> = passages.iter().map(|passage| passage.passage_id).collect();
    let mut links = review_links_for(client, &ids)?;
    let entries = passages
        .into_iter()
        .map(|passage| {
            let candidates = links.remove(&passage.passage_id).unwrap_or_default();
            ReviewQueueEntry {
                passage,
                links: candidates,
            }
        })
        .collect();
    Ok(ReviewQueue {
        entries,
        truncated,
        total,
        documents,
        filters: applied,
    })
}

/// Distinct source documents that have narrative passages, for the queue filter.
fn narrative_documents(client: &mut Client) -> Result<Vec<String>, GenealogyError> {
    let rows = client
        .query(
            "SELECT DISTINCT sd.relative_path
               FROM source_document sd
               JOIN source_narrative_passage p
                 ON p.source_document_id = sd.source_document_id
              ORDER BY sd.relative_path",
            &[],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// Reads every link candidate for one passage with its display name.
fn review_links(client: &mut Client, passage_id: i64) -> Result<Vec<ReviewLink>, GenealogyError> {
    Ok(review_links_for(client, &[passage_id])?
        .remove(&passage_id)
        .unwrap_or_default())
}

/// Reads every link candidate for a set of passages, grouped by passage id.
fn review_links_for(
    client: &mut Client,
    passage_ids: &[i64],
) -> Result<BTreeMap<i64, Vec<ReviewLink>>, GenealogyError> {
    if passage_ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows = client
        .query(
            "SELECT l.passage_id, pe.person_key, pe.display_name, l.link_status, l.match_kind,
                    l.evidence_note, l.reviewed_by, l.reviewed_at::text
               FROM source_narrative_link l
               JOIN person pe ON pe.person_id = l.person_id
              WHERE l.passage_id = ANY($1::bigint[])
              ORDER BY l.passage_id, pe.person_key",
            &[&passage_ids],
        )
        .map_err(GenealogyError::Query)?;
    let mut grouped: BTreeMap<i64, Vec<ReviewLink>> = BTreeMap::new();
    for row in &rows {
        grouped.entry(row.get(0)).or_default().push(ReviewLink {
            person_key: row.get(1),
            display_name: row.get(2),
            link_status: row.get(3),
            match_kind: row.get(4),
            evidence_note: row.get(5),
            reviewed_by: row.get(6),
            reviewed_at: row.get(7),
        });
    }
    Ok(grouped)
}

/// Builds genealogy-only context from the existing person reads.
///
/// Only [`genealogy_query::lookup_person`] and
/// [`genealogy_query::lookup_relationship`] are used: no fact is invented, and
/// no private identity data is read.
pub fn review_person_context(
    client: &mut Client,
    person_key: &str,
) -> Result<Option<ReviewPersonContext>, GenealogyError> {
    // The relationship reads and the person reads both borrow the client, so
    // the person is loaded first, then the family names are resolved.
    let Some(person) = genealogy_query::lookup_person(client, person_key)? else {
        return Ok(None);
    };
    let display_name = person
        .display_name
        .clone()
        .unwrap_or_else(|| person_key.to_string());
    let aliases: Vec<String> = client
        .query(
            "SELECT name_variant FROM person_name_variant WHERE person_id = (
                SELECT person_id FROM person WHERE person_key = $1
             ) ORDER BY name_variant",
            &[&person_key],
        )
        .map_err(GenealogyError::Query)?
        .iter()
        .map(|row| row.get(0))
        .collect();
    let mut birth: Option<String> = None;
    let mut death: Option<String> = None;
    for assertion in &person.assertions {
        if assertion.review_status != "verified" {
            continue;
        }
        let Some(date) = assertion.date.as_ref() else {
            continue;
        };
        let label = match date.qualifier.as_str() {
            "exact" => date.start.clone(),
            other => format!("{other} {}", date.start),
        };
        match assertion.kind.as_str() {
            "birth" | "birth_date" if birth.is_none() => birth = Some(label),
            "death" | "death_date" if death.is_none() => death = Some(label),
            _ => {}
        }
    }

    let parents = related_names(client, person_key, "parent")?;
    let spouses = related_names(client, person_key, "spouse")?;
    let children = related_names(client, person_key, "child")?;

    Ok(Some(ReviewPersonContext {
        person_key: person_key.to_string(),
        display_name,
        aliases,
        birth,
        death,
        parents,
        spouses,
        children,
    }))
}

/// Reads recorded parent/spouse/child display names from the relationship links.
///
/// This is a genealogy read only; it never creates or promotes a relationship.
fn related_names(
    client: &mut Client,
    person_key: &str,
    relation: &str,
) -> Result<Vec<String>, GenealogyError> {
    let sql = match relation {
        "parent" => {
            "SELECT p.display_name FROM parent_child_link pcl
                       JOIN person c ON c.person_id = pcl.child_id
                       JOIN person p ON p.person_id = pcl.parent_id
                      WHERE c.person_key = $1 ORDER BY p.display_name"
        }
        "child" => {
            "SELECT c.display_name FROM parent_child_link pcl
                      JOIN person p ON p.person_id = pcl.parent_id
                      JOIN person c ON c.person_id = pcl.child_id
                     WHERE p.person_key = $1 ORDER BY c.display_name"
        }
        "spouse" => {
            "SELECT s.display_name FROM spouse_link sl
                       JOIN person p ON p.person_id = sl.person_id
                       JOIN person s ON s.person_id = sl.spouse_id
                      WHERE p.person_key = $1 ORDER BY s.display_name"
        }
        _ => return Ok(Vec::new()),
    };
    let rows = client
        .query(sql, &[&person_key])
        .map_err(GenealogyError::Query)?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// Searches the catalog for the review screen's person selector.
///
/// The query is matched case- and accent-insensitively, including `đ` to `d`,
/// against the person key, display name, recorded name variants, and assertions.
/// The returned contexts preserve the original names and add only recorded
/// genealogy context. A query is never treated as an inferred identity: callers
/// must select a returned person key explicitly.
pub fn search_review_people(
    client: &mut Client,
    query: &str,
    limit: i64,
) -> Result<Vec<ReviewPersonContext>, GenealogyError> {
    let query = query.trim();
    if query.is_empty() {
        return Ok(Vec::new());
    }
    let limit = limit.clamp(1, 20);
    let current = super::current::CURRENT_ASSERTION;
    let sql = format!(
        "SELECT p.person_key
           FROM person p
          WHERE narrative_search_normalize(p.person_key)
                  LIKE '%' || narrative_search_normalize($1) || '%'
             OR narrative_search_normalize(p.display_name)
                  LIKE '%' || narrative_search_normalize($1) || '%'
             OR EXISTS (
                  SELECT 1 FROM person_name_variant v
                   WHERE v.person_id = p.person_id
                     AND narrative_search_normalize(v.name_variant)
                       LIKE '%' || narrative_search_normalize($1) || '%'
                )
             OR EXISTS (
                  SELECT 1 FROM assertion a
                   WHERE a.person_id = p.person_id
                     AND a.assertion_kind = 'name_variant'
                     AND a.value_text IS NOT NULL
                     AND ({current})
                     AND narrative_search_normalize(a.value_text)
                       LIKE '%' || narrative_search_normalize($1) || '%'
                )
          ORDER BY p.display_name, p.person_key
          LIMIT $2"
    );
    let rows = client
        .query(&sql, &[&query, &limit])
        .map_err(GenealogyError::Query)?;
    let mut out = Vec::new();
    for row in rows {
        let key: String = row.get(0);
        if let Some(context) = review_person_context(client, &key)? {
            out.push(context);
        }
    }
    Ok(out)
}

fn review_events(client: &mut Client, passage_id: i64) -> Result<Vec<ReviewEvent>, GenealogyError> {
    let rows = client
        .query(
            "SELECT e.event_id, pe.person_key, e.actor, e.action, e.from_status,
                    e.to_status, e.visibility, e.note, e.created_at::text
               FROM source_narrative_review_event e
               LEFT JOIN person pe ON pe.person_id = e.person_id
              WHERE e.passage_id = $1
              ORDER BY e.event_id",
            &[&passage_id],
        )
        .map_err(GenealogyError::Query)?;
    Ok(rows
        .iter()
        .map(|row| ReviewEvent {
            event_id: row.get(0),
            person_key: row.get(1),
            actor: row.get(2),
            action: row.get(3),
            from_status: row.get(4),
            to_status: row.get(5),
            visibility: row.get(6),
            note: row.get(7),
            created_at: row.get(8),
        })
        .collect())
}

/// Reads the complete admin review detail for one passage.
///
/// A missing passage returns `Ok(None)`. Otherwise the response carries the
/// complete passage body, its source document, every link candidate, genealogy
/// context for every linked person, and the append-only review events.
pub fn review_detail(
    client: &mut Client,
    passage_id: i64,
) -> Result<Option<ReviewPassageDetail>, GenealogyError> {
    let row = client
        .query_opt(
            "SELECT p.passage_id, p.language, p.section_title, sd.relative_path,
                    p.page_start, p.page_end, p.review_status, p.visibility, p.body,
                    p.updated_at::text
               FROM source_narrative_passage p
               JOIN source_document sd ON sd.source_document_id = p.source_document_id
              WHERE p.passage_id = $1",
            &[&passage_id],
        )
        .map_err(GenealogyError::Query)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let passage = review_passage_from_row(&row);
    let links = review_links(client, passage_id)?;
    let mut people = Vec::with_capacity(links.len());
    for link in &links {
        if let Some(context) = review_person_context(client, &link.person_key)? {
            people.push(context);
        }
    }
    let events = review_events(client, passage_id)?;
    Ok(Some(ReviewPassageDetail {
        passage,
        links,
        people,
        events,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# Family history

Front matter without a page.

## Trang PDF 7

### Đời thứ 11: TRẦN TỐI LẠC (1845–1916)

First paragraph.

Second paragraph with Trần Văn Sần named.

## Trang PDF 8

### Đời thứ 12: TRẦN DOÃN KHIÊM (1877–1922)

Khiêm paragraph mentioning Trần Thị Vọng.

## Trang PDF 9

Continuation of the previous section.

## Trang PDF 10

### Đời thứ 13

Another section.
";

    fn parse() -> NarrativeParse {
        parse_narrative(SAMPLE, "synthetic/vi.md")
    }

    #[test]
    fn sections_become_passages_with_page_ranges() {
        let parse = parse();
        assert_eq!(parse.passages.len(), 3);
        assert_eq!(parse.unpaged_sections, 1);
        assert_eq!(
            parse.passages[0].section_title.as_deref(),
            Some("Đời thứ 11: TRẦN TỐI LẠC (1845–1916)")
        );
        assert_eq!(parse.passages[0].page_start, 7);
        assert_eq!(parse.passages[0].page_end, 7);
        assert_eq!(parse.passages[1].page_start, 8);
        assert_eq!(parse.passages[1].page_end, 9);
        assert_eq!(parse.passages[2].page_start, 10);
    }

    #[test]
    fn a_section_continues_across_a_page_boundary() {
        let parse = parse();
        let khiem = &parse.passages[1];
        assert_eq!(khiem.page_start, 8);
        assert_eq!(khiem.page_end, 9, "the continuation extends the range");
        assert!(khiem.body.contains("Continuation"));
    }

    #[test]
    fn paragraph_breaks_and_wording_are_preserved() {
        let parse = parse();
        let lac = &parse.passages[0];
        assert_eq!(
            lac.body,
            "First paragraph.\n\nSecond paragraph with Trần Văn Sần named."
        );
    }

    #[test]
    fn slot_keys_are_stable_and_content_hashes_change_with_text() {
        let first = parse();
        let second = parse();
        assert_eq!(
            first
                .passages
                .iter()
                .map(|p| &p.slot_key)
                .collect::<Vec<_>>(),
            second
                .passages
                .iter()
                .map(|p| &p.slot_key)
                .collect::<Vec<_>>()
        );
        let changed = SAMPLE.replace("First paragraph.", "First paragraph changed.");
        let changed = parse_narrative(&changed, "synthetic/vi.md");
        assert_eq!(first.passages[0].slot_key, changed.passages[0].slot_key);
        assert_ne!(
            first.passages[0].content_sha256,
            changed.passages[0].content_sha256
        );
    }

    #[test]
    fn a_heading_resolves_and_a_body_mention_only_proposes() {
        let parse = parse();
        let people = vec![
            person(1, "P001", "Trần Tối Lạc", &[]),
            person(2, "P002", "Trần Văn Sần", &[]),
            person(3, "P003", "Trần Thị Vọng", &[]),
            person(4, "P004", "Trần Doãn Khiêm", &[]),
        ];
        let lac = propose_links(&parse.passages[0], &people);
        assert!(lac.iter().any(|link| link.person_key == "P001"
            && link.status == LinkStatus::Resolved
            && link.match_kind == MatchKind::SectionHeading));
        assert!(lac.iter().any(|link| link.person_key == "P002"
            && link.status == LinkStatus::Ambiguous
            && link.match_kind == MatchKind::NameMention));

        let khiem = propose_links(&parse.passages[1], &people);
        assert!(khiem
            .iter()
            .any(|link| link.person_key == "P004" && link.status == LinkStatus::Resolved));
        assert!(khiem
            .iter()
            .any(|link| link.person_key == "P003" && link.status == LinkStatus::Ambiguous));
    }

    #[test]
    fn a_namesake_heading_is_ambiguous_not_guessed() {
        let parse = parse();
        let people = vec![
            person(1, "P001", "Trần Tối Lạc", &[]),
            person(2, "P002", "Trần Tối Lạc", &[]),
        ];
        let links = propose_links(&parse.passages[0], &people);
        assert_eq!(links.len(), 2);
        assert!(links
            .iter()
            .all(|link| link.status == LinkStatus::Ambiguous));
    }

    #[test]
    fn a_given_name_alone_never_links() {
        let parse = parse();
        let people = vec![person(9, "P009", "Sần", &[])];
        let links = propose_links(&parse.passages[0], &people);
        assert!(links.is_empty(), "a one-word name is not an identity match");
    }

    #[test]
    fn explicit_identifiers_resolve() {
        let mut passage = parse().passages[0].clone();
        passage.body = "Recorded as P002 in the catalog.".to_string();
        passage.section_title = None;
        let people = vec![
            person(2, "P002", "Trần Văn Sần", &[]),
            person(3, "P003", "Trần Thị Vọng", &[]),
        ];
        let links = propose_links(&passage, &people);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].person_key, "P002");
        assert_eq!(links[0].status, LinkStatus::Resolved);
        assert_eq!(links[0].match_kind, MatchKind::ExplicitId);
    }

    #[test]
    fn a_hyphenated_catalog_key_is_matched_as_a_sequence() {
        let mut passage = parse().passages[0].clone();
        passage.section_title = None;
        passage.body = "Recorded as ce-0007-0123456789abcdef in the catalog.".to_string();
        let people = vec![person(7, "ce-0007-0123456789abcdef", "Trần Tối Lạc", &[])];
        let links = propose_links(&passage, &people);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].match_kind, MatchKind::ExplicitId);
        assert_eq!(links[0].status, LinkStatus::Resolved);
    }

    #[test]
    fn a_catalog_key_in_the_heading_is_matched_too() {
        let mut passage = parse().passages[0].clone();
        passage.section_title = Some("Đời thứ 11: ce-0007-0123456789abcdef".to_string());
        passage.body = "No name here.".to_string();
        let people = vec![person(7, "ce-0007-0123456789abcdef", "Trần Tối Lạc", &[])];
        let links = propose_links(&passage, &people);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].match_kind, MatchKind::ExplicitId);
    }

    #[test]
    fn page_markers_cover_all_forms_case_insensitively() {
        assert_eq!(page_marker("## PDF page 12"), Some(12));
        assert_eq!(page_marker("## pdf page 12"), Some(12));
        assert_eq!(page_marker("## Pdf Page 12"), Some(12));
        assert_eq!(page_marker("## Trang PDF 12"), Some(12));
        assert_eq!(page_marker("## Page 12"), Some(12));
        assert_eq!(page_marker("## Trang 12"), Some(12));
        assert_eq!(page_marker("[p. 12]"), Some(12));
        assert_eq!(page_marker("A line with no page marker"), None);
        assert_eq!(page_marker("## Not a page marker"), None);
    }

    #[test]
    fn import_decisions_require_review_before_replacing_published_text() {
        let pending = vec![("k".to_string(), "aaa".to_string(), "pending".to_string())];
        let approved = vec![("k".to_string(), "aaa".to_string(), "approved".to_string())];
        let rejected = vec![("k".to_string(), "aaa".to_string(), "rejected".to_string())];
        assert_eq!(decide(&pending, "aaa"), ImportDecision::Unchanged);
        assert_eq!(decide(&pending, "bbb"), ImportDecision::Update);
        assert_eq!(decide(&approved, "aaa"), ImportDecision::Unchanged);
        assert_eq!(decide(&approved, "bbb"), ImportDecision::Revision);
        assert_eq!(decide(&rejected, "bbb"), ImportDecision::Revision);
        assert_eq!(decide(&[], "aaa"), ImportDecision::Insert);
    }

    fn person(id: i64, key: &str, name: &str, aliases: &[&str]) -> LinkablePerson {
        LinkablePerson {
            person_id: id,
            person_key: key.to_string(),
            display_name: name.to_string(),
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
        }
    }
}
