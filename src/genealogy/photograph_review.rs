//! AP-105: photograph-to-person link proposals and publication review.
//!
//! A new photograph starts `unpublished`. A link to a person is proposed only
//! from an explicit caption that names that person, using the same folding,
//! alias, and name-order matching as [`super::person_search`]. A link never
//! derives from page position, ordering, or a facial comparison.
//!
//! Proposal rules:
//!
//! - a caption with no recorded name yields no link;
//! - every distinct exact or alias name in the caption is proposed, so a group
//!   caption proposes each named person;
//! - if two recorded people share the matched name, or only a weak partial or
//!   reordered match exists, the result is `ambiguous` and a reviewer must
//!   choose;
//! - confirming or rejecting a link, editing a caption, and setting visibility
//!   each append exactly one append-only review event carrying the reviewer,
//!   the time (the event row's `created_at`), and the prior value;
//! - repeating a decision is a no-op and appends no event.
//!
//! Review events use the existing append-only `archive_photograph_inventory_event`
//! ledger with action `reviewed`; the operation, asset, link, prior, and next
//! values live in the event detail. Caption edits update the inventory image
//! caption, so the asset keeps its document/page/object provenance.

use std::collections::{BTreeMap, BTreeSet};

use postgres::{Client, GenericClient};

use super::current::CURRENT_ASSERTION;
use super::inventory;
use super::person_search::{self, Candidate, MatchKind, PersonNames};
use super::photograph_context::{self, LayoutProposal, PersonContext};
use super::GenealogyError;

/// The publication state of a new photograph.
pub const PUBLICATION_UNPUBLISHED: &str = "unpublished";
/// The publication state after a reviewer publishes a photograph.
pub const PUBLICATION_PUBLISHED: &str = "published";

/// The stored status of a photograph-to-person link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkStatus {
    /// A candidate the matcher proposed; a reviewer has not decided.
    Proposed,
    /// The reviewer confirmed the person is in the photograph.
    Confirmed,
    /// The reviewer rejected the candidate.
    Rejected,
}

impl LinkStatus {
    /// The stored wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            LinkStatus::Proposed => "proposed",
            LinkStatus::Confirmed => "confirmed",
            LinkStatus::Rejected => "rejected",
        }
    }

    /// Parses a stored status. Unknown values are a domain error.
    pub fn parse(value: &str) -> Result<Self, GenealogyError> {
        match value {
            "proposed" => Ok(LinkStatus::Proposed),
            "confirmed" => Ok(LinkStatus::Confirmed),
            "rejected" => Ok(LinkStatus::Rejected),
            other => Err(GenealogyError::Domain(format!(
                "unknown photograph link status {other}"
            ))),
        }
    }
}

/// A person candidate a caption names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProposalCandidate {
    /// Stable person key.
    pub person_key: String,
    /// The recorded display name.
    pub display_name: String,
    /// How the caption matched this person.
    pub match_kind: MatchKind,
}

/// The outcome of reading a caption for person links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalOutcome {
    /// The caption names nobody recorded. No link is proposed.
    NoLink,
    /// One or more distinct named people. A proposal names each of them.
    Proposed {
        /// The matched people, in catalog order.
        people: Vec<ProposalCandidate>,
    },
    /// A reviewer must choose. `proposed` holds distinct named people that are
    /// still safe to propose; `candidates` holds the ambiguous namesakes or
    /// weak leads that must not become links without a decision.
    Ambiguous {
        /// Distinct named people that are safe to propose.
        proposed: Vec<ProposalCandidate>,
        /// Candidates that need an explicit reviewer choice.
        candidates: Vec<ProposalCandidate>,
    },
}

