//! Deterministic person lookup for "Ask the archive" identity questions.
//!
//! The document retrieval path answers from approved RAG chunks only; it never
//! consults the genealogy store. When a family member asks about a person by
//! name, this module resolves that name against the authorized genealogy
//! catalog and answers from stored assertions alone.
//!
//! Matching is evidence-ordered and never merges identities:
//!
//! - accent and case folding make `Hân` and `Han` comparable, and `Đ/đ` folds
//!   to `d`;
//! - name tokens are compared as an unordered set, so Vietnamese
//!   `Family Middle Given` and Western `Given Middle Family` orderings agree;
//! - an exact folded name outranks a recorded alias, which outranks a
//!   reordered match, which outranks a partial token match;
//! - more than one person at the best rank is reported as ambiguous, so
//!   namesakes are chosen by a human and never merged;
//! - a match is a candidate, not proof. The caller must present the match kind
//!   and the recorded review status, and must not alter records.
//!
//! Every read targets the separate genealogy database and returns stored
//! statuses verbatim. Nothing here writes, promotes, links, or approves.

use std::collections::{BTreeMap, BTreeSet};

use postgres::Client;
use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

use super::query::{lookup_person, AssertionRow, PersonDetail};
use super::GenealogyError;

/// Upper bound on catalog rows one person question loads. The family catalog is
/// small; a larger store is reported as truncated rather than silently searched
/// only in part.
pub const MAX_SEARCH_PEOPLE: i64 = 2_000;

/// The largest selection step the API returns. When more people tie at the best
/// rank, the payload says so instead of pretending the list is complete.
pub const MAX_CANDIDATES: usize = 10;

/// One cataloged person with every recorded spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonNames {
    /// Stable person key.
    pub person_key: String,
    /// The recorded display name.
    pub display_name: String,
    /// The recorded generation, when the catalog stored one.
    pub generation: Option<i32>,
    /// Recorded `name_variant` spellings from the assertion ledger.
    pub aliases: Vec<String>,
    /// `verified`, `needs_review`, or `unverified`, derived exactly as the tree
    /// read derives it.
    pub evidence_status: String,
    /// The recorded birth year, when a current verified birth assertion
    /// records one. `None` when no such assertion exists; never a default.
    pub birth_year: Option<i32>,
    /// The recorded family branch label/key, when a current verified
    /// branch claim records one. `None` otherwise.
    pub branch: Option<String>,
}

/// The authorized catalog projection used for matching.
#[derive(Debug, Clone)]
pub struct Catalog {
    /// Cataloged people, ordered by person key.
    pub people: Vec<PersonNames>,
    /// True when the catalog holds more rows than this read returned.
    pub truncated: bool,
}

/// How a query name matched one recorded spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchKind {
    /// The folded query equals the folded display name.
    Exact,
    /// The folded query equals a recorded name variant.
    Alias,
    /// The folded query has the same tokens in a different order.
    Reordered,
    /// Every query token is recorded, and the recorded name has more tokens.
    Partial,
}

impl Candidate {
    fn rank(&self) -> u8 {
        self.match_kind.map(MatchKind::rank).unwrap_or(0)
    }
}

impl MatchKind {
    /// The stable wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            MatchKind::Exact => "exact",
            MatchKind::Alias => "alias",
            MatchKind::Reordered => "reordered",
            MatchKind::Partial => "partial",
        }
    }

    /// The comparison rank. An exact spelling and a recorded alias share the
    /// best rank so a namesake pair split across the two is still ambiguous.
    fn rank(self) -> u8 {
        match self {
            MatchKind::Exact | MatchKind::Alias => 1,
            MatchKind::Reordered => 2,
            MatchKind::Partial => 3,
        }
    }
}

/// A recorded parent link, with the status the graph read assigns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParentRef {
    /// Stable parent key.
    pub person_key: String,
    /// The parent's recorded display name.
    pub display_name: String,
    /// `verified`, `needs_review`, or `unverified`.
    pub evidence_status: String,
}

/// One person that matched the question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Stable person key.
    pub person_key: String,
    /// The recorded display name.
    pub display_name: String,
    /// The recorded generation, when present.
    pub generation: Option<i32>,
    /// `verified`, `needs_review`, or `unverified`.
    pub evidence_status: String,
    /// How the query matched this person. `None` when the caller selected the
    /// person by key and the question text did not rank them.
    pub match_kind: Option<MatchKind>,
    /// The recorded spelling that matched, if one did.
    pub matched_name: Option<String>,
    /// Recorded parents, used only to distinguish namesakes.
    pub recorded_parents: Vec<ParentRef>,
    /// The recorded birth year, when a current verified birth assertion
    /// records one. `None` when no such assertion exists; never a default.
    pub birth_year: Option<i32>,
    /// The recorded family branch label/key, when a current verified
    /// branch claim records one. `None` otherwise.
    pub branch: Option<String>,
}

