//! Explicit "set as current source of truth" decisions (TRAN-AUTHORITY).
//!
//! An authorized reviewer may designate one reviewed document version as
//! authoritative for a defined scope: the whole document, one person, or a set
//! of selected assertions. Applying the decision makes the assertions that
//! version supports the *current* ones and supersedes the conflicting current
//! assertions inside the same scope.
//!
//! The guarantees this module makes:
//!
//! - a superseded assertion keeps its value, value dates, citations, and review
//!   events; only `superseded_at` changes, so every prior version stays readable
//!   through the authorized history view;
//! - `review_status` is never written here. An authoritative assertion is not a
//!   verified one, and an override never satisfies a verification requirement;
//! - omission is not deletion: a current assertion the document does not mention
//!   is only superseded when the reviewer names it in `remove_assertion_ids`;
//! - the apply is bound to the exact document version content hash and to the
//!   preview the reviewer confirmed, inside one transaction with the decision
//!   rows locked, so a stale or concurrent confirmation cannot overwrite a newer
//!   decision;
//! - a reversal is a new decision that restores what the reversed decision
//!   replaced. Nothing is deleted.

use std::collections::{BTreeMap, BTreeSet};

use postgres::{Client, GenericClient};
use serde::Serialize;
use thiserror::Error;

use super::query::Citation;
use super::GenealogyError;

/// The most items one decision may touch.
pub const MAX_DECISION_ITEMS: usize = 500;

/// Assertion kinds that describe one value per person. A newer source replaces
/// them even when the recorded spelling (and therefore the assertion key)
/// differs. Every other kind is multi-valued: two different aliases, branches,
/// parents, spouses, or links can legitimately coexist.
fn is_single_valued(kind: &str) -> bool {
    matches!(kind, "display_name" | "birth_date" | "death_date")
}

/// The scope of one authority decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Every assertion the document version supports.
    Document,
    /// The assertions the document version supports for one person.
    Person,
    /// Only the explicitly selected assertions.
    Assertions,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Document => "document",
            Scope::Person => "person",
            Scope::Assertions => "assertions",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value.trim() {
            "document" => Scope::Document,
            "person" => Scope::Person,
            "assertions" => Scope::Assertions,
            _ => return None,
        })
    }
}

/// What a reviewer asks to preview or apply.
#[derive(Debug, Clone)]
pub struct ScopeSpec<'a> {
    /// `document`, `person`, or `assertions`.
    pub scope: &'a str,
    /// Required for the `person` scope.
    pub person_key: Option<&'a str>,
    /// Required for the `assertions` scope.
    pub assertion_ids: &'a [i64],
    /// Current assertions the reviewer explicitly confirms removing.
    pub remove_assertion_ids: &'a [i64],
}

impl<'a> ScopeSpec<'a> {
    /// A document-scope request that removes nothing.
    pub fn document() -> Self {
        ScopeSpec {
            scope: "document",
            person_key: None,
            assertion_ids: &[],
            remove_assertion_ids: &[],
        }
    }
}

/// One assertion as the preview and the history present it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AssertionView {
    pub assertion_id: i64,
    pub person_key: String,
    pub display_name: String,
    pub kind: String,
    pub key: String,
    pub value_text: Option<String>,
    pub review_status: String,
    pub citations: Vec<CitationView>,
}

/// One citation, serialized for the browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CitationView {
    pub document_ref: String,
    pub page: i32,
    pub locator: Option<String>,
    pub excerpt: Option<String>,
    pub label: String,
}

impl From<&Citation> for CitationView {
    fn from(citation: &Citation) -> Self {
        CitationView {
            document_ref: citation.document_ref.clone(),
            page: citation.page,
            locator: citation.locator.clone(),
            excerpt: citation.excerpt.clone(),
            label: citation.label(),
        }
    }
}

/// The action one incoming assertion takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing current occupied that assertion slot.
    Added,
    /// A current assertion in the same slot is superseded.
    Replaced,
    /// This assertion is already the current one.
    Unchanged,
    /// The reviewer explicitly confirmed removing a current assertion.
    Removed,
}

impl Action {
    pub fn as_str(self) -> &'static str {
        match self {
            Action::Added => "added",
            Action::Replaced => "replaced",
            Action::Unchanged => "unchanged",
            Action::Removed => "removed",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "added" => Action::Added,
            "replaced" => Action::Replaced,
            "unchanged" => Action::Unchanged,
            "removed" => Action::Removed,
            _ => return None,
        })
    }
}

/// One planned change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewItem {
    pub action: String,
    /// The assertion that becomes current. `None` only for a removal.
    pub incoming: Option<AssertionView>,
    /// The current assertion(s) this decision supersedes.
    pub replaces: Vec<AssertionView>,
}

/// One reason an apply needs explicit acknowledgement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Conflict {
    /// `replaces_verified`, `replaces_disputed`, `removes_verified`, or
    /// `removes_disputed`.
    pub code: String,
    pub assertion: AssertionView,
}

/// The full picture a reviewer confirms before applying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuthorityPreview {
    pub document_version_id: i64,
    pub document_ref: String,
    pub content_sha256: String,
    pub scope: String,
    pub person_key: Option<String>,
    pub items: Vec<PreviewItem>,
    /// Current assertions the document does not mention. Omission is not
    /// deletion: they become removals only when the reviewer names them.
    pub omissions: Vec<AssertionView>,
    pub conflicts: Vec<Conflict>,
    /// The confirmation token. An apply whose recomputed preview differs is
    /// refused, so a stale review cannot overwrite a newer change.
    pub preview_hash: String,
}