/// Proposes person links from an explicit caption alone.
///
/// The caption is the only input. The function never reads page number,
/// ordering, or image geometry, so a link cannot derive from position or a
/// facial comparison.
pub fn propose_from_caption(caption: &str, catalog: &[PersonNames]) -> ProposalOutcome {
    let folded = person_search::fold(caption);
    let tokens: Vec<&str> = folded.split_whitespace().collect();
    if tokens.is_empty() {
        return ProposalOutcome::NoLink;
    }

    let mut exact: BTreeMap<String, Candidate> = BTreeMap::new();
    let mut weak: BTreeMap<String, Candidate> = BTreeMap::new();
    for start in 0..tokens.len() {
        for end in (start + 1)..=tokens.len() {
            let window = tokens[start..end].join(" ");
            for candidate in person_search::rank(catalog, &window) {
                let slot = match candidate.match_kind {
                    Some(MatchKind::Exact | MatchKind::Alias) => &mut exact,
                    Some(MatchKind::Reordered | MatchKind::Partial) => &mut weak,
                    None => continue,
                };
                slot.entry(candidate.person_key.clone())
                    .and_modify(|current| {
                        if candidate.match_kind < current.match_kind {
                            *current = candidate.clone();
                        }
                    })
                    .or_insert(candidate);
            }
        }
    }

    // A person already matched exactly is not also a weak lead.
    for key in exact.keys() {
        weak.remove(key);
    }

    if !exact.is_empty() {
        // Two people with the same recorded spelling are namesakes: the
        // caption cannot choose between them.
        let mut groups: BTreeMap<String, Vec<Candidate>> = BTreeMap::new();
        for candidate in exact.into_values() {
            let matched = candidate
                .matched_name
                .clone()
                .unwrap_or_else(|| candidate.display_name.clone());
            groups
                .entry(person_search::fold(&matched))
                .or_default()
                .push(candidate);
        }
        let mut proposed: Vec<ProposalCandidate> = Vec::new();
        let mut ambiguous: Vec<ProposalCandidate> = Vec::new();
        for mut group in groups.into_values() {
            if group.len() == 1 {
                proposed.push(to_candidate(&group[0]));
            } else {
                group.sort_by(|left, right| left.person_key.cmp(&right.person_key));
                ambiguous.extend(group.iter().map(to_candidate));
            }
        }
        // A weak lead never blocks the safe exact proposals, but it does need
        // a reviewer's choice before any weaker association is written.
        let mut weak_candidates: Vec<Candidate> = weak.into_values().collect();
        if !weak_candidates.is_empty() {
            sort_candidates(&mut weak_candidates);
            ambiguous.extend(
                person_search::top_tier(&weak_candidates)
                    .into_iter()
                    .map(to_candidate),
            );
        }
        if !ambiguous.is_empty() {
            return ProposalOutcome::Ambiguous {
                proposed,
                candidates: ambiguous,
            };
        }
        return ProposalOutcome::Proposed { people: proposed };
    }

    if !weak.is_empty() {
        let mut candidates: Vec<Candidate> = weak.into_values().collect();
        sort_candidates(&mut candidates);
        let tier = person_search::top_tier(&candidates);
        return ProposalOutcome::Ambiguous {
            proposed: Vec::new(),
            candidates: tier
                .iter()
                .map(|candidate| to_candidate(candidate))
                .collect(),
        };
    }

    ProposalOutcome::NoLink
}

fn sort_candidates(candidates: &mut [Candidate]) {
    candidates.sort_by(|left, right| {
        left.match_kind
            .cmp(&right.match_kind)
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.person_key.cmp(&right.person_key))
    });
}

fn to_candidate(candidate: &Candidate) -> ProposalCandidate {
    ProposalCandidate {
        person_key: candidate.person_key.clone(),
        display_name: candidate.display_name.clone(),
        match_kind: candidate.match_kind.unwrap_or(MatchKind::Partial),
    }
}

/// The stored state of one link a reviewer acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRow {
    /// The link id.
    pub link_id: i64,
    /// The photograph asset the link belongs to.
    pub asset_id: i64,
    /// The person the link names.
    pub person_id: i64,
    /// The recorded status.
    pub status: LinkStatus,
}

/// One append-only review event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReviewEvent {
    /// The operation, carried in the event detail.
    pub operation: &'static str,
    /// The reviewer who acted.
    pub actor: String,
    /// The photograph asset the event concerns.
    pub asset_id: i64,
    /// The link the event concerns, when the operation names one.
    pub link_id: Option<i64>,
    /// The person the operation names, when it names one.
    pub person_id: Option<i64>,
    /// The prior value.
    pub prior: String,
    /// The requested next value.
    pub next: String,
    /// The caption that produced a proposal, when the operation is a proposal.
    pub caption: Option<String>,
    /// The people a proposal named, when the operation is a proposal.
    pub people: Vec<String>,
}

impl ReviewEvent {
    /// The JSON detail recorded on the ledger.
    pub fn detail(&self) -> String {
        serde_json::json!({
            "operation": self.operation,
            "asset_id": self.asset_id,
            "link_id": self.link_id,
            "person_id": self.person_id,
            "prior": self.prior,
            "next": self.next,
            "caption": self.caption,
            "people": self.people,
        })
        .to_string()
    }
}

/// The result of a decision: either a real state change with one event, or a
/// no-op because the requested value already equals the stored value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The state changes; exactly this one event is appended.
    Changed {
        /// The prior stored value.
        prior: String,
        /// The new stored value.
        next: String,
        /// The event to append.
        event: Box<ReviewEvent>,
    },
    /// The requested value equals the stored value; nothing is written.
    NoChange,
}

/// Decides a link status change (confirm or reject).
pub fn decide_link(
    link: &LinkRow,
    next: LinkStatus,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    if next == LinkStatus::Proposed {
        return Err(GenealogyError::Domain(
            "a link decision must confirm or reject".into(),
        ));
    }
    if link.status == next {
        return Ok(Decision::NoChange);
    }
    Ok(Decision::Changed {
        prior: link.status.as_str().to_string(),
        next: next.as_str().to_string(),
        event: Box::new(ReviewEvent {
            operation: operation_for(next),
            actor: actor.to_string(),
            asset_id: link.asset_id,
            link_id: Some(link.link_id),
            person_id: Some(link.person_id),
            prior: link.status.as_str().to_string(),
            next: next.as_str().to_string(),
            caption: None,
            people: Vec::new(),
        }),
    })
}