/// How the question referred to a person.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PersonQuestionKind {
    /// `who is …` / `ai là …`: an identity question even when nothing matches.
    Identity,
    /// `tell me about …`: a person question only when a recorded name matches.
    Topic,
    /// A bare phrase with no identity wording: only a strong match counts.
    Bare,
}

/// A question that names a person.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonQuestion {
    /// The name text the question supplied.
    pub name: String,
    /// How the question referred to the person.
    pub kind: PersonQuestionKind,
}

/// The deterministic outcome of a person question.
#[derive(Debug, Clone)]
pub enum PersonResult {
    /// The question is not about a named person. The document path answers it.
    NotPersonQuestion,
    /// No authorized person matches the recorded name.
    NoMatch {
        query_name: String,
        /// True when the catalog holds no people at all, which is an operator
        /// condition rather than an absent person.
        catalog_empty: bool,
        catalog_truncated: bool,
    },
    /// Several people tie at the best match rank. A human must choose.
    Ambiguous {
        query_name: String,
        candidates: Vec<Candidate>,
        /// True when more candidates exist than `candidates` contains.
        more: bool,
        catalog_truncated: bool,
    },
    /// Exactly one person matched, or the caller selected one.
    Found {
        query_name: String,
        candidate: Box<Candidate>,
        detail: PersonDetail,
        /// True when the person has only name assertions, so no further
        /// detail can be stated.
        name_only: bool,
        /// True when the catalog holds more people than this read searched, so
        /// a namesake outside the bound cannot be ruled out.
        catalog_truncated: bool,
    },
}

/// Folds a name for comparison: lower case, Unicode decomposition with every
/// combining mark removed, `đ` written as `d`, and every other non-alphanumeric
/// character treated as a token separator.
pub fn fold(name: &str) -> String {
    let lowered = name.to_lowercase();
    let mut out = String::with_capacity(lowered.len());
    let mut separator = false;
    for ch in lowered.nfd() {
        if is_combining_mark(ch) {
            continue;
        }
        let mapped = match ch {
            'đ' => 'd',
            c if c.is_alphanumeric() => c,
            _ => ' ',
        };
        if mapped == ' ' {
            if !out.is_empty() {
                separator = true;
            }
            continue;
        }
        if separator {
            out.push(' ');
            separator = false;
        }
        out.push(mapped);
    }
    out
}

fn tokens(folded: &str) -> Vec<&str> {
    folded.split_whitespace().collect()
}

/// Words that never form a person name. They keep ordinary archive questions on
/// the document path instead of turning them into identity lookups.
const STOPWORDS: &[&str] = &[
    "a",
    "an",
    "ancestor",
    "ancestors",
    "anyone",
    "are",
    "archive",
    "branch",
    "branches",
    "called",
    "child",
    "children",
    "did",
    "do",
    "does",
    "earliest",
    "everyone",
    "family",
    "father",
    "first",
    "genealogy",
    "grandfather",
    "grandmother",
    "head",
    "her",
    "his",
    "history",
    "how",
    "husband",
    "is",
    "last",
    "mother",
    "my",
    "named",
    "our",
    "parent",
    "parents",
    "people",
    "person",
    "recorded",
    "relation",
    "relations",
    "someone",
    "story",
    "that",
    "the",
    "their",
    "these",
    "this",
    "those",
    "tree",
    "was",
    "we",
    "were",
    "what",
    "when",
    "where",
    "which",
    "who",
    "why",
    "wife",
    "your",
];

/// Identity phrasings. Matching is ASCII case-insensitive at the head of the
/// question; the extracted remainder must still look like a name.
const IDENTITY_PREFIXES: &[&str] = &["who is ", "who was ", "who's ", "whos ", "ai là ", "ai la "];

/// Broader phrasings that may or may not be about a person. These only use the
/// person path when a recorded name actually matches, so a question such as
/// "tell me about land records" keeps its document answer.
const TOPIC_PREFIXES: &[&str] = &[
    "tell me about ",
    "what do you know about ",
    "what do we know about ",
    "thông tin về ",
    "thong tin ve ",
];

#[derive(Debug)]
struct NameMatch {
    kind: MatchKind,
    matched_name: String,
}