/// A request to make one document version authoritative.
#[derive(Debug, Clone)]
pub struct ApplyInput<'a> {
    pub document_version_id: i64,
    /// The RAG-side content hash the caller revalidated.
    pub content_sha256: &'a str,
    pub spec: ScopeSpec<'a>,
    pub reason: &'a str,
    pub actor: &'a str,
    /// The `preview_hash` the reviewer saw.
    pub preview_hash: &'a str,
    /// The reviewer confirmed the effect, not only the button.
    pub confirm: bool,
    /// The reviewer accepted every listed conflict.
    pub acknowledge_conflicts: bool,
}

/// One decision as the history view presents it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecisionRow {
    pub decision_id: i64,
    pub scope: String,
    pub kind: String,
    pub status: String,
    pub document_version_id: i64,
    pub document_ref: String,
    pub content_sha256: String,
    pub person_key: Option<String>,
    pub reason: String,
    pub actor: String,
    pub created_at: String,
    pub retired_at: Option<String>,
    pub reverses_decision_id: Option<i64>,
    pub supersedes_decision_id: Option<i64>,
    pub items: Vec<PreviewItem>,
}

/// The result of an apply or a reversal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DecisionReport {
    pub decision_id: i64,
    pub status: String,
    pub applied: usize,
    pub superseded: usize,
    pub removed: usize,
    pub already_applied: bool,
}

/// A reversal request.
#[derive(Debug, Clone)]
pub struct ReverseInput<'a> {
    pub decision_id: i64,
    pub reason: &'a str,
    pub actor: &'a str,
}