/// Decides a link that has no stored row yet, for a reviewer choosing between
/// namesakes. The prior value is `none`.
pub fn decide_new_link(
    asset_id: i64,
    person_id: i64,
    next: LinkStatus,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    if next == LinkStatus::Proposed {
        return Err(GenealogyError::Domain(
            "a link decision must confirm or reject".into(),
        ));
    }
    Ok(Decision::Changed {
        prior: "none".to_string(),
        next: next.as_str().to_string(),
        event: Box::new(ReviewEvent {
            operation: operation_for(next),
            actor: actor.to_string(),
            asset_id,
            link_id: None,
            person_id: Some(person_id),
            prior: "none".to_string(),
            next: next.as_str().to_string(),
            caption: None,
            people: Vec::new(),
        }),
    })
}

fn operation_for(next: LinkStatus) -> &'static str {
    match next {
        LinkStatus::Confirmed => "link_confirmed",
        LinkStatus::Rejected => "link_rejected",
        LinkStatus::Proposed => "link_proposed",
    }
}

/// Confirms a stored link.
pub fn confirm(link: &LinkRow, actor: &str) -> Result<Decision, GenealogyError> {
    decide_link(link, LinkStatus::Confirmed, actor)
}

/// Rejects a stored link.
pub fn reject(link: &LinkRow, actor: &str) -> Result<Decision, GenealogyError> {
    decide_link(link, LinkStatus::Rejected, actor)
}

/// Edits a caption. An equal caption is a no-op.
pub fn edit_caption(asset_id: i64, current: Option<&str>, next: &str, actor: &str) -> Decision {
    let current_text = current.unwrap_or("");
    if current_text == next {
        return Decision::NoChange;
    }
    Decision::Changed {
        prior: current_text.to_string(),
        next: next.to_string(),
        event: Box::new(ReviewEvent {
            operation: "caption_edited",
            actor: actor.to_string(),
            asset_id,
            link_id: None,
            person_id: None,
            prior: current_text.to_string(),
            next: next.to_string(),
            caption: Some(next.to_string()),
            people: Vec::new(),
        }),
    }
}

/// Sets a photograph publication state. An equal state is a no-op.
pub fn set_visibility(
    asset_id: i64,
    current: &str,
    next: &str,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    if next != PUBLICATION_UNPUBLISHED && next != PUBLICATION_PUBLISHED {
        return Err(GenealogyError::Domain(format!(
            "unknown publication state {next}"
        )));
    }
    if current == next {
        return Ok(Decision::NoChange);
    }
    Ok(Decision::Changed {
        prior: current.to_string(),
        next: next.to_string(),
        event: Box::new(ReviewEvent {
            operation: "visibility_set",
            actor: actor.to_string(),
            asset_id,
            link_id: None,
            person_id: None,
            prior: current.to_string(),
            next: next.to_string(),
            caption: None,
            people: Vec::new(),
        }),
    })
}

/// One proposed person persisted from a layout-aware proposal run.
///
/// The row is still `proposed`; a confirmed or rejected decision is never
/// overwritten by a later proposal run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLayoutCandidate {
    /// The person the proposal names.
    pub person_id: i64,
    /// Stable person key.
    pub person_key: String,
    /// The recorded display name.
    pub display_name: String,
    /// `exact`, `alias`, `reordered`, or `partial`.
    pub match_kind: String,
    /// `high`, `medium`, or `low`.
    pub confidence: String,
    /// `caption`, `explicit_group`, `section_heading`, `nearby_text`,
    /// `biography`, or `adjacent_page`.
    pub source_kind: String,
    /// The PDF page the supporting text is on.
    pub source_page: i32,
    /// The bounded supporting text.
    pub supporting_text: String,
    /// Why the confidence was assigned.
    pub confidence_basis: String,
    /// Why a reviewer must choose, when the match is ambiguous.
    pub ambiguity: Option<String>,
    /// 1-based strength order.
    pub candidate_rank: i32,
}

/// The result of one persisted layout-aware proposal run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredLayoutProposal {
    /// Candidates in strength order, excluding links a reviewer already decided.
    pub candidates: Vec<StoredLayoutCandidate>,
    /// Why the photo is left unlinked, when no candidate is supported.
    pub unlinked_reason: Option<String>,
    /// Confirmed or rejected links left untouched by this run.
    pub preserved_decisions: usize,
}