fn match_name(
    folded_query: &str,
    query_tokens: &[&str],
    query_set: &BTreeSet<&str>,
    name: &str,
    is_alias: bool,
) -> Option<NameMatch> {
    let folded = fold(name);
    if folded.is_empty() {
        return None;
    }
    let kind = if folded == folded_query {
        if is_alias {
            MatchKind::Alias
        } else {
            MatchKind::Exact
        }
    } else {
        let recorded = tokens(&folded);
        let recorded_set: BTreeSet<&str> = recorded.iter().copied().collect();
        if recorded_set == *query_set {
            MatchKind::Reordered
        } else if query_tokens.len() >= 2
            && query_set.len() == query_tokens.len()
            && query_set.is_subset(&recorded_set)
            && recorded_set.len() > query_set.len()
        {
            // Dropping a middle name leaves a real but weak candidate. It is
            // never treated as proof of identity.
            MatchKind::Partial
        } else {
            return None;
        }
    };
    Some(NameMatch {
        kind,
        matched_name: name.to_string(),
    })
}

fn best_match(
    person: &PersonNames,
    folded_query: &str,
    query_tokens: &[&str],
    query_set: &BTreeSet<&str>,
) -> Option<NameMatch> {
    let mut best: Option<NameMatch> = None;
    let names = std::iter::once((person.display_name.as_str(), false))
        .chain(person.aliases.iter().map(|alias| (alias.as_str(), true)));
    for (name, is_alias) in names {
        let Some(found) = match_name(folded_query, query_tokens, query_set, name, is_alias) else {
            continue;
        };
        let replace = match &best {
            None => true,
            // Exact beats alias at the same rank; ties keep the first spelling,
            // which is always the display name.
            Some(current) => (found.kind.rank(), found.kind) < (current.kind.rank(), current.kind),
        };
        if replace {
            best = Some(found);
        }
    }
    best
}

/// Ranks every recorded person against `query_name`, best match first.
///
/// The order is deterministic: match rank, then display name, then person key.
pub fn rank(people: &[PersonNames], query_name: &str) -> Vec<Candidate> {
    let folded_query = fold(query_name);
    let query_tokens = tokens(&folded_query);
    if query_tokens.is_empty() {
        return Vec::new();
    }
    let query_set: BTreeSet<&str> = query_tokens.iter().copied().collect();
    let mut candidates: Vec<Candidate> = people
        .iter()
        .filter_map(|person| {
            let found = best_match(person, &folded_query, &query_tokens, &query_set)?;
            Some(Candidate {
                person_key: person.person_key.clone(),
                display_name: person.display_name.clone(),
                generation: person.generation,
                evidence_status: person.evidence_status.clone(),
                match_kind: Some(found.kind),
                matched_name: Some(found.matched_name),
                recorded_parents: Vec::new(),
                birth_year: person.birth_year,
                branch: person.branch.clone(),
            })
        })
        .collect();
    candidates.sort_by(|left, right| {
        left.rank()
            .cmp(&right.rank())
            // A recorded exact spelling is a better answer than a recorded
            // alias at the same rank; both stay candidates for a namesake tie.
            .then_with(|| left.match_kind.cmp(&right.match_kind))
            .then_with(|| left.display_name.cmp(&right.display_name))
            .then_with(|| left.person_key.cmp(&right.person_key))
    });
    candidates
}

/// One ranked search candidate with namesake-distinguishing detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// The ranked candidate.
    pub candidate: Candidate,
}

/// Searches the authorized catalog for a partial query and returns ranked
/// candidates with recorded parents attached, never merged.
///
/// The result is bounded by `MAX_CANDIDATES`; `truncated` is true when more
/// candidates exist than were returned. The caller authorizes before opening
/// the genealogy database; this function only reads.
pub fn search(
    client: &mut Client,
    query: &str,
) -> Result<(Vec<Candidate>, bool, bool), GenealogyError> {
    let catalog = load_catalog(client)?;
    let mut ranked = rank(&catalog.people, query);
    let more = ranked.len() > MAX_CANDIDATES;
    ranked.truncate(MAX_CANDIDATES);
    let keys: Vec<String> = ranked
        .iter()
        .map(|candidate| candidate.person_key.clone())
        .collect();
    let parents = recorded_parents(client, &keys)?;
    for candidate in &mut ranked {
        candidate.recorded_parents = parents
            .get(&candidate.person_key)
            .cloned()
            .unwrap_or_default();
    }
    Ok((ranked, more, catalog.truncated))
}