/// Errors raised by the authority workflow.
#[derive(Debug, Error)]
pub enum AuthorityError {
    #[error("no reviewed extraction exists for this document version")]
    NotFound,
    #[error("the document version no longer matches the reviewed content")]
    Stale,
    #[error("{0}")]
    Invalid(String),
    #[error("this change requires explicit acknowledgement of {0} conflict(s)")]
    Conflicts(usize),
    #[error(transparent)]
    Genealogy(#[from] GenealogyError),
}

impl From<postgres::Error> for AuthorityError {
    fn from(err: postgres::Error) -> Self {
        AuthorityError::Genealogy(GenealogyError::Query(err))
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

fn preview_hash(parts: &[String]) -> String {
    format!("{:016x}", fnv1a64(parts.join("\n").as_bytes()))
}

/// The slot an assertion occupies when deciding what a newer source replaces.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Slot {
    Single(i64, String),
    Keyed(i64, String, String),
}

fn slot_of(assertion: &RawAssertion) -> Slot {
    if is_single_valued(&assertion.kind) {
        Slot::Single(assertion.person_id, assertion.kind.clone())
    } else {
        Slot::Keyed(
            assertion.person_id,
            assertion.kind.clone(),
            assertion.key.clone(),
        )
    }
}

#[derive(Debug, Clone)]
struct RawAssertion {
    assertion_id: i64,
    person_id: i64,
    person_key: String,
    display_name: String,
    kind: String,
    key: String,
    value_text: Option<String>,
    review_status: String,
    citations: Vec<Citation>,
}

impl RawAssertion {
    fn view(&self) -> AssertionView {
        AssertionView {
            assertion_id: self.assertion_id,
            person_key: self.person_key.clone(),
            display_name: self.display_name.clone(),
            kind: self.kind.clone(),
            key: self.key.clone(),
            value_text: self.value_text.clone(),
            review_status: self.review_status.clone(),
            citations: self.citations.iter().map(CitationView::from).collect(),
        }
    }
}

const ASSERTION_COLUMNS: &str = "a.assertion_id, a.person_id, p.person_key, \
     COALESCE(NULLIF(BTRIM(p.display_name), ''), 'Unnamed person'), \
     a.assertion_kind, a.assertion_key, a.value_text, a.review_status";

fn load_raw<C: GenericClient>(
    client: &mut C,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
) -> Result<Vec<RawAssertion>, AuthorityError> {
    let rows = client.query(sql, params)?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let assertion_id: i64 = row.try_get(0)?;
        let citations = load_citations(client, assertion_id)?;
        out.push(RawAssertion {
            assertion_id,
            person_id: row.try_get(1)?,
            person_key: row.try_get(2)?,
            display_name: row.try_get(3)?,
            kind: row.try_get(4)?,
            key: row.try_get(5)?,
            value_text: row.try_get(6)?,
            review_status: row.try_get(7)?,
            citations,
        });
    }
    Ok(out)
}

fn load_citations<C: GenericClient>(
    client: &mut C,
    assertion_id: i64,
) -> Result<Vec<Citation>, AuthorityError> {
    let rows = client.query(
        "SELECT document_ref, page, locator, excerpt FROM assertion_evidence
          WHERE assertion_id = $1 AND withdrawn_at IS NULL
          ORDER BY document_ref, page, locator",
        &[&assertion_id],
    )?;
    Ok(rows
        .iter()
        .map(|row| Citation {
            document_ref: row.get(0),
            page: row.get(1),
            locator: row.get(2),
            excerpt: row.get(3),
        })
        .collect())
}

struct StoredExtraction {
    document_ref: String,
    content_sha256: String,
}

fn load_extraction<C: GenericClient>(
    client: &mut C,
    version_id: i64,
) -> Result<StoredExtraction, AuthorityError> {
    let row = client
        .query_opt(
            "SELECT document_source, content_sha256 FROM document_extraction
              WHERE document_version_id = $1
              ORDER BY extraction_id DESC LIMIT 1",
            &[&version_id],
        )?
        .ok_or(AuthorityError::NotFound)?;
    Ok(StoredExtraction {
        document_ref: row.get(0),
        content_sha256: row.get(1),
    })
}

fn load_incoming<C: GenericClient>(
    client: &mut C,
    version_id: i64,
) -> Result<Vec<RawAssertion>, AuthorityError> {
    let sql = format!(
        "SELECT {ASSERTION_COLUMNS}
           FROM assertion a
           JOIN person p ON p.person_id = a.person_id
          WHERE EXISTS (
                SELECT 1 FROM assertion_evidence ae
                 WHERE ae.assertion_id = a.assertion_id
                   AND ae.document_version_id = $1
                   AND ae.withdrawn_at IS NULL)
          ORDER BY p.person_key, a.assertion_kind, a.assertion_key, a.assertion_id"
    );
    load_raw(client, &sql, &[&version_id])
}

/// Loads the current (non-superseded) assertions of the given people, plus the
/// ones the reviewer asked to remove by id.
fn load_current<C: GenericClient>(
    client: &mut C,
    person_ids: &[i64],
    extra_ids: &[i64],
) -> Result<Vec<RawAssertion>, AuthorityError> {
    let sql = format!(
        "SELECT {ASSERTION_COLUMNS}
           FROM assertion a
           JOIN person p ON p.person_id = a.person_id
          WHERE a.superseded_at IS NULL
            AND (a.person_id = ANY($1::bigint[]) OR a.assertion_id = ANY($2::bigint[]))
          ORDER BY p.person_key, a.assertion_kind, a.assertion_key, a.assertion_id"
    );
    load_raw(client, &sql, &[&person_ids.to_vec(), &extra_ids.to_vec()])
}

fn person_id_for<C: GenericClient>(
    client: &mut C,
    person_key: &str,
) -> Result<Option<i64>, AuthorityError> {
    Ok(client
        .query_opt(
            "SELECT person_id FROM person WHERE person_key = $1",
            &[&person_key],
        )?
        .map(|row| row.get(0)))
}

/// Computes the preview for one document version and scope.
pub fn preview<C: GenericClient>(
    client: &mut C,
    version_id: i64,
    content_sha256: &str,
    spec: &ScopeSpec<'_>,
) -> Result<AuthorityPreview, AuthorityError> {
    compute_preview(client, version_id, content_sha256, spec)
}

fn compute_preview<C: GenericClient>(
    client: &mut C,
    version_id: i64,
    content_sha256: &str,
    spec: &ScopeSpec<'_>,
) -> Result<AuthorityPreview, AuthorityError> {
    let scope = Scope::parse(spec.scope)
        .ok_or_else(|| AuthorityError::Invalid("unknown authority scope".to_string()))?;
    let extraction = load_extraction(client, version_id)?;
    if extraction.content_sha256 != content_sha256 {
        return Err(AuthorityError::Stale);
    }

    let all_incoming = load_incoming(client, version_id)?;
    if all_incoming.is_empty() {
        return Err(AuthorityError::Invalid(
            "this document version supports no genealogy assertion".to_string(),
        ));
    }

    let (incoming, _scope_person_id, person_key) = match scope {
        Scope::Document => (all_incoming, None, None),
        Scope::Person => {
            let key = spec
                .person_key
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .ok_or_else(|| {
                    AuthorityError::Invalid("the person scope requires a person key".to_string())
                })?;
            let person_id = person_id_for(client, key)?.ok_or_else(|| {
                AuthorityError::Invalid("the selected person does not exist".to_string())
            })?;
            let filtered: Vec<RawAssertion> = all_incoming
                .into_iter()
                .filter(|assertion| assertion.person_id == person_id)
                .collect();
            if filtered.is_empty() {
                return Err(AuthorityError::Invalid(
                    "this document version says nothing about that person".to_string(),
                ));
            }
            (filtered, Some(person_id), Some(key.to_string()))
        }
        Scope::Assertions => {
            let requested: BTreeSet<i64> = spec.assertion_ids.iter().copied().collect();
            if requested.is_empty() {
                return Err(AuthorityError::Invalid(
                    "select at least one assertion for this scope".to_string(),
                ));
            }
            let filtered: Vec<RawAssertion> = all_incoming
                .into_iter()
                .filter(|assertion| requested.contains(&assertion.assertion_id))
                .collect();
            if filtered.len() != requested.len() {
                return Err(AuthorityError::Invalid(
                    "every selected assertion must come from this document version".to_string(),
                ));
            }
            (filtered, None, None)
        }
    };

    if incoming.len() > MAX_DECISION_ITEMS {
        return Err(AuthorityError::Invalid(format!(
            "one decision may touch at most {MAX_DECISION_ITEMS} assertions"
        )));
    }

    let person_ids: Vec<i64> = {
        let mut ids: Vec<i64> = incoming.iter().map(|row| row.person_id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let current = load_current(client, &person_ids, spec.remove_assertion_ids)?;

    // Map the current assertions by slot so a newer source replaces the value a
    // person already has instead of adding a silent second current value.
    let mut current_by_slot: BTreeMap<Slot, Vec<RawAssertion>> = BTreeMap::new();
    for assertion in &current {
        current_by_slot
            .entry(slot_of(assertion))
            .or_default()
            .push(assertion.clone());
    }
    let current_by_id: BTreeMap<i64, RawAssertion> = current
        .iter()
        .map(|assertion| (assertion.assertion_id, assertion.clone()))
        .collect();

    // Only the kinds the document speaks about for a person can be omissions;
    // a document about one person never offers to remove another person's
    // unrelated facts.
    let mut kinds_by_person: BTreeMap<i64, BTreeSet<String>> = BTreeMap::new();
    for assertion in &incoming {
        kinds_by_person
            .entry(assertion.person_id)
            .or_default()
            .insert(assertion.kind.clone());
    }

    let incoming_ids: BTreeSet<i64> = incoming.iter().map(|row| row.assertion_id).collect();
    let incoming_slots: BTreeSet<Slot> = incoming.iter().map(slot_of).collect();

    let mut removals: Vec<RawAssertion> = Vec::new();
    for id in spec.remove_assertion_ids {
        let Some(assertion) = current_by_id.get(id) else {
            return Err(AuthorityError::Invalid(format!(
                "assertion {id} is not a current assertion inside this scope"
            )));
        };
        if !person_ids.contains(&assertion.person_id) {
            return Err(AuthorityError::Invalid(format!(
                "assertion {id} is outside this document's scope"
            )));
        }
        if incoming_slots.contains(&slot_of(assertion)) {
            return Err(AuthorityError::Invalid(format!(
                "assertion {id} is already replaced by this document; remove it is redundant"
            )));
        }
        removals.push(assertion.clone());
    }
    let removal_ids: BTreeSet<i64> = removals.iter().map(|row| row.assertion_id).collect();

    let mut items: Vec<PreviewItem> = Vec::new();
    let mut conflicts: Vec<Conflict> = Vec::new();
    let mut superseded_ids: BTreeSet<i64> = BTreeSet::new();

    for assertion in &incoming {
        let slot = slot_of(assertion);
        let self_is_current = current_by_id.contains_key(&assertion.assertion_id);
        // Everything else already current in the same slot is what this
        // assertion would replace. The incoming row itself is never its own
        // replacement.
        let occupying: Vec<RawAssertion> = current_by_slot
            .get(&slot)
            .map(|rows| {
                rows.iter()
                    .filter(|row| row.assertion_id != assertion.assertion_id)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        if occupying.is_empty() && self_is_current {
            items.push(PreviewItem {
                action: Action::Unchanged.as_str().to_string(),
                incoming: Some(assertion.view()),
                replaces: Vec::new(),
            });
            continue;
        }
        let action = if occupying.is_empty() {
            Action::Added
        } else {
            Action::Replaced
        };
        if action == Action::Replaced {
            for previous in &occupying {
                superseded_ids.insert(previous.assertion_id);
                if let Some(code) = conflict_code(&previous.review_status, "replaces") {
                    conflicts.push(Conflict {
                        code,
                        assertion: previous.view(),
                    });
                }
            }
        }
        items.push(PreviewItem {
            action: action.as_str().to_string(),
            incoming: Some(assertion.view()),
            replaces: occupying.iter().map(RawAssertion::view).collect(),
        });
    }

    for removal in &removals {
        superseded_ids.insert(removal.assertion_id);
        if let Some(code) = conflict_code(&removal.review_status, "removes") {
            conflicts.push(Conflict {
                code,
                assertion: removal.view(),
            });
        }
        items.push(PreviewItem {
            action: Action::Removed.as_str().to_string(),
            incoming: None,
            replaces: vec![removal.view()],
        });
    }

    let omissions: Vec<AssertionView> = current
        .iter()
        .filter(|assertion| {
            !incoming_ids.contains(&assertion.assertion_id)
                && !removal_ids.contains(&assertion.assertion_id)
                && kinds_by_person
                    .get(&assertion.person_id)
                    .is_some_and(|kinds| kinds.contains(&assertion.kind))
                && !incoming_slots.contains(&slot_of(assertion))
        })
        .map(RawAssertion::view)
        .collect();

    // The token covers the inputs *and* every derived effect, so a concurrent
    // change to a slot, an omission, or a conflict invalidates the preview.
    let mut parts: Vec<String> = vec![
        "authority-preview-v1".to_string(),
        version_id.to_string(),
        extraction.content_sha256.clone(),
        scope.as_str().to_string(),
        person_key.clone().unwrap_or_default(),
    ];
    let mut incoming_sorted: Vec<i64> = incoming_ids.iter().copied().collect();
    incoming_sorted.sort_unstable();
    parts.push(
        incoming_sorted
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    let mut superseded_sorted: Vec<i64> = superseded_ids.iter().copied().collect();
    superseded_sorted.sort_unstable();
    parts.push(
        superseded_sorted
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    let mut omission_sorted: Vec<i64> = omissions.iter().map(|view| view.assertion_id).collect();
    omission_sorted.sort_unstable();
    parts.push(
        omission_sorted
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
    );
    let mut conflict_keys: Vec<String> = conflicts
        .iter()
        .map(|conflict| format!("{}:{}", conflict.code, conflict.assertion.assertion_id))
        .collect();
    conflict_keys.sort();
    parts.push(conflict_keys.join(","));

    // Sort conflicts by the assertion they concern so the browser order is stable.
    conflicts.sort_by(|left, right| {
        left.assertion
            .person_key
            .cmp(&right.assertion.person_key)
            .then_with(|| {
                left.assertion
                    .assertion_id
                    .cmp(&right.assertion.assertion_id)
            })
    });

    Ok(AuthorityPreview {
        document_version_id: version_id,
        document_ref: extraction.document_ref,
        content_sha256: extraction.content_sha256,
        scope: scope.as_str().to_string(),
        person_key,
        items,
        omissions,
        conflicts,
        preview_hash: preview_hash(&parts),
    })
}

fn conflict_code(review_status: &str, verb: &str) -> Option<String> {
    match review_status {
        "verified" => Some(format!("{verb}_verified")),
        "disputed" => Some(format!("{verb}_disputed")),
        _ => None,
    }
}

/// Designates one document version as the current source of truth.
///
/// The preview is recomputed inside the transaction and must match the token the
/// reviewer confirmed. The decision row and every supersession are written
/// together, so a partial apply cannot exist.
pub fn apply(client: &mut Client, input: ApplyInput<'_>) -> Result<DecisionReport, AuthorityError> {
    let reason = input.reason.trim();
    if !input.confirm {
        return Err(AuthorityError::Invalid(
            "the decision requires explicit confirmation".to_string(),
        ));
    }
    if reason.is_empty() {
        return Err(AuthorityError::Invalid(
            "a reason is required for an authoritative decision".to_string(),
        ));
    }
    if reason.chars().count() > 500 {
        return Err(AuthorityError::Invalid(
            "the reason must be at most 500 characters".to_string(),
        ));
    }
    if input.actor.trim().is_empty() {
        return Err(AuthorityError::Invalid("an actor is required".to_string()));
    }

    let mut tx = client.transaction()?;
    lock_target(
        &mut tx,
        input.document_version_id,
        input.spec.scope,
        input.spec.person_key,
    )?;

    // First pass: find the rows to lock. The second pass, taken after the locks
    // are held, is the one that decides; a concurrent writer that changed a row
    // we did not lock makes the second pass disagree with the reviewer's token.
    let hint = compute_preview(
        &mut tx,
        input.document_version_id,
        input.content_sha256,
        &input.spec,
    )?;
    let hint_touched = touched_assertions(&hint);
    // Lock every touched assertion row so two reviewers cannot race on the same
    // slot, and so a concurrent verification cannot slip between the preview and
    // the write.
    tx.query(
        "SELECT assertion_id FROM assertion
          WHERE assertion_id = ANY($1::bigint[]) ORDER BY assertion_id FOR UPDATE",
        &[&hint_touched],
    )?;

    let expected = compute_preview(
        &mut tx,
        input.document_version_id,
        input.content_sha256,
        &input.spec,
    )?;
    if expected.preview_hash != input.preview_hash {
        return Err(AuthorityError::Stale);
    }
    if !expected.conflicts.is_empty() && !input.acknowledge_conflicts {
        return Err(AuthorityError::Conflicts(expected.conflicts.len()));
    }

    let touched = touched_assertions(&expected);

    let decision_id = reserve_decision_id(&mut tx)?;
    // Retire first, with the reserved id recorded as the retiring decision, so
    // the partial unique index never sees two active decisions for one target.
    retire_overlapping(
        &mut tx,
        decision_id,
        input.document_version_id,
        input.spec.scope,
        expected.person_key.as_deref(),
        &touched,
    )?;
    insert_decision(
        &mut tx,
        decision_id,
        input.document_version_id,
        &expected,
        input.spec.scope,
        reason,
        input.actor,
        input.preview_hash,
    )?;

    let mut applied = 0usize;
    let mut superseded = 0usize;
    let mut removed = 0usize;
    for item in &expected.items {
        let action = Action::parse(&item.action)
            .ok_or_else(|| AuthorityError::Invalid("unknown preview action".to_string()))?;
        match action {
            Action::Removed => {
                for previous in &item.replaces {
                    supersede(&mut tx, previous.assertion_id, decision_id)?;
                    superseded += 1;
                }
                if let Some(previous) = item.replaces.first() {
                    insert_item(&mut tx, decision_id, previous.assertion_id, None, action)?;
                }
                removed += 1;
            }
            Action::Unchanged => {
                if let Some(incoming) = &item.incoming {
                    insert_item(&mut tx, decision_id, incoming.assertion_id, None, action)?;
                }
            }
            Action::Added | Action::Replaced => {
                let incoming = item
                    .incoming
                    .as_ref()
                    .ok_or_else(|| AuthorityError::Invalid("missing assertion".to_string()))?;
                let first_replaced = item.replaces.first().map(|row| row.assertion_id);
                for previous in &item.replaces {
                    supersede(&mut tx, previous.assertion_id, decision_id)?;
                    superseded += 1;
                }
                restore(&mut tx, incoming.assertion_id)?;
                insert_item(
                    &mut tx,
                    decision_id,
                    incoming.assertion_id,
                    first_replaced,
                    action,
                )?;
                applied += 1;
            }
        }
    }

    tx.execute(
        "INSERT INTO authority_decision_event (decision_id, action, actor, detail)
         VALUES ($1, 'applied', $2, $3::text::jsonb)",
        &[
            &decision_id,
            &input.actor,
            &serde_json::json!({
                "scope": expected.scope,
                "document_version_id": input.document_version_id,
                "preview_hash": input.preview_hash,
                "applied": applied,
                "superseded": superseded,
                "removed": removed,
                "conflicts": expected.conflicts.len(),
            })
            .to_string(),
        ],
    )?;
    tx.commit()?;

    Ok(DecisionReport {
        decision_id,
        status: "active".to_string(),
        applied,
        superseded,
        removed,
        already_applied: false,
    })
}

/// Takes a transaction-scoped advisory lock per document version, so a second
/// reviewer cannot interleave a preview-then-apply sequence for the same source.
fn lock_target<C: GenericClient>(
    client: &mut C,
    version_id: i64,
    _scope: &str,
    _person_key: Option<&str>,
) -> Result<(), AuthorityError> {
    let key = format!("authority-source:{version_id}");
    client.execute("SELECT pg_advisory_xact_lock(hashtext($1))", &[&key])?;
    Ok(())
}

/// Every assertion a preview would write.
fn touched_assertions(preview: &AuthorityPreview) -> Vec<i64> {
    let mut ids: BTreeSet<i64> = BTreeSet::new();
    for item in &preview.items {
        if let Some(incoming) = &item.incoming {
            ids.insert(incoming.assertion_id);
        }
        for replaced in &item.replaces {
            ids.insert(replaced.assertion_id);
        }
    }
    ids.into_iter().collect()
}

/// Reserves the next decision id so a decision can be retired *before* the new
/// row exists, which is what keeps the partial unique index satisfied.
fn reserve_decision_id<C: GenericClient>(client: &mut C) -> Result<i64, AuthorityError> {
    Ok(client
        .query_one("SELECT nextval('authority_decision_decision_id_seq')", &[])?
        .get(0))
}

#[allow(clippy::too_many_arguments)]
fn insert_decision<C: GenericClient>(
    client: &mut C,
    decision_id: i64,
    version_id: i64,
    preview: &AuthorityPreview,
    scope: &str,
    reason: &str,
    actor: &str,
    preview_hash: &str,
) -> Result<(), AuthorityError> {
    let person_id: Option<i64> = match preview.person_key.as_deref() {
        Some(key) => person_id_for(client, key)?,
        None => None,
    };
    // The active decision on the same target, if any, was retired immediately
    // before this insert. The chain is recorded so the history stays readable.
    let previous: Option<i64> = client
        .query_opt(
            "SELECT decision_id FROM authority_decision
              WHERE status = 'superseded' AND retired_by_decision_id = $1
                AND scope = $2 AND document_version_id = $3
                AND scope_person_id IS NOT DISTINCT FROM $4
              ORDER BY decision_id DESC LIMIT 1",
            &[&decision_id, &scope, &version_id, &person_id],
        )?
        .map(|row| row.get(0));
    client.execute(
        "INSERT INTO authority_decision
             (decision_id, scope, document_version_id, document_ref, content_sha256,
              scope_person_id, kind, reason, actor, preview_hash, status,
              supersedes_decision_id)
         VALUES ($1, $2, $3, $4, $5, $6, 'designate', $7, $8, $9, 'active', $10)",
        &[
            &decision_id,
            &scope,
            &version_id,
            &preview.document_ref,
            &preview.content_sha256,
            &person_id,
            &reason,
            &actor,
            &preview_hash,
            &previous,
        ],
    )?;
    Ok(())
}

fn insert_item<C: GenericClient>(
    client: &mut C,
    decision_id: i64,
    assertion_id: i64,
    superseded_assertion_id: Option<i64>,
    action: Action,
) -> Result<(), AuthorityError> {
    client.execute(
        "INSERT INTO authority_decision_item
             (decision_id, assertion_id, superseded_assertion_id, action)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (decision_id, assertion_id) DO NOTHING",
        &[
            &decision_id,
            &assertion_id,
            &superseded_assertion_id,
            &action.as_str(),
        ],
    )?;
    Ok(())
}

fn supersede<C: GenericClient>(
    client: &mut C,
    assertion_id: i64,
    decision_id: i64,
) -> Result<(), AuthorityError> {
    client.execute(
        "UPDATE assertion
            SET superseded_at = now(), superseded_by_decision_id = $2
          WHERE assertion_id = $1 AND superseded_at IS NULL",
        &[&assertion_id, &decision_id],
    )?;
    Ok(())
}

fn restore<C: GenericClient>(client: &mut C, assertion_id: i64) -> Result<(), AuthorityError> {
    client.execute(
        "UPDATE assertion
            SET superseded_at = NULL, superseded_by_decision_id = NULL
          WHERE assertion_id = $1 AND superseded_at IS NOT NULL",
        &[&assertion_id],
    )?;
    Ok(())
}

/// Retires any other active decision that governs an assertion this decision
/// touched, plus the previous active decision on the same target. Retirement is
/// recorded, never deleted.
fn retire_overlapping<C: GenericClient>(
    client: &mut C,
    decision_id: i64,
    version_id: i64,
    scope: &str,
    person_key: Option<&str>,
    touched: &[i64],
) -> Result<(), AuthorityError> {
    let person_id = match person_key {
        Some(key) => person_id_for(client, key)?,
        None => None,
    };
    client.execute(
        "UPDATE authority_decision d
            SET status = 'superseded', retired_at = now(), retired_by_decision_id = $1
          WHERE d.decision_id <> $1
            AND d.status = 'active'
            AND (
              EXISTS (
                SELECT 1 FROM authority_decision_item i
                 WHERE i.decision_id = d.decision_id
                   AND i.assertion_id = ANY($5::bigint[]))
              OR (
                d.scope = $3
                AND d.document_version_id = $2
                AND d.scope_person_id IS NOT DISTINCT FROM $4
              )
            )",
        &[
            &decision_id,
            &version_id,
            &scope,
            &person_id,
            &touched.to_vec(),
        ],
    )?;
    Ok(())
}

/// Reverses one active decision through a new, audited decision.
///
/// The reversed decision keeps its rows. What it made current is superseded and
/// what it replaced is restored, so current views return to the prior state.
pub fn reverse(
    client: &mut Client,
    input: ReverseInput<'_>,
) -> Result<DecisionReport, AuthorityError> {
    let reason = input.reason.trim();
    if reason.is_empty() {
        return Err(AuthorityError::Invalid(
            "a reason is required to reverse a decision".to_string(),
        ));
    }
    if reason.chars().count() > 500 {
        return Err(AuthorityError::Invalid(
            "the reason must be at most 500 characters".to_string(),
        ));
    }
    if input.actor.trim().is_empty() {
        return Err(AuthorityError::Invalid("an actor is required".to_string()));
    }

    let mut tx = client.transaction()?;
    let target = tx.query_opt(
        "SELECT document_version_id, scope FROM authority_decision WHERE decision_id = $1",
        &[&input.decision_id],
    )?;
    // Take the advisory lock first, matching the apply order, so the two paths
    // cannot deadlock over the same rows.
    if let Some(target) = &target {
        let version_id: i64 = target.get(0);
        let scope: String = target.get(1);
        lock_target(&mut tx, version_id, &scope, None)?;
    }
    let row = tx
        .query_opt(
            "SELECT scope, document_version_id, document_ref, content_sha256,
                    scope_person_id, preview_hash
               FROM authority_decision
              WHERE decision_id = $1
              FOR UPDATE",
            &[&input.decision_id],
        )?
        .ok_or(AuthorityError::NotFound)?;
    let status: String = tx
        .query_one(
            "SELECT status FROM authority_decision WHERE decision_id = $1",
            &[&input.decision_id],
        )?
        .get(0);
    if status != "active" {
        return Err(AuthorityError::Invalid(
            "only the active decision can be reversed".to_string(),
        ));
    }
    let scope: String = row.get(0);
    let version_id: i64 = row.get(1);
    let document_ref: String = row.get(2);
    let content_sha256: String = row.get(3);
    let scope_person_id: Option<i64> = row.get(4);
    let _previous_hash: String = row.get(5);

    let items = tx.query(
        "SELECT assertion_id, superseded_assertion_id, action
           FROM authority_decision_item WHERE decision_id = $1
          ORDER BY item_id",
        &[&input.decision_id],
    )?;
    let mut plan: Vec<(Action, i64, Option<i64>)> = Vec::new();
    for item in &items {
        let assertion_id: i64 = item.get(0);
        let superseded_id: Option<i64> = item.get(1);
        let action = Action::parse(&item.get::<_, String>(2))
            .ok_or_else(|| AuthorityError::Invalid("unknown stored action".to_string()))?;
        match action {
            Action::Added | Action::Replaced => {
                plan.push((Action::Removed, assertion_id, None));
                if let Some(previous) = superseded_id {
                    plan.push((Action::Replaced, previous, Some(assertion_id)));
                }
            }
            Action::Removed => {
                if superseded_id.is_none() {
                    plan.push((Action::Added, assertion_id, None));
                }
            }
            Action::Unchanged => {}
        }
    }

    let touched: Vec<i64> = {
        let mut ids: BTreeSet<i64> = BTreeSet::new();
        for (_, assertion_id, superseded) in &plan {
            ids.insert(*assertion_id);
            if let Some(previous) = superseded {
                ids.insert(*previous);
            }
        }
        ids.into_iter().collect()
    };
    tx.query(
        "SELECT assertion_id FROM assertion
          WHERE assertion_id = ANY($1::bigint[]) ORDER BY assertion_id FOR UPDATE",
        &[&touched],
    )?;

    let reversal_hash = preview_hash(&[
        "authority-reversal-v1".to_string(),
        input.decision_id.to_string(),
        touched
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(","),
    ]);
    let reversal_id = reserve_decision_id(&mut tx)?;
    // Retire the reversed decision before inserting the reversal, so the partial
    // unique index never sees two active decisions for one target.
    tx.execute(
        "UPDATE authority_decision
            SET status = 'reversed', retired_at = now(), retired_by_decision_id = $2
          WHERE decision_id = $1",
        &[&input.decision_id, &reversal_id],
    )?;
    tx.execute(
        "INSERT INTO authority_decision
             (decision_id, scope, document_version_id, document_ref, content_sha256,
              scope_person_id, kind, reason, actor, preview_hash, status,
              reverses_decision_id)
         VALUES ($1, $2, $3, $4, $5, $6, 'reversal', $7, $8, $9, 'active', $10)",
        &[
            &reversal_id,
            &scope,
            &version_id,
            &document_ref,
            &content_sha256,
            &scope_person_id,
            &reason,
            &input.actor,
            &reversal_hash,
            &input.decision_id,
        ],
    )?;

    let mut applied = 0usize;
    let mut superseded = 0usize;
    let mut removed = 0usize;
    for (action, assertion_id, previous) in &plan {
        match action {
            Action::Removed => {
                supersede(&mut tx, *assertion_id, reversal_id)?;
                insert_item(&mut tx, reversal_id, *assertion_id, None, *action)?;
                removed += 1;
            }
            Action::Added | Action::Replaced => {
                restore(&mut tx, *assertion_id)?;
                insert_item(&mut tx, reversal_id, *assertion_id, *previous, *action)?;
                superseded += 1;
                applied += 1;
            }
            Action::Unchanged => {}
        }
    }

    tx.execute(
        "INSERT INTO authority_decision_event (decision_id, action, actor, detail)
         VALUES ($1, 'reversed', $2, $3::text::jsonb)",
        &[
            &input.decision_id,
            &input.actor,
            &serde_json::json!({
                "reversal_decision_id": reversal_id,
                "reason": reason,
            })
            .to_string(),
        ],
    )?;
    tx.commit()?;

    Ok(DecisionReport {
        decision_id: reversal_id,
        status: "active".to_string(),
        applied,
        superseded,
        removed,
        already_applied: false,
    })
}

/// Lists decisions newest first, optionally filtered to one person.
///
/// The rows are the authorized history view: they show every version, the
/// reviewer, the reason, and what each decision replaced.
pub fn list_decisions(
    client: &mut Client,
    person_key: Option<&str>,
) -> Result<Vec<DecisionRow>, AuthorityError> {
    let person_id = match person_key.map(str::trim).filter(|key| !key.is_empty()) {
        Some(key) => Some(person_id_for(client, key)?.ok_or_else(|| {
            AuthorityError::Invalid("the selected person does not exist".to_string())
        })?),
        None => None,
    };
    let rows = client.query(
        "SELECT d.decision_id, d.scope, d.kind, d.status, d.document_version_id,
                d.document_ref, d.content_sha256, p.person_key, d.reason, d.actor,
                d.created_at::text, d.retired_at::text, d.reverses_decision_id,
                d.supersedes_decision_id
           FROM authority_decision d
           LEFT JOIN person p ON p.person_id = d.scope_person_id
          WHERE $1::bigint IS NULL
             OR d.scope_person_id = $1
             OR EXISTS (
                  SELECT 1 FROM authority_decision_item i
                    JOIN assertion a ON a.assertion_id = i.assertion_id
                   WHERE i.decision_id = d.decision_id AND a.person_id = $1)
          ORDER BY d.decision_id DESC
          LIMIT 100",
        &[&person_id],
    )?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let decision_id: i64 = row.get(0);
        let items = load_decision_items(client, decision_id)?;
        out.push(DecisionRow {
            decision_id,
            scope: row.get(1),
            kind: row.get(2),
            status: row.get(3),
            document_version_id: row.get(4),
            document_ref: row.get(5),
            content_sha256: row.get(6),
            person_key: row.get(7),
            reason: row.get(8),
            actor: row.get(9),
            created_at: row.get(10),
            retired_at: row.get(11),
            reverses_decision_id: row.get(12),
            supersedes_decision_id: row.get(13),
            items,
        });
    }
    Ok(out)
}

fn load_decision_items(
    client: &mut Client,
    decision_id: i64,
) -> Result<Vec<PreviewItem>, AuthorityError> {
    let rows = client.query(
        "SELECT i.action, i.assertion_id, i.superseded_assertion_id
           FROM authority_decision_item i
          WHERE i.decision_id = $1
          ORDER BY i.item_id",
        &[&decision_id],
    )?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let action: String = row.get(0);
        let is_removal = action == "removed";
        let assertion_id: i64 = row.get(1);
        let superseded_id: Option<i64> = row.get(2);
        let incoming = load_assertion_view(client, assertion_id)?;
        let replaces = match superseded_id {
            Some(id) => load_assertion_view(client, id)?.into_iter().collect(),
            None => Vec::new(),
        };
        out.push(PreviewItem {
            action,
            incoming: if is_removal { None } else { incoming },
            replaces,
        });
    }
    Ok(out)
}

fn load_assertion_view(
    client: &mut Client,
    assertion_id: i64,
) -> Result<Option<AssertionView>, AuthorityError> {
    let sql = format!(
        "SELECT {ASSERTION_COLUMNS}
           FROM assertion a
           JOIN person p ON p.person_id = a.person_id
          WHERE a.assertion_id = $1"
    );
    let rows = client.query(&sql, &[&assertion_id])?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let citations = load_citations(client, assertion_id)?;
    Ok(Some(AssertionView {
        assertion_id: row.try_get(0)?,
        person_key: row.try_get(2)?,
        display_name: row.try_get(3)?,
        kind: row.try_get(4)?,
        key: row.try_get(5)?,
        value_text: row.try_get(6)?,
        review_status: row.try_get(7)?,
        citations: citations.iter().map(CitationView::from).collect(),
    }))
}

/// True when the assertion is the current one for its slot.
pub fn is_current<C: GenericClient>(
    client: &mut C,
    assertion_id: i64,
) -> Result<bool, AuthorityError> {
    let row = client.query_opt(
        "SELECT superseded_at IS NULL FROM assertion WHERE assertion_id = $1",
        &[&assertion_id],
    )?;
    Ok(row.map(|row| row.get(0)).unwrap_or(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kind: &str, key: &str, person: i64) -> RawAssertion {
        RawAssertion {
            assertion_id: 1,
            person_id: person,
            person_key: "P1".to_string(),
            display_name: "Name".to_string(),
            kind: kind.to_string(),
            key: key.to_string(),
            value_text: None,
            review_status: "unverified".to_string(),
            citations: Vec::new(),
        }
    }

    #[test]
    fn single_valued_kinds_share_one_slot() {
        assert_eq!(
            slot_of(&raw("display_name", "display:aaaa", 1)),
            slot_of(&raw("display_name", "display:bbbb", 1))
        );
        assert_eq!(
            slot_of(&raw("birth_date", "date:1900", 1)),
            slot_of(&raw("birth_date", "date:1901", 1))
        );
    }

    #[test]
    fn multi_valued_kinds_keep_distinct_slots() {
        assert_ne!(
            slot_of(&raw("name_variant", "variant:aaaa", 1)),
            slot_of(&raw("name_variant", "variant:bbbb", 1))
        );
        assert_ne!(
            slot_of(&raw("parent_child", "parent_child:aaaa", 1)),
            slot_of(&raw("parent_child", "parent_child:bbbb", 1))
        );
    }

    #[test]
    fn slots_do_not_cross_people() {
        assert_ne!(
            slot_of(&raw("display_name", "display:aaaa", 1)),
            slot_of(&raw("display_name", "display:aaaa", 2))
        );
    }

    #[test]
    fn preview_hash_is_stable_and_order_independent_for_the_inputs() {
        let first = preview_hash(&["a".into(), "b".into()]);
        let second = preview_hash(&["a".into(), "b".into()]);
        assert_eq!(first, second);
        assert_ne!(first, preview_hash(&["b".into(), "a".into()]));
    }

    #[test]
    fn reviewed_current_assertions_are_conflicts() {
        assert_eq!(
            conflict_code("verified", "replaces").as_deref(),
            Some("replaces_verified")
        );
        assert_eq!(
            conflict_code("disputed", "removes").as_deref(),
            Some("removes_disputed")
        );
        assert_eq!(conflict_code("unverified", "replaces"), None);
    }
}