/// Loads the person catalog with the dates and relationships used to
/// distinguish namesakes.
///
/// A truncated catalog fails closed: a namesake outside the loaded bound cannot
/// be ruled out, so the caller must not propose links from a partial catalog.
pub fn load_person_contexts(client: &mut Client) -> Result<Vec<PersonContext>, GenealogyError> {
    let catalog = person_search::load_catalog(client)?;
    if catalog.truncated {
        return Err(GenealogyError::Domain(
            "person catalog is truncated; refusing photograph link proposals".into(),
        ));
    }
    let mut people: BTreeMap<String, PersonContext> = catalog
        .people
        .into_iter()
        .map(|person| {
            (
                person.person_key.clone(),
                PersonContext {
                    person_key: person.person_key,
                    display_name: person.display_name,
                    aliases: person.aliases,
                    generation: person.generation,
                    evidence_status: person.evidence_status,
                    dates: Vec::new(),
                    parent_keys: Vec::new(),
                    spouse_keys: Vec::new(),
                },
            )
        })
        .collect();

    let sql = format!(
        "SELECT p.person_key, a.value_date_start, a.value_date_end, a.value_text \
           FROM person p \
           JOIN assertion a ON a.person_id = p.person_id \
          WHERE a.assertion_kind IN ('birth_date', 'death_date') \
            AND ({CURRENT_ASSERTION})"
    );
    for row in client.query(&sql, &[])? {
        let key: String = row.get(0);
        let Some(person) = people.get_mut(&key) else {
            continue;
        };
        let mut years: BTreeSet<String> = person.dates.iter().cloned().collect();
        for value in [
            row.get::<_, Option<String>>(1),
            row.get::<_, Option<String>>(2),
            row.get::<_, Option<String>>(3),
        ]
        .into_iter()
        .flatten()
        {
            years.extend(years_in(&value));
        }
        person.dates = years.into_iter().collect();
    }

    let parent_sql = "SELECT child.person_key, parent.person_key \
                        FROM parent_child_link l \
                        JOIN person child ON child.person_id = l.child_id \
                        JOIN person parent ON parent.person_id = l.parent_id";
    for row in client.query(parent_sql, &[])? {
        let child: String = row.get(0);
        let parent: String = row.get(1);
        if let Some(person) = people.get_mut(&child) {
            person.parent_keys.push(parent);
        }
    }

    let spouse_sql = "SELECT p.person_key, s.person_key \
                        FROM spouse_link l \
                        JOIN person p ON p.person_id = l.person_id \
                        JOIN person s ON s.person_id = l.spouse_id \
                       UNION \
                      SELECT s.person_key, p.person_key \
                        FROM spouse_link l \
                        JOIN person p ON p.person_id = l.person_id \
                        JOIN person s ON s.person_id = l.spouse_id";
    for row in client.query(spouse_sql, &[])? {
        let person: String = row.get(0);
        let spouse: String = row.get(1);
        if let Some(entry) = people.get_mut(&person) {
            entry.spouse_keys.push(spouse);
        }
    }

    for person in people.values_mut() {
        person.dates.sort();
        person.dates.dedup();
        person.parent_keys.sort();
        person.parent_keys.dedup();
        person.spouse_keys.sort();
        person.spouse_keys.dedup();
    }
    Ok(people.into_values().collect())
}

/// Reads the original PDF, extracts its layout, proposes people, and persists
/// only proposed links.
///
/// Confirmed and rejected links are preserved exactly: the refresh deletes only
/// `proposed` rows and the upsert updates only `proposed` rows. A candidate whose
/// link a reviewer already decided is left out of the returned proposal and is
/// counted in `preserved_decisions`.
pub fn propose_layout_links_db(
    client: &mut Client,
    asset_id: i64,
    bytes: &[u8],
    actor: &str,
) -> Result<StoredLayoutProposal, GenealogyError> {
    let asset = client
        .query_opt(
            "SELECT a.inventory_id, a.object_number, a.object_generation, i.document_version_id \
               FROM archive_photograph_asset a \
               JOIN archive_photograph_inventory i ON i.inventory_id = a.inventory_id \
              WHERE a.asset_id = $1",
            &[&asset_id],
        )?
        .ok_or_else(|| GenealogyError::Domain(format!("no photograph asset {asset_id}")))?;
    let inventory_id: i64 = asset.get(0);
    let object_number: i64 = asset.get(1);
    let object_generation: i32 = asset.get(2);

    let page_number: i32 = client
        .query_opt(
            "SELECT page_number FROM archive_photograph_occurrence \
              WHERE asset_id = $1 \
              ORDER BY page_number, occurrence_ordinal \
              LIMIT 1",
            &[&asset_id],
        )?
        .map(|row| row.get(0))
        .unwrap_or(1);
    let caption = caption_for_asset(client, asset_id)?;

    let source =
        inventory::inventory_bytes(bytes).map_err(|err| GenealogyError::Domain(err.to_string()))?;
    let placement = source
        .pages
        .iter()
        .find(|page| page.page_number == page_number as u32)
        .and_then(|page| {
            page.images.iter().find(|image| {
                image.object_number == object_number as u32
                    && image.generation == object_generation as u16
            })
        })
        .and_then(|image| image.placement);
    let context = photograph_context::extract_context(bytes)
        .map_err(|err| GenealogyError::Domain(err.to_string()))?;
    let catalog = load_person_contexts(client)?;
    let proposal = photograph_context::propose_layout_links(
        &context,
        page_number as u32,
        placement,
        caption.as_deref(),
        &catalog,
    );
    persist_layout_proposal(
        client,
        inventory_id,
        asset_id,
        caption.as_deref(),
        &proposal,
        actor,
    )
}