/// The strongest match tier and every person tied at it.
///
/// A caller that must not merge identities (for example document extraction)
/// uses this to decide between one confirmed candidate, several candidates, and
/// no candidate at all.
pub fn top_tier(candidates: &[Candidate]) -> Vec<&Candidate> {
    let Some(best) = candidates.first().map(Candidate::rank) else {
        return Vec::new();
    };
    candidates
        .iter()
        .filter(|candidate| candidate.rank() == best)
        .collect()
}

/// True when a candidate only matched a subset of the recorded name tokens.
///
/// A weak match is a lead for a human, never proof of identity.
pub fn is_weak(candidate: &Candidate) -> bool {
    candidate.match_kind == Some(MatchKind::Partial)
}

/// True when the folded query equals a recorded spelling exactly.
///
/// Anything else — a reordered name, a dropped middle name, a partial token
/// match — is a candidate for a human, never a confirmed identity.
pub fn is_exact_name(candidate: &Candidate) -> bool {
    matches!(
        candidate.match_kind,
        Some(MatchKind::Exact | MatchKind::Alias)
    )
}

/// Reads every cataloged person with their recorded spellings and the same
/// evidence status the tree read assigns.
pub fn load_catalog(client: &mut Client) -> Result<Catalog, GenealogyError> {
    let current = super::current::CURRENT_ASSERTION;
    let sql = format!(
        "SELECT p.person_key,
                COALESCE(NULLIF(BTRIM(p.display_name), ''), 'Unnamed person') AS display_name,
                p.generation,
                COALESCE(
                  (SELECT ARRAY_AGG(DISTINCT a.value_text ORDER BY a.value_text)
                     FROM assertion a
                    WHERE a.person_id = p.person_id
                      AND a.assertion_kind = 'name_variant'
                      AND a.value_text IS NOT NULL
                      AND ({current})),
                  ARRAY[]::text[]
                ) AS aliases,
                (SELECT MIN(SUBSTRING(a.value_date_start FROM 1 FOR 4)::int)
                   FROM assertion a
                  WHERE a.person_id = p.person_id
                    AND a.assertion_kind IN ('birth', 'birth_date')
                    AND a.review_status = 'verified'
                    AND a.value_date_start ~ '^[0-9]{{4}}'
                    AND ({current})) AS birth_year,
                (SELECT MIN(COALESCE(NULLIF(BTRIM(fb.branch_label), ''), fb.branch_key))
                   FROM assertion a
                   JOIN family_branch fb ON fb.branch_key = a.value_text
                  WHERE a.person_id = p.person_id
                    AND a.assertion_kind = 'branch_claim'
                    AND a.review_status = 'verified'
                    AND ({current})) AS branch,
                CASE
                  WHEN COALESCE(BOOL_OR(a.review_status IN ('needs_review', 'disputed')), false)
                    THEN 'needs_review'
                  WHEN COALESCE(BOOL_OR(a.review_status = 'verified'), false)
                    THEN 'verified'
                  ELSE 'unverified'
                END AS evidence_status
           FROM person p
           LEFT JOIN assertion a
             ON a.person_id = p.person_id AND ({current})
          GROUP BY p.person_id
          ORDER BY p.person_key
          LIMIT $1"
    );
    let rows = client.query(&sql, &[&(MAX_SEARCH_PEOPLE + 1)])?;
    let truncated = rows.len() as i64 > MAX_SEARCH_PEOPLE;
    let people = rows
        .iter()
        .take(MAX_SEARCH_PEOPLE as usize)
        .map(|row| {
            Ok(PersonNames {
                person_key: row.try_get(0)?,
                display_name: row.try_get(1)?,
                generation: row.try_get(2)?,
                aliases: row.try_get(3)?,
                birth_year: row.try_get(4)?,
                branch: row.try_get(5)?,
                evidence_status: row.try_get(6)?,
            })
        })
        .collect::<Result<Vec<_>, postgres::Error>>()?;
    Ok(Catalog { people, truncated })
}

/// Reads the recorded parents of the given people so a selection step can
/// distinguish namesakes without exposing anything the tree read hides.
pub fn recorded_parents(
    client: &mut Client,
    keys: &[String],
) -> Result<BTreeMap<String, Vec<ParentRef>>, GenealogyError> {
    let mut out: BTreeMap<String, Vec<ParentRef>> = BTreeMap::new();
    if keys.is_empty() {
        return Ok(out);
    }
    let keys = keys.to_vec();
    let current = super::current::CURRENT_ASSERTION;
    let sql = format!(
        "SELECT child.person_key, parent.person_key,
                COALESCE(NULLIF(BTRIM(parent.display_name), ''), 'Unnamed person'),
                CASE
                  WHEN EXISTS (
                    SELECT 1 FROM assertion a
                     WHERE a.assertion_kind = 'parent_child'
                       AND (
                         (a.person_id = parent.person_id
                          AND (a.object_person_id = child.person_id
                               OR a.value_text = child.person_key))
                         OR
                         (a.person_id = child.person_id
                          AND (a.object_person_id = parent.person_id
                               OR a.value_text = parent.person_key))
                       )
                       AND a.review_status IN ('needs_review', 'disputed')
                       AND ({current})
                  ) THEN 'needs_review'
                  WHEN EXISTS (
                    SELECT 1 FROM assertion a
                     WHERE a.assertion_kind = 'parent_child'
                       AND (
                         (a.person_id = parent.person_id
                          AND (a.object_person_id = child.person_id
                               OR a.value_text = child.person_key))
                         OR
                         (a.person_id = child.person_id
                          AND (a.object_person_id = parent.person_id
                               OR a.value_text = parent.person_key))
                       )
                       AND a.review_status = 'verified'
                       AND ({current})
                  ) THEN 'verified'
                  WHEN EXISTS (
                    SELECT 1 FROM assertion a
                     WHERE a.assertion_kind = 'parent_child'
                       AND (
                         (a.person_id = parent.person_id
                          AND (a.object_person_id = child.person_id
                               OR a.value_text = child.person_key))
                         OR
                         (a.person_id = child.person_id
                          AND (a.object_person_id = parent.person_id
                               OR a.value_text = parent.person_key))
                       )
                       AND ({current})
                  ) THEN 'unverified'
                  ELSE ''
                END AS evidence_status
           FROM parent_child_link pcl
           JOIN person parent ON parent.person_id = pcl.parent_id
           JOIN person child ON child.person_id = pcl.child_id
          WHERE child.person_key = ANY($1::text[])
          ORDER BY child.person_key, parent.person_key"
    );
    let rows = client.query(&sql, &[&keys])?;
    for row in rows {
        let status: String = row.try_get(3)?;
        // The selection step lists a parent only when a current, supported
        // assertion backs the link, matching the tree and the person detail.
        if status.is_empty() {
            continue;
        }
        let child_key: String = row.try_get(0)?;
        out.entry(child_key).or_default().push(ParentRef {
            person_key: row.try_get(1)?,
            display_name: row.try_get(2)?,
            evidence_status: status,
        });
    }
    Ok(out)
}

fn trim_quotes(text: &str) -> &str {
    text.trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\''
                | '\u{201c}'
                | '\u{201d}'
                | '\u{2018}'
                | '\u{2019}'
                | '\u{00ab}'
                | '\u{00bb}'
        )
    })
}

/// Strips `prefix` from the head of `text` ignoring case, including non-ASCII
/// letters, and returns the remainder.
fn strip_prefix_case_insensitive<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let mut consumed = 0usize;
    let mut chars = text.char_indices();
    for expected in prefix.chars() {
        let (index, actual) = chars.next()?;
        if actual.to_lowercase().eq(std::iter::once(expected)) {
            consumed = index + actual.len_utf8();
        } else {
            return None;
        }
    }
    Some(&text[consumed..])
}

/// Accepts a short run of alphabetic words as a person name.
fn sanitize_name(raw: &str) -> Option<String> {
    let cleaned = trim_quotes(raw).trim();
    if cleaned.is_empty() {
        return None;
    }
    let words: Vec<&str> = cleaned.split_whitespace().collect();
    if words.is_empty() || words.len() > 5 {
        return None;
    }
    for word in &words {
        let word = word.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '\'' && ch != '-');
        if word.is_empty() {
            return None;
        }
        if !word
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '\'' || ch == '-')
        {
            return None;
        }
        if !word.chars().any(|ch| ch.is_alphabetic()) {
            return None;
        }
        let folded = fold(word);
        if folded.is_empty() || STOPWORDS.contains(&folded.as_str()) {
            return None;
        }
    }
    let folded_words: Vec<String> = words.iter().map(|word| fold(word)).collect();
    if folded_words.iter().all(|word| word.chars().count() < 2) {
        return None;
    }
    Some(words.join(" "))
}