fn persist_layout_proposal(
    client: &mut Client,
    inventory_id: i64,
    asset_id: i64,
    caption: Option<&str>,
    proposal: &LayoutProposal,
    actor: &str,
) -> Result<StoredLayoutProposal, GenealogyError> {
    let mut transaction = client.transaction()?;
    let existing: BTreeMap<i64, String> = transaction
        .query(
            "SELECT person_id, status FROM archive_photograph_person_link \
              WHERE asset_id = $1",
            &[&asset_id],
        )?
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();

    let mut stored: Vec<StoredLayoutCandidate> = Vec::new();
    let mut preserved_decisions = 0usize;
    for candidate in &proposal.candidates {
        let person_id: i64 = transaction
            .query_one(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&candidate.person_key],
            )?
            .get(0);
        if let Some(status) = existing.get(&person_id) {
            if status.as_str() != "proposed" {
                preserved_decisions += 1;
                continue;
            }
        }
        stored.push(StoredLayoutCandidate {
            person_id,
            person_key: candidate.person_key.clone(),
            display_name: candidate.display_name.clone(),
            match_kind: candidate.match_kind.as_str().to_string(),
            confidence: candidate.confidence.as_str().to_string(),
            source_kind: candidate.source_kind.as_str().to_string(),
            source_page: candidate.source_page as i32,
            supporting_text: candidate.supporting_text.clone(),
            confidence_basis: candidate.confidence_basis.clone(),
            ambiguity: candidate.ambiguity.clone(),
            candidate_rank: candidate.candidate_rank,
        });
    }

    let person_ids: Vec<i64> = stored.iter().map(|candidate| candidate.person_id).collect();
    transaction.execute(
        "DELETE FROM archive_photograph_person_link \
          WHERE asset_id = $1 AND status = 'proposed' \
            AND person_id <> ALL($2::bigint[])",
        &[&asset_id, &person_ids],
    )?;

    for candidate in &stored {
        transaction.execute(
            "INSERT INTO archive_photograph_person_link \
                 (asset_id, person_id, status, source_page, supporting_text, \
                  confidence, confidence_basis, ambiguity_note, match_kind, \
                  candidate_rank, proposal_origin) \
             VALUES ($1, $2, 'proposed', $3, $4, $5, $6, $7, $8, $9, 'layout') \
             ON CONFLICT (asset_id, person_id) DO UPDATE \
                SET source_page = EXCLUDED.source_page, \
                    supporting_text = EXCLUDED.supporting_text, \
                    confidence = EXCLUDED.confidence, \
                    confidence_basis = EXCLUDED.confidence_basis, \
                    ambiguity_note = EXCLUDED.ambiguity_note, \
                    match_kind = EXCLUDED.match_kind, \
                    candidate_rank = EXCLUDED.candidate_rank, \
                    proposal_origin = 'layout' \
              WHERE archive_photograph_person_link.status = 'proposed'",
            &[
                &asset_id,
                &candidate.person_id,
                &candidate.source_page,
                &candidate.supporting_text,
                &candidate.confidence,
                &candidate.confidence_basis,
                &candidate.ambiguity,
                &candidate.match_kind,
                &candidate.candidate_rank,
            ],
        )?;
    }

    let people: Vec<String> = stored
        .iter()
        .map(|candidate| candidate.person_key.clone())
        .collect();
    append_event(
        &mut transaction,
        inventory_id,
        &ReviewEvent {
            operation: "layout_proposals",
            actor: actor.to_string(),
            asset_id,
            link_id: None,
            person_id: None,
            prior: "none".to_string(),
            next: format!("{} proposed", stored.len()),
            caption: caption.map(str::to_string),
            people,
        },
    )?;
    transaction.commit()?;
    Ok(StoredLayoutProposal {
        candidates: stored,
        unlinked_reason: proposal.unlinked_reason.clone(),
        preserved_decisions,
    })
}

fn caption_for_asset(client: &mut Client, asset_id: i64) -> Result<Option<String>, GenealogyError> {
    let row = client.query_opt(
        "SELECT COALESCE( \
                  (SELECT NULLIF(btrim(i.caption), '') \
                     FROM archive_photograph_inventory_image i \
                    WHERE i.inventory_id = a.inventory_id \
                      AND i.object_number = a.object_number \
                      AND i.object_generation = a.object_generation \
                      AND i.caption IS NOT NULL \
                      AND btrim(i.caption) <> '' \
                    ORDER BY i.page_number, i.occurrence_ordinal \
                    LIMIT 1), \
                  (SELECT NULLIF(btrim(p.caption), '') \
                     FROM archive_photograph_inventory_page p \
                    WHERE p.inventory_id = a.inventory_id \
                      AND p.page_number = ( \
                          SELECT o.page_number \
                            FROM archive_photograph_occurrence o \
                           WHERE o.asset_id = a.asset_id \
                           ORDER BY o.page_number, o.occurrence_ordinal \
                           LIMIT 1) \
                  ) \
                ) \
           FROM archive_photograph_asset a \
          WHERE a.asset_id = $1",
        &[&asset_id],
    )?;
    Ok(row.and_then(|row| row.get(0)))
}

/// Extracts four-digit years from a date value.
fn years_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut years = Vec::new();
    let mut index = 0;
    while index + 4 <= bytes.len() {
        if bytes[index..index + 4].iter().all(u8::is_ascii_digit) {
            let year = &text[index..index + 4];
            if !years.iter().any(|existing: &String| existing == year) {
                years.push(year.to_string());
            }
            index += 4;
        } else {
            index += 1;
        }
    }
    years
}
/// Loads one link, or `None` when the pair has no row.
pub fn load_link<C: GenericClient>(
    client: &mut C,
    asset_id: i64,
    person_id: i64,
) -> Result<Option<LinkRow>, GenealogyError> {
    let row = client.query_opt(
        "SELECT link_id, asset_id, person_id, status
           FROM archive_photograph_person_link
          WHERE asset_id = $1 AND person_id = $2",
        &[&asset_id, &person_id],
    )?;
    let Some(row) = row else {
        return Ok(None);
    };
    let status: String = row.get(3);
    Ok(Some(LinkRow {
        link_id: row.get(0),
        asset_id: row.get(1),
        person_id: row.get(2),
        status: LinkStatus::parse(&status)?,
    }))
}

/// Proposes links from `caption` and persists the safe proposals.
///
/// Ambiguous candidates are returned but not written; a reviewer chooses with
/// [`confirm_db`] or [`reject_db`]. A repeated proposal whose links already
/// exist writes no event. A truncated person catalog fails closed, because a
/// namesake outside the loaded bound cannot be ruled out.
pub fn propose_links_db(
    client: &mut Client,
    asset_id: i64,
    caption: &str,
    actor: &str,
) -> Result<ProposalOutcome, GenealogyError> {
    let catalog = person_search::load_catalog(client)?;
    let outcome = propose_from_caption(caption, &catalog.people);
    if catalog.truncated && outcome != ProposalOutcome::NoLink {
        return Err(GenealogyError::Domain(
            "person catalog is truncated; refusing photograph link proposals".into(),
        ));
    }
    let proposed = match &outcome {
        ProposalOutcome::NoLink => return Ok(outcome),
        ProposalOutcome::Proposed { people } => people.clone(),
        ProposalOutcome::Ambiguous { proposed, .. } => proposed.clone(),
    };
    if proposed.is_empty() {
        return Ok(outcome);
    }

    let mut transaction = client.transaction()?;
    let inventory_id = inventory_for_asset(&mut transaction, asset_id)?;
    let mut inserted = 0u64;
    let mut keys: Vec<String> = Vec::new();
    for person in &proposed {
        let person_id: i64 = transaction
            .query_one(
                "SELECT person_id FROM person WHERE person_key = $1",
                &[&person.person_key],
            )?
            .get(0);
        inserted += transaction.execute(
            "INSERT INTO archive_photograph_person_link (asset_id, person_id, status)
                  VALUES ($1, $2, 'proposed')
             ON CONFLICT (asset_id, person_id) DO NOTHING",
            &[&asset_id, &person_id],
        )?;
        keys.push(person.person_key.clone());
    }
    if inserted > 0 {
        append_event(
            &mut transaction,
            inventory_id,
            &ReviewEvent {
                operation: "link_proposed",
                actor: actor.to_string(),
                asset_id,
                link_id: None,
                person_id: None,
                prior: "none".to_string(),
                next: "proposed".to_string(),
                caption: Some(caption.to_string()),
                people: keys,
            },
        )?;
    }
    transaction.commit()?;
    Ok(outcome)
}

/// Confirms a link. A namesake choice with no stored row creates the row.
pub fn confirm_db(
    client: &mut Client,
    asset_id: i64,
    person_id: i64,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    decide_link_db(client, asset_id, person_id, LinkStatus::Confirmed, actor)
}

/// Rejects a link. A namesake choice with no stored row creates a rejected row.
pub fn reject_db(
    client: &mut Client,
    asset_id: i64,
    person_id: i64,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    decide_link_db(client, asset_id, person_id, LinkStatus::Rejected, actor)
}