/// Classifies a question as naming a person and extracts the name text.
///
/// Returns `None` when the question does not look like a person question, so
/// ordinary archive questions keep their existing document answer. The caller
/// must still confirm a strong candidate before answering; `resolve` reports
/// `NotPersonQuestion` for a weak or absent match.
pub fn extract_person_question(question: &str) -> Option<PersonQuestion> {
    let trimmed = question.trim();
    let trimmed = trimmed.trim_end_matches(|ch: char| {
        matches!(ch, '?' | '!' | '.' | ',' | ';' | ':' | '\u{2014}' | '-')
    });
    let trimmed = trim_quotes(trimmed).trim();
    if trimmed.is_empty() {
        return None;
    }

    for (kind, prefixes) in [
        (PersonQuestionKind::Identity, IDENTITY_PREFIXES),
        (PersonQuestionKind::Topic, TOPIC_PREFIXES),
    ] {
        for prefix in prefixes {
            if let Some(remainder) = strip_prefix_case_insensitive(trimmed, prefix) {
                return sanitize_name(remainder.trim()).map(|name| PersonQuestion { name, kind });
            }
        }
    }

    sanitize_name(trimmed).map(|name| PersonQuestion {
        name,
        kind: PersonQuestionKind::Bare,
    })
}

fn evidence_status(assertions: &[AssertionRow]) -> &'static str {
    if assertions.iter().any(|assertion| {
        assertion.review_status == "needs_review" || assertion.review_status == "disputed"
    }) {
        "needs_review"
    } else if assertions
        .iter()
        .any(|assertion| assertion.review_status == "verified")
    {
        "verified"
    } else {
        "unverified"
    }
}

fn is_name_only(assertions: &[AssertionRow]) -> bool {
    assertions
        .iter()
        .all(|assertion| matches!(assertion.kind.as_str(), "display_name" | "name_variant"))
}

fn candidate_for(person: &PersonNames, found: Option<(MatchKind, String)>) -> Candidate {
    let (match_kind, matched_name) = match found {
        Some((kind, name)) => (Some(kind), Some(name)),
        None => (None, None),
    };
    Candidate {
        person_key: person.person_key.clone(),
        display_name: person.display_name.clone(),
        generation: person.generation,
        evidence_status: person.evidence_status.clone(),
        match_kind,
        matched_name,
        recorded_parents: Vec::new(),
        birth_year: person.birth_year,
        branch: person.branch.clone(),
    }
}