fn decide_link_db(
    client: &mut Client,
    asset_id: i64,
    person_id: i64,
    next: LinkStatus,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    let mut transaction = client.transaction()?;
    let inventory_id = inventory_for_asset(&mut transaction, asset_id)?;
    let existing = load_link(&mut transaction, asset_id, person_id)?;
    let decision = match &existing {
        Some(link) => decide_link(link, next, actor)?,
        None => decide_new_link(asset_id, person_id, next, actor)?,
    };
    let changed = match &decision {
        Decision::NoChange => None,
        Decision::Changed {
            next: new_status,
            event,
            ..
        } => Some((new_status.clone(), event.clone())),
    };
    if let Some((new_status, mut event)) = changed {
        let link_id = match existing {
            Some(link) => {
                transaction.execute(
                    "UPDATE archive_photograph_person_link
                        SET status = $1, decided_by = $2, decided_at = now()
                      WHERE link_id = $3",
                    &[&new_status, &actor, &link.link_id],
                )?;
                link.link_id
            }
            None => transaction
                .query_one(
                    "INSERT INTO archive_photograph_person_link
                         (asset_id, person_id, status, decided_by, decided_at)
                     VALUES ($1, $2, $3, $4, now())
                     RETURNING link_id",
                    &[&asset_id, &person_id, &new_status, &actor],
                )?
                .get(0),
        };
        event.link_id = Some(link_id);
        append_event(&mut transaction, inventory_id, &event)?;
        transaction.commit()?;
        return Ok(Decision::Changed {
            prior: event.prior.clone(),
            next: event.next.clone(),
            event,
        });
    }
    transaction.commit()?;
    Ok(decision)
}

/// Edits the inventory image caption for the asset's source object.
pub fn edit_caption_db(
    client: &mut Client,
    asset_id: i64,
    next: &str,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    let mut transaction = client.transaction()?;
    let row = transaction
        .query_opt(
            "SELECT a.inventory_id, a.object_number, a.object_generation,
                    (SELECT i.caption
                       FROM archive_photograph_inventory_image i
                      WHERE i.inventory_id = a.inventory_id
                        AND i.object_number = a.object_number
                        AND i.object_generation = a.object_generation
                      ORDER BY i.page_number, i.occurrence_ordinal
                      LIMIT 1)
               FROM archive_photograph_asset a
              WHERE a.asset_id = $1",
            &[&asset_id],
        )?
        .ok_or_else(|| GenealogyError::Domain(format!("no photograph asset {asset_id}")))?;
    let inventory_id: i64 = row.get(0);
    let object_number: i64 = row.get(1);
    let object_generation: i32 = row.get(2);
    let current: Option<String> = row.get(3);
    let decision = edit_caption(asset_id, current.as_deref(), next, actor);
    if let Decision::Changed { event, .. } = &decision {
        let updated = transaction.execute(
            "UPDATE archive_photograph_inventory_image
                SET caption = $1
              WHERE inventory_id = $2 AND object_number = $3 AND object_generation = $4",
            &[&next, &inventory_id, &object_number, &object_generation],
        )?;
        if updated == 0 {
            return Err(GenealogyError::Domain(format!(
                "no inventory image caption row for asset {asset_id}"
            )));
        }
        append_event(&mut transaction, inventory_id, event)?;
    }
    transaction.commit()?;
    Ok(decision)
}

/// Sets a photograph publication state.
pub fn set_visibility_db(
    client: &mut Client,
    asset_id: i64,
    next: &str,
    actor: &str,
) -> Result<Decision, GenealogyError> {
    let mut transaction = client.transaction()?;
    let inventory_id = inventory_for_asset(&mut transaction, asset_id)?;
    let current: String = transaction
        .query_one(
            "SELECT publication_state FROM archive_photograph_asset WHERE asset_id = $1",
            &[&asset_id],
        )?
        .get(0);
    let decision = set_visibility(asset_id, &current, next, actor)?;
    if let Decision::Changed { event, .. } = &decision {
        transaction.execute(
            "UPDATE archive_photograph_asset
                SET publication_state = $1 WHERE asset_id = $2",
            &[&next, &asset_id],
        )?;
        append_event(&mut transaction, inventory_id, event)?;
    }
    transaction.commit()?;
    Ok(decision)
}

fn inventory_for_asset<C: GenericClient>(
    client: &mut C,
    asset_id: i64,
) -> Result<i64, GenealogyError> {
    client
        .query_opt(
            "SELECT inventory_id FROM archive_photograph_asset WHERE asset_id = $1",
            &[&asset_id],
        )?
        .map(|row| row.get(0))
        .ok_or_else(|| GenealogyError::Domain(format!("no photograph asset {asset_id}")))
}