/// Resolves a person question, or an explicit person key, against the
/// authorized catalog.
///
/// The caller authorizes before opening the genealogy database. This function
/// only reads, and it reports a tie as ambiguous instead of choosing.
pub fn resolve(
    client: &mut Client,
    question: Option<&PersonQuestion>,
    selected_key: Option<&str>,
) -> Result<PersonResult, GenealogyError> {
    if question.is_none() && selected_key.is_none() {
        return Ok(PersonResult::NotPersonQuestion);
    }
    let catalog = load_catalog(client)?;

    if let Some(key) = selected_key {
        let query_name = question.map(|found| found.name.clone()).unwrap_or_default();
        let Some(person) = catalog
            .people
            .iter()
            .find(|person| person.person_key == key)
        else {
            return Ok(PersonResult::NoMatch {
                query_name,
                catalog_empty: catalog.people.is_empty(),
                catalog_truncated: catalog.truncated,
            });
        };
        let matched = rank(&catalog.people, &query_name)
            .into_iter()
            .find(|candidate| candidate.person_key == key)
            .and_then(|candidate| candidate.match_kind.zip(candidate.matched_name));
        let detail = lookup_person(client, &person.person_key)?;
        let Some(detail) = detail else {
            return Ok(PersonResult::NoMatch {
                query_name,
                catalog_empty: false,
                catalog_truncated: catalog.truncated,
            });
        };
        let name_only = is_name_only(&detail.assertions);
        return Ok(PersonResult::Found {
            query_name,
            candidate: Box::new(candidate_for(person, matched)),
            detail,
            name_only,
            catalog_truncated: catalog.truncated,
        });
    }

    let question = question.expect("a question or a selected key is present");
    let ranked = rank(&catalog.people, &question.name);
    let Some(best_rank) = ranked.first().map(Candidate::rank) else {
        return Ok(match question.kind {
            // "Who is …" is an identity question even when nothing matches; the
            // answer must say that no person is recorded, not that documents
            // are missing.
            PersonQuestionKind::Identity => PersonResult::NoMatch {
                query_name: question.name.clone(),
                catalog_empty: catalog.people.is_empty(),
                catalog_truncated: catalog.truncated,
            },
            // A topic question or a bare phrase that matches nothing is not an
            // identity question; the document path keeps ownership of it.
            PersonQuestionKind::Topic | PersonQuestionKind::Bare => PersonResult::NotPersonQuestion,
        });
    };

    // A weak partial match must never hijack a question that did not use an
    // identity phrasing.
    if question.kind != PersonQuestionKind::Identity && best_rank >= MatchKind::Partial.rank() {
        return Ok(PersonResult::NotPersonQuestion);
    }

    let mut best: Vec<Candidate> = ranked
        .into_iter()
        .take_while(|candidate| candidate.rank() == best_rank)
        .collect();

    if best.len() > 1 {
        let more = best.len() > MAX_CANDIDATES;
        best.truncate(MAX_CANDIDATES);
        let keys: Vec<String> = best
            .iter()
            .map(|candidate| candidate.person_key.clone())
            .collect();
        let parents = recorded_parents(client, &keys)?;
        for candidate in &mut best {
            candidate.recorded_parents = parents
                .get(&candidate.person_key)
                .cloned()
                .unwrap_or_default();
        }
        return Ok(PersonResult::Ambiguous {
            query_name: question.name.clone(),
            candidates: best,
            more,
            catalog_truncated: catalog.truncated,
        });
    }

    let candidate = best.remove(0);
    let person = catalog
        .people
        .iter()
        .find(|person| person.person_key == candidate.person_key);
    let Some(detail) = lookup_person(client, &candidate.person_key)? else {
        return Ok(PersonResult::NoMatch {
            query_name: question.name.clone(),
            catalog_empty: false,
            catalog_truncated: catalog.truncated,
        });
    };
    let mut candidate = candidate;
    if let Some(person) = person {
        candidate.evidence_status = evidence_status(&detail.assertions).to_string();
        candidate.generation = person.generation;
    }
    let name_only = is_name_only(&detail.assertions);
    Ok(PersonResult::Found {
        query_name: question.name.clone(),
        candidate: Box::new(candidate),
        detail,
        name_only,
        catalog_truncated: catalog.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn person(key: &str, name: &str, aliases: &[&str], status: &str) -> PersonNames {
        PersonNames {
            person_key: key.to_string(),
            display_name: name.to_string(),
            generation: None,
            aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            evidence_status: status.to_string(),
            birth_year: None,
            branch: None,
        }
    }

    fn catalog() -> Vec<PersonNames> {
        vec![
            person("P013", "Trần Đức Hân", &[], "verified"),
            person("P021", "Trần Thị Huyền", &[], "verified"),
            person("P022", "Lê Thanh Phong", &[], "needs_review"),
            person("P023", "Lê Thanh Phong", &[], "unverified"),
        ]
    }

    #[test]
    fn fold_removes_vietnamese_marks_and_d_stroke() {
        assert_eq!(fold("Trần Đức Hân"), "tran duc han");
        assert_eq!(fold("ĐỖ  THỊ   LAN"), "do thi lan");
        assert_eq!(fold("  Nguyễn—Văn  An "), "nguyen van an");
        assert_eq!(fold("O'Brien"), "o brien");
    }

    #[test]
    fn accent_and_order_folding_matches_the_western_order_question() {
        let candidates = rank(&catalog(), "Han Duc Tran");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].person_key, "P013");
        assert_eq!(candidates[0].match_kind, Some(MatchKind::Reordered));
    }

    #[test]
    fn exact_folded_display_name_matches() {
        let candidates = rank(&catalog(), "Tran Duc Han");
        assert_eq!(candidates[0].person_key, "P013");
        assert_eq!(candidates[0].match_kind, Some(MatchKind::Exact));
    }

    #[test]
    fn recorded_alias_is_reported_as_an_alias_and_outranks_reordering() {
        let people = vec![person(
            "P013",
            "Trần Đức Hân",
            &["Han Duc Tran"],
            "verified",
        )];
        let candidates = rank(&people, "Han Duc Tran");
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].person_key, "P013");
        assert_eq!(candidates[0].match_kind, Some(MatchKind::Alias));
        assert_eq!(candidates[0].matched_name.as_deref(), Some("Han Duc Tran"));

        let people = vec![person(
            "P100",
            "Nguyễn Văn Bình",
            &["Binh Nguyen"],
            "verified",
        )];
        let candidates = rank(&people, "Binh Nguyen");
        assert_eq!(candidates[0].match_kind, Some(MatchKind::Alias));
    }

    #[test]
    fn exact_display_name_outranks_a_recorded_alias() {
        let people = vec![
            person("P200", "Trần Văn Nam", &[], "verified"),
            person("P201", "Nguyễn Văn Hải", &["Tran Van Nam"], "verified"),
        ];
        let candidates = rank(&people, "Tran Van Nam");
        assert_eq!(candidates[0].person_key, "P200");
        assert_eq!(candidates[0].match_kind, Some(MatchKind::Exact));
    }

    #[test]
    fn namesakes_are_two_candidates_and_never_merged() {
        let candidates = rank(&catalog(), "Le Thanh Phong");
        assert_eq!(candidates.len(), 2);
        assert_eq!(
            candidates
                .iter()
                .map(|candidate| candidate.person_key.as_str())
                .collect::<Vec<_>>(),
            vec!["P022", "P023"]
        );
        assert!(candidates
            .iter()
            .all(|candidate| candidate.match_kind == Some(MatchKind::Exact)));
    }

    #[test]
    fn dropping_a_middle_name_is_only_a_partial_match() {
        let candidates = rank(&catalog(), "Tran Han");
        assert_eq!(candidates[0].person_key, "P013");
        assert_eq!(candidates[0].match_kind, Some(MatchKind::Partial));
    }

    #[test]
    fn a_single_token_query_is_never_a_partial_match() {
        // "Tran" is recorded in several names but names only one fragment, so
        // it is not a candidate at all rather than a partial match.
        assert!(rank(&catalog(), "Tran").is_empty());
        assert!(rank(&catalog(), "Han").iter().all(|candidate| matches!(
            candidate.match_kind,
            Some(MatchKind::Exact | MatchKind::Alias | MatchKind::Reordered)
        )));
    }

    #[test]
    fn ranks_are_deterministic() {
        let first = rank(&catalog(), "Le Thanh Phong");
        let second = rank(&catalog(), "Le Thanh Phong");
        assert_eq!(first, second);
    }

    #[test]
    fn identity_phrasings_extract_a_name() {
        let found = extract_person_question("Who is Han Duc Tran").expect("person question");
        assert_eq!(found.name, "Han Duc Tran");
        assert_eq!(found.kind, PersonQuestionKind::Identity);

        let found = extract_person_question("ai là Trần Đức Hân?").expect("person question");
        assert_eq!(found.name, "Trần Đức Hân");
        assert_eq!(found.kind, PersonQuestionKind::Identity);

        // The Vietnamese prefix matches regardless of case.
        let found = extract_person_question("AI LÀ Trần Đức Hân").expect("person question");
        assert_eq!(found.name, "Trần Đức Hân");
        assert_eq!(found.kind, PersonQuestionKind::Identity);
    }

    #[test]
    fn topic_phrasings_are_classified_separately() {
        let found =
            extract_person_question("tell me about Lê Thanh Phong").expect("person question");
        assert_eq!(found.name, "Lê Thanh Phong");
        assert_eq!(found.kind, PersonQuestionKind::Topic);

        // A broad topic question containing a function word is not a person
        // question at all, so it never leaves the document path.
        assert!(extract_person_question("Tell me about the land records").is_none());
        assert!(extract_person_question("What do you know about land records").is_some());
    }

    #[test]
    fn ordinary_archive_questions_are_not_person_questions() {
        for question in [
            "Where did the family live?",
            "Which family events are recorded in the register?",
            "Who are the earliest recorded ancestors?",
            "Who is my grandmother?",
            "What do the sources say about the family branches?",
            "who is the earliest recorded ancestor",
        ] {
            assert!(
                extract_person_question(question).is_none(),
                "{question} must stay a document question"
            );
        }
    }

    #[test]
    fn rank_carries_a_recorded_birth_year() {
        let mut with_year = person("P030", "Trần Văn An", &[], "verified");
        with_year.birth_year = Some(1911);
        let without_year = person("P031", "Trần Văn Ân", &[], "verified");
        let people = vec![with_year, without_year];
        let candidates = rank(&people, "Tran Van An");
        let found = candidates
            .iter()
            .find(|candidate| candidate.person_key == "P030")
            .expect("the person with a birth year ranks");
        assert_eq!(found.birth_year, Some(1911));
        let absent = candidates
            .iter()
            .find(|candidate| candidate.person_key == "P031")
            .expect("the person without a birth year ranks");
        assert_eq!(absent.birth_year, None);
    }

    #[test]
    fn a_bare_name_is_recognized() {
        let found = extract_person_question("Trần Đức Hân").expect("bare name");
        assert_eq!(found.name, "Trần Đức Hân");
        assert_eq!(found.kind, PersonQuestionKind::Bare);
    }
}