fn append_event<C: GenericClient>(
    client: &mut C,
    inventory_id: i64,
    event: &ReviewEvent,
) -> Result<(), GenealogyError> {
    client.execute(
        "INSERT INTO archive_photograph_inventory_event
             (inventory_id, action, actor, detail)
         VALUES ($1, 'reviewed', $2, $3::text::jsonb)",
        &[&inventory_id, &event.actor, &event.detail()],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(key: &str, name: &str, aliases: &[&str]) -> PersonNames {
        PersonNames {
            person_key: key.to_string(),
            display_name: name.to_string(),
            generation: None,
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            evidence_status: "unverified".to_string(),
        }
    }

    fn keys(outcome: &ProposalOutcome) -> Vec<&str> {
        match outcome {
            ProposalOutcome::NoLink => Vec::new(),
            ProposalOutcome::Proposed { people } => people
                .iter()
                .map(|person| person.person_key.as_str())
                .collect(),
            ProposalOutcome::Ambiguous {
                proposed,
                candidates,
            } => proposed
                .iter()
                .chain(candidates.iter())
                .map(|person| person.person_key.as_str())
                .collect(),
        }
    }

    #[test]
    fn a_single_exact_name_is_proposed() {
        let catalog = vec![person("p-an", "Nguyễn Văn An", &[])];
        let outcome = propose_from_caption("Nguyễn Văn An at the reunion", &catalog);
        assert_eq!(keys(&outcome), vec!["p-an"]);
        assert!(matches!(outcome, ProposalOutcome::Proposed { .. }));
    }

    #[test]
    fn a_group_caption_proposes_each_named_person() {
        let catalog = vec![
            person("p-an", "Nguyễn Văn An", &[]),
            person("p-binh", "Trần Thị Bình", &[]),
        ];
        let outcome = propose_from_caption("Nguyễn Văn An và Trần Thị Bình", &catalog);
        let mut keys = keys(&outcome);
        keys.sort();
        assert_eq!(keys, vec!["p-an", "p-binh"]);
        assert!(matches!(outcome, ProposalOutcome::Proposed { .. }));
    }

    #[test]
    fn an_alias_is_proposed() {
        let catalog = vec![person("p-an", "Nguyễn Văn An", &["Ông An"])];
        let outcome = propose_from_caption("Ông An", &catalog);
        assert_eq!(keys(&outcome), vec!["p-an"]);
        match outcome {
            ProposalOutcome::Proposed { people } => {
                assert_eq!(people[0].match_kind, MatchKind::Alias);
            }
            other => panic!("expected a proposal, got {other:?}"),
        }
    }

    #[test]
    fn namesakes_are_ambiguous() {
        let catalog = vec![
            person("p-an-1", "Nguyễn Văn An", &[]),
            person("p-an-2", "Nguyễn Văn An", &[]),
        ];
        let outcome = propose_from_caption("Nguyễn Văn An", &catalog);
        match outcome {
            ProposalOutcome::Ambiguous {
                proposed,
                candidates,
            } => {
                assert!(proposed.is_empty());
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn an_exact_name_with_a_weak_lead_is_ambiguous() {
        let catalog = vec![
            person("p-an", "Nguyễn Văn An", &[]),
            person("p-binh", "Nguyễn Văn An Bình", &[]),
        ];
        match propose_from_caption("Nguyễn Văn An", &catalog) {
            ProposalOutcome::Ambiguous {
                proposed,
                candidates,
            } => {
                assert_eq!(proposed.len(), 1);
                assert_eq!(proposed[0].person_key, "p-an");
                assert_eq!(candidates.len(), 1);
                assert_eq!(candidates[0].person_key, "p-binh");
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn an_unnamed_caption_yields_no_link() {
        let catalog = vec![person("p-an", "Nguyễn Văn An", &[])];
        assert_eq!(
            propose_from_caption("Family gathering, 1962", &catalog),
            ProposalOutcome::NoLink
        );
    }

    #[test]
    fn a_weak_match_is_ambiguous() {
        let catalog = vec![person("p-an", "Nguyễn Văn An", &[])];
        match propose_from_caption("Nguyễn Văn", &catalog) {
            ProposalOutcome::Ambiguous { candidates, .. } => {
                assert_eq!(candidates.len(), 1);
            }
            other => panic!("expected ambiguity, got {other:?}"),
        }
    }

    #[test]
    fn repeating_a_decision_is_a_no_op() {
        let link = LinkRow {
            link_id: 1,
            asset_id: 2,
            person_id: 3,
            status: LinkStatus::Confirmed,
        };
        assert_eq!(confirm(&link, "reviewer").unwrap(), Decision::NoChange);
        assert_eq!(
            set_visibility(2, PUBLICATION_PUBLISHED, PUBLICATION_PUBLISHED, "reviewer").unwrap(),
            Decision::NoChange
        );
        assert_eq!(
            edit_caption(2, Some("same"), "same", "reviewer"),
            Decision::NoChange
        );
    }

    #[test]
    fn an_invalid_visibility_is_rejected() {
        assert!(set_visibility(1, PUBLICATION_UNPUBLISHED, "restricted", "reviewer").is_err());
    }
}
