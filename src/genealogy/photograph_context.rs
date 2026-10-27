//! Layout-aware photograph context and person-link proposals.
//!
//! A photograph's page, placement, and the document's text are used together to
//! propose the people a source identifies. The engine never recognizes faces and
//! never merges identities:
//!
//! * an explicit photo caption is the strongest signal;
//! * a section heading or the only person named on a biography page can propose
//!   the biography subject, with a lower confidence and a stated basis;
//! * text near the image (above or below, within one text band) can propose a
//!   nearby name, but a nearby name alone never links every person on the page;
//! * a group caption or a caption naming several people can propose each named
//!   person, while a biography that merely mentions several relatives proposes
//!   none of them;
//! * a biography continued on an adjacent page is used only as a fallback and is
//!   always low confidence;
//! * dates and recorded relationships distinguish namesakes when the source
//!   states a matching year or names a recorded relative.
//!
//! A proposal is never approval. The caller stores proposed links separately
//! from confirmed links, and a reviewer makes the final choice.

use std::collections::{BTreeMap, BTreeSet};

use lopdf::{content::Content, Document, Encoding, Object};
use thiserror::Error;

use super::inventory::Placement;
use super::person_search::{self, MatchKind};

/// Errors raised while reading the PDF text layout.
#[derive(Debug, Error)]
pub enum ContextError {
    /// The source bytes are not a readable PDF.
    #[error("the PDF could not be read: {0}")]
    Unreadable(String),
}

/// One text line with its approximate layout position.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLine {
    /// 1-based page number.
    pub page_number: u32,
    /// The normalized line text.
    pub text: String,
    /// Left edge in default user space.
    pub x: f64,
    /// Baseline in default user space.
    pub y: f64,
    /// The largest font size on the line.
    pub font_size: f64,
}

/// The text layout of one page.
#[derive(Debug, Clone, PartialEq)]
pub struct PageContext {
    /// 1-based page number.
    pub page_number: u32,
    /// Page box width, when the page states one.
    pub width: Option<f64>,
    /// Page box height, when the page states one.
    pub height: Option<f64>,
    /// Lines in reading order (top to bottom, then left to right).
    pub lines: Vec<TextLine>,
}

impl PageContext {
    /// The page text with one line per source line.
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|line| line.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The text layout of one document.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentContext {
    /// Pages in page-number order.
    pub pages: Vec<PageContext>,
}

impl DocumentContext {
    /// Finds one page.
    pub fn page(&self, page_number: u32) -> Option<&PageContext> {
        self.pages
            .iter()
            .find(|page| page.page_number == page_number)
    }
}

/// One cataloged person plus the context used to distinguish namesakes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersonContext {
    /// Stable person key.
    pub person_key: String,
    /// The recorded display name.
    pub display_name: String,
    /// Recorded name variants.
    pub aliases: Vec<String>,
    /// Recorded generation, when present.
    pub generation: Option<i32>,
    /// `verified`, `needs_review`, or `unverified`.
    pub evidence_status: String,
    /// Four-digit years from recorded birth and death dates.
    pub dates: Vec<String>,
    /// Recorded parent person keys.
    pub parent_keys: Vec<String>,
    /// Recorded spouse person keys.
    pub spouse_keys: Vec<String>,
}

/// How much confidence the source supports for a proposed person.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// A weak lead, or an adjacent-page fallback.
    Low,
    /// A nearby name or a biography subject inferred from a heading.
    Medium,
    /// An explicit caption or group caption names the person.
    High,
}

impl Confidence {
    /// The stable wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

/// Where a proposal's evidence came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceKind {
    /// The biography continues on an adjacent page.
    AdjacentPage,
    /// The page is a biography and names exactly one recorded person.
    Biography,
    /// Text near the image names the person.
    NearbyText,
    /// A section heading names the person.
    SectionHeading,
    /// A photo caption names the person.
    Caption,
    /// A caption identifies several people in a group.
    ExplicitGroup,
}

impl SourceKind {
    /// The stable wire value.
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::AdjacentPage => "adjacent_page",
            SourceKind::Biography => "biography",
            SourceKind::NearbyText => "nearby_text",
            SourceKind::SectionHeading => "section_heading",
            SourceKind::Caption => "caption",
            SourceKind::ExplicitGroup => "explicit_group",
        }
    }
}

/// One proposed person, strongest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutCandidate {
    /// Stable person key.
    pub person_key: String,
    /// The recorded display name.
    pub display_name: String,
    /// How the source matched the recorded name.
    pub match_kind: MatchKind,
    /// How strongly the source supports the link.
    pub confidence: Confidence,
    /// Where the evidence came from.
    pub source_kind: SourceKind,
    /// The 1-based PDF page the evidence is on.
    pub source_page: u32,
    /// The bounded source text that supports the proposal.
    pub supporting_text: String,
    /// Why the confidence was assigned, including date or relationship signals.
    pub confidence_basis: String,
    /// Why a namesake or weak match needs an explicit reviewer choice.
    pub ambiguity: Option<String>,
    /// 1-based strength order shown to the reviewer.
    pub candidate_rank: i32,
}

/// The result of one layout-aware proposal run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayoutProposal {
    /// Candidates in strength order. Empty when nothing is supported.
    pub candidates: Vec<LayoutCandidate>,
    /// Why the photo is left unlinked, when no candidate is supported.
    pub unlinked_reason: Option<String>,
}

impl LayoutProposal {
    fn unlinked(reason: impl Into<String>) -> Self {
        Self {
            candidates: Vec::new(),
            unlinked_reason: Some(reason.into()),
        }
    }
}

/// Reads the text layout of every page in `bytes`.
pub fn extract_context(bytes: &[u8]) -> Result<DocumentContext, ContextError> {
    let doc = Document::load_mem(bytes).map_err(|err| ContextError::Unreadable(err.to_string()))?;
    let pages = doc.get_pages();
    let mut out = Vec::with_capacity(pages.len());
    for (&page_number, &page_id) in &pages {
        let (width, height) = page_box_size(&doc, page_id).unzip();
        let mut lines = page_lines(&doc, page_id, page_number);
        if lines.is_empty() {
            lines = fallback_lines(&doc, page_number);
        }
        out.push(PageContext {
            page_number,
            width,
            height,
            lines,
        });
    }
    out.sort_by_key(|page| page.page_number);
    Ok(DocumentContext { pages: out })
}

/// Proposes people for one photograph from its page layout and nearby text.
///
/// `page_number` is the first page that draws the photo. `placement` is the
/// photo's placement rectangle when the content operators resolved it.
/// `explicit_caption` is a caption the document or a reviewer recorded; it
/// outranks every inferred source.
pub fn propose_layout_links(
    context: &DocumentContext,
    page_number: u32,
    placement: Option<Placement>,
    explicit_caption: Option<&str>,
    catalog: &[PersonContext],
) -> LayoutProposal {
    let Some(page) = context.page(page_number) else {
        return LayoutProposal::unlinked("the photo's page has no readable text");
    };
    let page_text = page.text();
    let previous_text = page_number
        .checked_sub(1)
        .and_then(|number| context.page(number))
        .map(PageContext::text)
        .unwrap_or_default();
    let next_text = context
        .page(page_number.saturating_add(1))
        .map(PageContext::text)
        .unwrap_or_default();
    let full_context = format!("{previous_text}\n{page_text}\n{next_text}");

    let heading = detect_heading(page, placement);
    let caption_text = explicit_caption
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| caption_lines(page, placement).join(" "));
    let biography_text = biography_lines(page, &heading, &caption_text).join("\n");

    let mut hits: BTreeMap<String, CandidateBuilder> = BTreeMap::new();

    // 1. An explicit caption (or a caption-like line next to the image) is the
    // strongest source. A caption naming several people is an explicit group.
    if !caption_text.is_empty() {
        let mut caption_hits = find_matches(&caption_text, catalog, true);
        if is_name_list_like(&caption_text) {
            merge_hits(&mut caption_hits, suffix_matches(&caption_text, catalog));
        }
        let group = caption_hits.len() >= 2 || is_group_phrase(&caption_text);
        let caption_count = caption_hits.len();
        for hit in caption_hits {
            let source_kind = if group && caption_count >= 2 {
                SourceKind::ExplicitGroup
            } else {
                SourceKind::Caption
            };
            add_hit(
                &mut hits,
                catalog,
                hit,
                source_kind,
                Confidence::High,
                page_number,
                &caption_text,
            );
        }
    }

    // 2. A section heading names the biography subject. It is a strong signal
    // only when no caption identifies the people already.
    if let Some(heading) = &heading {
        if !same_text(&heading.text, &caption_text) {
            let heading_hits = find_matches(&heading.text, catalog, true);
            let confidence = if caption_text.is_empty() {
                Confidence::High
            } else {
                Confidence::Medium
            };
            for hit in heading_hits {
                add_hit(
                    &mut hits,
                    catalog,
                    hit,
                    SourceKind::SectionHeading,
                    confidence,
                    page_number,
                    &heading.text,
                );
            }
        }
    }

    // 3. Text immediately above or below the image. Only short, caption-like
    // lines are used; a biography paragraph is handled by the next rule. A
    // comma-separated name list is also used when an explicit caption exists,
    // because the list itself is the caption on many archive pages.
    for line in nearby_lines(page, placement) {
        let list_like = is_name_list_like(&line);
        if !caption_text.is_empty() && !list_like {
            continue;
        }
        if !is_caption_like(&line)
            && !list_like
            && (!is_label_like(&line) || is_heading_like(&line))
        {
            continue;
        }
        let mut line_hits = find_matches(&line, catalog, false);
        if list_like {
            merge_hits(&mut line_hits, suffix_matches(&line, catalog));
        }
        for hit in line_hits {
            add_hit(
                &mut hits,
                catalog,
                hit,
                SourceKind::NearbyText,
                Confidence::Medium,
                page_number,
                &line,
            );
        }
    }

    // 4. A biography page with exactly one recorded person can identify that
    // person. Several names on a biography page are not a group photo.
    if hits.is_empty() && !biography_text.is_empty() {
        let biography_hits = find_matches(&biography_text, catalog, false);
        if biography_hits.len() == 1 {
            let hit = biography_hits.into_iter().next().expect("one hit");
            let supporting = first_sentence_naming(&biography_text, &hit);
            add_hit(
                &mut hits,
                catalog,
                hit,
                SourceKind::Biography,
                Confidence::Low,
                page_number,
                &supporting,
            );
        } else if biography_hits.len() > 1 {
            return LayoutProposal::unlinked(
                "the page names several recorded people and no caption identifies who is in the photo",
            );
        }
    }

    // 5. A biography that continues on the next or previous page is a weak
    // fallback only when the target page itself names nobody.
    if hits.is_empty() {
        for (kind, text, number) in [
            (
                "previous",
                previous_text.as_str(),
                page_number.saturating_sub(1),
            ),
            ("next", next_text.as_str(), page_number.saturating_add(1)),
        ] {
            if text.trim().is_empty() || number == page_number {
                continue;
            }
            let adjacent_hits = find_matches(text, catalog, false);
            if adjacent_hits.len() == 1 {
                let hit = adjacent_hits.into_iter().next().expect("one hit");
                let supporting = first_sentence_naming(text, &hit);
                let mut builder = CandidateBuilder::new(
                    catalog,
                    hit,
                    SourceKind::AdjacentPage,
                    Confidence::Low,
                    number,
                    &supporting,
                );
                builder
                    .notes
                    .push(format!("the biography continues on the {kind} page"));
                insert_candidate(&mut hits, builder);
            }
        }
    }

    if hits.is_empty() {
        return LayoutProposal::unlinked(
            "no caption, section heading, nearby text, or biography names a recorded person",
        );
    }

    let mut candidates: Vec<CandidateBuilder> = hits.into_values().collect();
    for candidate in &mut candidates {
        let (score, notes) = disambiguation_signals(&candidate.person, &full_context, catalog);
        candidate.score = score;
        candidate.notes.extend(notes);
    }
    disambiguate(&mut candidates, catalog);
    candidates.sort_by(|left, right| {
        right
            .confidence
            .cmp(&left.confidence)
            .then_with(|| right.source_kind.cmp(&left.source_kind))
            .then_with(|| match_rank(left.match_kind).cmp(&match_rank(right.match_kind)))
            .then_with(|| right.score.cmp(&left.score))
            .then_with(|| evidence_rank(&right.person).cmp(&evidence_rank(&left.person)))
            .then_with(|| left.person.display_name.cmp(&right.person.display_name))
            .then_with(|| left.person.person_key.cmp(&right.person.person_key))
    });

    let mut out = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.into_iter().enumerate() {
        out.push(LayoutCandidate {
            person_key: candidate.person.person_key.clone(),
            display_name: candidate.person.display_name.clone(),
            match_kind: candidate.match_kind,
            confidence: candidate.confidence,
            source_kind: candidate.source_kind,
            source_page: candidate.source_page,
            supporting_text: bounded(&candidate.supporting_text, 600),
            confidence_basis: candidate.basis(),
            ambiguity: candidate.ambiguity,
            candidate_rank: (index + 1) as i32,
        });
    }
    LayoutProposal {
        candidates: out,
        unlinked_reason: None,
    }
}

// ---------------------------------------------------------------------------
// Candidate assembly
// ---------------------------------------------------------------------------

/// One name match in one source text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NameHit {
    person_key: String,
    display_name: String,
    match_kind: MatchKind,
    matched_text: String,
    /// Why a weak or spacing-tolerant match needs care.
    note: Option<String>,
    /// Set when several records share the matched words.
    ambiguity: Option<String>,
}

/// One candidate while its evidence is being assembled.
#[derive(Debug, Clone)]
struct CandidateBuilder {
    person: PersonContext,
    match_kind: MatchKind,
    source_kind: SourceKind,
    confidence: Confidence,
    source_page: u32,
    supporting_text: String,
    notes: Vec<String>,
    ambiguity: Option<String>,
    score: i32,
}

impl CandidateBuilder {
    fn new(
        catalog: &[PersonContext],
        hit: NameHit,
        source_kind: SourceKind,
        confidence: Confidence,
        source_page: u32,
        supporting_text: &str,
    ) -> Self {
        let person = catalog
            .iter()
            .find(|person| person.person_key == hit.person_key)
            .cloned()
            .unwrap_or(PersonContext {
                person_key: hit.person_key.clone(),
                display_name: hit.display_name.clone(),
                aliases: Vec::new(),
                generation: None,
                evidence_status: "unverified".to_string(),
                dates: Vec::new(),
                parent_keys: Vec::new(),
                spouse_keys: Vec::new(),
            });
        let mut notes = vec![source_note(source_kind)];
        if hit.match_kind == MatchKind::Reordered {
            notes.push(
                "the name tokens appear in a different order; verify the identity".to_string(),
            );
        }
        if let Some(note) = &hit.note {
            notes.push(note.clone());
        }
        let confidence = if hit.ambiguity.is_some() {
            confidence.min(Confidence::Low)
        } else {
            confidence
        };
        Self {
            person,
            match_kind: hit.match_kind,
            source_kind,
            confidence,
            source_page,
            supporting_text: supporting_text.to_string(),
            notes,
            ambiguity: hit.ambiguity,
            score: 0,
        }
    }

    fn basis(&self) -> String {
        let mut parts = self.notes.clone();
        if let Some(ambiguity) = &self.ambiguity {
            parts.push(ambiguity.clone());
        }
        parts.join("; ")
    }

    fn strength(&self) -> (Confidence, SourceKind, u8, usize) {
        (
            self.confidence,
            self.source_kind,
            match_rank(self.match_kind),
            self.supporting_text.chars().count(),
        )
    }
}

fn source_note(source: SourceKind) -> String {
    match source {
        SourceKind::Caption => "an explicit photo caption names this person".to_string(),
        SourceKind::ExplicitGroup => {
            "the photo caption identifies this person in a group".to_string()
        }
        SourceKind::SectionHeading => "the section heading names this person".to_string(),
        SourceKind::NearbyText => "the name appears in text next to the photo".to_string(),
        SourceKind::Biography => {
            "the photo's page is a biography and this is the only recorded person named".to_string()
        }
        SourceKind::AdjacentPage => {
            "the biography continues on an adjacent page, where this person is named".to_string()
        }
    }
}

fn add_hit(
    hits: &mut BTreeMap<String, CandidateBuilder>,
    catalog: &[PersonContext],
    hit: NameHit,
    source_kind: SourceKind,
    confidence: Confidence,
    source_page: u32,
    supporting_text: &str,
) {
    let builder = CandidateBuilder::new(
        catalog,
        hit,
        source_kind,
        confidence,
        source_page,
        supporting_text,
    );
    insert_candidate(hits, builder);
}

fn insert_candidate(hits: &mut BTreeMap<String, CandidateBuilder>, candidate: CandidateBuilder) {
    match hits.get(&candidate.person.person_key) {
        Some(existing) if existing.strength() >= candidate.strength() => {}
        _ => {
            hits.insert(candidate.person.person_key.clone(), candidate);
        }
    }
}

fn match_rank(kind: MatchKind) -> u8 {
    match kind {
        MatchKind::Exact | MatchKind::Alias => 1,
        MatchKind::Reordered => 2,
        MatchKind::Partial => 3,
    }
}

fn evidence_rank(person: &PersonContext) -> u8 {
    match person.evidence_status.as_str() {
        "verified" => 3,
        "unverified" => 2,
        _ => 1,
    }
}

/// Marks namesakes and weak matches for an explicit reviewer choice.
fn disambiguate(candidates: &mut [CandidateBuilder], catalog: &[PersonContext]) {
    let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        groups
            .entry(person_search::fold(&candidate.person.display_name))
            .or_default()
            .push(index);
    }
    for indices in groups.values().filter(|indices| indices.len() > 1) {
        let best = indices
            .iter()
            .map(|index| candidates[*index].score)
            .max()
            .unwrap_or(0);
        let top: Vec<usize> = indices
            .iter()
            .copied()
            .filter(|index| candidates[*index].score == best)
            .collect();
        if top.len() == 1 {
            let winner = top[0];
            let winner_note = candidates[winner]
                .notes
                .iter()
                .find(|note| note.contains("year") || note.contains("relative"))
                .cloned()
                .unwrap_or_else(|| "the stronger record matches the source context".to_string());
            let winner_key = candidates[winner].person.person_key.clone();
            for &index in indices {
                if index != winner {
                    candidates[index].ambiguity = Some(format!(
                        "a namesake with the same name; the source favors the record {winner_key} ({winner_note})"
                    ));
                    candidates[index].confidence = Confidence::Low;
                }
            }
            candidates[winner]
                .notes
                .push("distinguished from a namesake by date or relationship signals".to_string());
        } else {
            for &index in indices {
                candidates[index].ambiguity = Some(
                    "two recorded people share this name; the source gives no distinguishing date or relationship, so choose the right person"
                        .to_string(),
                );
                if candidates[index].confidence > Confidence::Medium {
                    candidates[index].confidence = Confidence::Medium;
                }
            }
        }
    }

    for candidate in candidates.iter_mut() {
        if candidate.match_kind == MatchKind::Reordered && candidate.confidence > Confidence::Medium
        {
            candidate.confidence = Confidence::Medium;
        }
    }

    // Keep the catalog ordering stable even when the caller's slice is not.
    let _ = catalog;
}

/// Scores the date and relationship signals that distinguish namesakes.
fn disambiguation_signals(
    person: &PersonContext,
    context: &str,
    catalog: &[PersonContext],
) -> (i32, Vec<String>) {
    let folded = person_search::fold(context);
    let words: BTreeSet<&str> = folded.split_whitespace().collect();
    let mut score = 0;
    let mut notes = Vec::new();

    for date in &person.dates {
        if words.contains(date.as_str()) {
            score += 2;
            notes.push(format!(
                "the source states {date}, matching this record's date"
            ));
        }
    }

    let relationship_terms = [
        "father", "mother", "son", "daughter", "husband", "wife", "spouse", "brother", "sister",
        "parent", "child", "cha", "me", "con", "vo", "chong", "anh", "chi", "em",
    ];
    let has_relationship = relationship_terms.iter().any(|term| words.contains(term));
    if has_relationship {
        for key in person.parent_keys.iter().chain(person.spouse_keys.iter()) {
            let Some(relative) = catalog.iter().find(|other| &other.person_key == key) else {
                continue;
            };
            let name = person_search::fold(&relative.display_name);
            if !name.is_empty() && folded.contains(&name) {
                score += 1;
                notes.push(format!(
                    "the source names a recorded relative ({})",
                    relative.display_name
                ));
                break;
            }
        }
    }

    if person.evidence_status == "verified" {
        score += 1;
        notes.push("the person record is verified".to_string());
    }

    (score, notes)
}

// ---------------------------------------------------------------------------
// Name matching
// ---------------------------------------------------------------------------

/// Finds the best recorded-name match for each person in `text`.
fn find_matches(text: &str, catalog: &[PersonContext], allow_single_token: bool) -> Vec<NameHit> {
    let index = NameIndex::build(catalog, allow_single_token);
    if index.lengths.is_empty() {
        return Vec::new();
    }
    let folded = person_search::fold(text);
    let tokens: Vec<&str> = folded.split_whitespace().collect();
    let mut best: BTreeMap<String, NameHit> = BTreeMap::new();
    for start in 0..tokens.len() {
        for &length in &index.lengths {
            if start + length > tokens.len() {
                continue;
            }
            let window = &tokens[start..start + length];
            let joined = window.join(" ");
            if let Some(entries) = index.exact.get(&joined) {
                for (person_key, display_name, kind) in entries {
                    record_hit(
                        &mut best,
                        NameHit {
                            person_key: person_key.clone(),
                            display_name: display_name.clone(),
                            match_kind: *kind,
                            matched_text: joined.clone(),
                            note: None,
                            ambiguity: None,
                        },
                    );
                }
            }
            let mut sorted = window.to_vec();
            sorted.sort_unstable();
            let sorted_key = sorted.join(" ");
            if let Some(entries) = index.reordered.get(&sorted_key) {
                for (person_key, display_name, _) in entries {
                    record_hit(
                        &mut best,
                        NameHit {
                            person_key: person_key.clone(),
                            display_name: display_name.clone(),
                            match_kind: MatchKind::Reordered,
                            matched_text: joined.clone(),
                            note: None,
                            ambiguity: None,
                        },
                    );
                }
            }
        }
    }
    // The document's text layer sometimes drops or splits word spaces. Keep the
    // existing exact/alias/reordered matches and add a spacing-tolerant match
    // for any recorded name the token windows missed.
    for hit in compact_matches(text, catalog) {
        best.entry(hit.person_key.clone()).or_insert(hit);
    }
    best.into_values().collect()
}

/// Folds text and removes every space, so noisy PDF spacing cannot hide a name.
fn compact_fold(text: &str) -> String {
    person_search::fold(text).split_whitespace().collect()
}

/// Matches recorded names after ignoring word spaces.
fn compact_matches(text: &str, catalog: &[PersonContext]) -> Vec<NameHit> {
    let compact_text = compact_fold(text);
    if compact_text.is_empty() {
        return Vec::new();
    }
    let mut hits = Vec::new();
    for person in catalog {
        for name in std::iter::once(&person.display_name).chain(person.aliases.iter()) {
            let folded = person_search::fold(name);
            let tokens: Vec<&str> = folded.split_whitespace().collect();
            if tokens.len() < 2 {
                continue;
            }
            let compact = tokens.join("");
            if compact.chars().count() < 6 || !compact_text.contains(&compact) {
                continue;
            }
            hits.push(NameHit {
                person_key: person.person_key.clone(),
                display_name: person.display_name.clone(),
                match_kind: MatchKind::Partial,
                matched_text: compact,
                note: Some(
                    "the PDF text layer loses word boundaries; the name matched after ignoring spaces"
                        .to_string(),
                ),
                ambiguity: None,
            });
        }
    }
    hits
}

/// Matches the final words of a recorded name in a caption's name list.
///
/// Archive captions often give only `Given Middle` for `Surname Given Middle`. The
/// final two words are used only for comma-separated name lists, never for a
/// biography sentence. When several records share those words, each candidate
/// carries an ambiguity note and is downgraded to low confidence.
fn suffix_matches(text: &str, catalog: &[PersonContext]) -> Vec<NameHit> {
    let segments: Vec<String> = text
        .split([',', ';', '/', '&'])
        .flat_map(|part| part.split(" và "))
        .flat_map(|part| part.split(" and "))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(compact_fold)
        .filter(|part| part.chars().count() >= 6)
        .collect();
    if segments.is_empty() {
        return Vec::new();
    }

    let mut owners: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for person in catalog {
        for name in std::iter::once(&person.display_name).chain(person.aliases.iter()) {
            let folded = person_search::fold(name);
            let tokens: Vec<&str> = folded.split_whitespace().collect();
            if tokens.len() < 2 {
                continue;
            }
            let suffix = tokens[tokens.len() - 2..].join("");
            if suffix.chars().count() >= 6 {
                owners
                    .entry(suffix)
                    .or_default()
                    .insert(person.person_key.clone());
            }
        }
    }

    let mut hits = Vec::new();
    for person in catalog {
        for name in std::iter::once(&person.display_name).chain(person.aliases.iter()) {
            let folded = person_search::fold(name);
            let tokens: Vec<&str> = folded.split_whitespace().collect();
            if tokens.len() < 2 {
                continue;
            }
            let suffix = tokens[tokens.len() - 2..].join("");
            if suffix.chars().count() < 6 {
                continue;
            }
            if !segments.iter().any(|segment| segment.contains(&suffix)) {
                continue;
            }
            let shared = owners.get(&suffix).is_some_and(|people| people.len() > 1);
            hits.push(NameHit {
                person_key: person.person_key.clone(),
                display_name: person.display_name.clone(),
                match_kind: MatchKind::Partial,
                matched_text: suffix.clone(),
                note: Some(
                    "the caption gives only the final words of the recorded name; verify the identity"
                        .to_string(),
                ),
                ambiguity: shared.then(|| {
                    "several recorded people share these final name words; choose the right person"
                        .to_string()
                }),
            });
        }
    }
    hits
}

fn merge_hits(target: &mut Vec<NameHit>, extra: Vec<NameHit>) {
    let mut seen: BTreeMap<String, usize> = target
        .iter()
        .enumerate()
        .map(|(index, hit)| (hit.person_key.clone(), index))
        .collect();
    for hit in extra {
        match seen.get(&hit.person_key) {
            Some(&index) => {
                if target[index].note.is_none() {
                    target[index].note = hit.note;
                }
                if target[index].ambiguity.is_none() {
                    target[index].ambiguity = hit.ambiguity;
                }
            }
            None => {
                seen.insert(hit.person_key.clone(), target.len());
                target.push(hit);
            }
        }
    }
}

fn record_hit(best: &mut BTreeMap<String, NameHit>, hit: NameHit) {
    match best.get(&hit.person_key) {
        Some(existing)
            if (match_rank(existing.match_kind), existing.matched_text.len())
                <= (match_rank(hit.match_kind), hit.matched_text.len()) => {}
        _ => {
            best.insert(hit.person_key.clone(), hit);
        }
    }
}

/// A folded name index over the catalog.
struct NameIndex {
    exact: BTreeMap<String, Vec<(String, String, MatchKind)>>,
    reordered: BTreeMap<String, Vec<(String, String, MatchKind)>>,
    lengths: BTreeSet<usize>,
}

impl NameIndex {
    fn build(catalog: &[PersonContext], allow_single_token: bool) -> Self {
        let mut exact: BTreeMap<String, Vec<(String, String, MatchKind)>> = BTreeMap::new();
        let mut reordered: BTreeMap<String, Vec<(String, String, MatchKind)>> = BTreeMap::new();
        let mut lengths = BTreeSet::new();
        for person in catalog {
            for (name, kind) in std::iter::once((&person.display_name, MatchKind::Exact))
                .chain(person.aliases.iter().map(|alias| (alias, MatchKind::Alias)))
            {
                let folded = person_search::fold(name);
                let tokens: Vec<&str> = folded.split_whitespace().collect();
                if tokens.is_empty() {
                    continue;
                }
                if tokens.len() == 1 && !allow_single_token {
                    continue;
                }
                if tokens.len() == 1 && is_common_word(tokens[0]) {
                    continue;
                }
                let joined = tokens.join(" ");
                exact.entry(joined).or_default().push((
                    person.person_key.clone(),
                    person.display_name.clone(),
                    kind,
                ));
                let mut sorted = tokens;
                sorted.sort_unstable();
                reordered.entry(sorted.join(" ")).or_default().push((
                    person.person_key.clone(),
                    person.display_name.clone(),
                    kind,
                ));
                lengths.insert(folded.split_whitespace().count());
            }
        }
        Self {
            exact,
            reordered,
            lengths,
        }
    }
}

/// Common words that must not turn a one-word name into a false positive.
fn is_common_word(word: &str) -> bool {
    matches!(
        word,
        "a" | "an"
            | "and"
            | "are"
            | "as"
            | "at"
            | "be"
            | "by"
            | "for"
            | "from"
            | "he"
            | "her"
            | "his"
            | "in"
            | "is"
            | "it"
            | "of"
            | "on"
            | "or"
            | "she"
            | "the"
            | "they"
            | "this"
            | "to"
            | "was"
            | "were"
            | "with"
    )
}

// ---------------------------------------------------------------------------
// Layout helpers
// ---------------------------------------------------------------------------

fn page_box_size(doc: &Document, page_id: lopdf::ObjectId) -> Option<(f64, f64)> {
    let page = doc.get_object(page_id).ok()?.as_dict().ok()?;
    for key in [b"CropBox".as_slice(), b"MediaBox".as_slice()] {
        let Ok(value) = page.get(key) else {
            continue;
        };
        let Some(Object::Array(values)) = resolved(doc, value) else {
            continue;
        };
        if values.len() != 4 {
            continue;
        }
        let numbers: Vec<f64> = values
            .iter()
            .filter_map(|value| match value {
                Object::Integer(number) => Some(*number as f64),
                Object::Real(number) => Some(*number as f64),
                _ => None,
            })
            .collect();
        if numbers.len() != 4 {
            continue;
        }
        let width = (numbers[2] - numbers[0]).abs();
        let height = (numbers[3] - numbers[1]).abs();
        if width > 0.0 && height > 0.0 {
            return Some((width, height));
        }
    }
    None
}

fn resolved<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn page_lines(doc: &Document, page_id: lopdf::ObjectId, page_number: u32) -> Vec<TextLine> {
    let Ok(content_data) = doc.get_page_content(page_id) else {
        return Vec::new();
    };
    let Ok(content) = Content::decode(&content_data) else {
        return Vec::new();
    };
    let encodings = page_encodings(doc, page_id);
    let mut segments: Vec<Segment> = Vec::new();
    let mut text_matrix = IDENTITY;
    let mut line_matrix = IDENTITY;
    let mut leading = 0.0;
    let mut font_size = 12.0;
    let mut current_encoding: Option<&Encoding<'_>> = None;

    for operation in &content.operations {
        match operation.operator.as_str() {
            "BT" => {
                text_matrix = IDENTITY;
                line_matrix = IDENTITY;
            }
            "Tm" => {
                if let Some(matrix) = matrix(&operation.operands) {
                    text_matrix = matrix;
                    line_matrix = matrix;
                }
            }
            "Td" => {
                if let Some((tx, ty)) = pair(&operation.operands) {
                    line_matrix[4] += tx;
                    line_matrix[5] += ty;
                    text_matrix = line_matrix;
                }
            }
            "TD" => {
                if let Some((tx, ty)) = pair(&operation.operands) {
                    leading = -ty;
                    line_matrix[4] += tx;
                    line_matrix[5] += ty;
                    text_matrix = line_matrix;
                }
            }
            "T*" => {
                line_matrix[5] -= leading;
                text_matrix = line_matrix;
            }
            "TL" => {
                if let Some(value) = number(operation.operands.first()) {
                    leading = value;
                }
            }
            "Tf" => {
                if let Some(size) = number(operation.operands.get(1)) {
                    font_size = size;
                }
                if let Some(Object::Name(name)) = operation.operands.first() {
                    current_encoding = encodings.get(name);
                }
            }
            "Tj" | "'" | "\"" => {
                let operand_index = if operation.operator == "\"" { 2 } else { 0 };
                if operation.operator != "Tj" {
                    line_matrix[5] -= leading;
                    text_matrix = line_matrix;
                }
                if let Some(text) = operation
                    .operands
                    .get(operand_index)
                    .and_then(|object| decode_text(current_encoding, object))
                {
                    push_segment(&mut segments, text, text_matrix, font_size);
                }
            }
            "TJ" => {
                if let Some(Object::Array(items)) = operation.operands.first() {
                    let mut text = String::new();
                    for item in items {
                        if let Some(decoded) = decode_text(current_encoding, item) {
                            text.push_str(&decoded);
                        } else if item.as_i64().is_ok_and(|value| value < -100)
                            || item.as_float().is_ok_and(|value| f64::from(value) < -100.0)
                        {
                            text.push(' ');
                        }
                    }
                    push_segment(&mut segments, text, text_matrix, font_size);
                }
            }
            _ => {}
        }
    }

    group_lines(segments, page_number)
}

const IDENTITY: [f64; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

#[derive(Debug, Clone)]
struct Segment {
    x: f64,
    y: f64,
    font_size: f64,
    text: String,
}

fn decode_text(encoding: Option<&Encoding<'_>>, object: &Object) -> Option<String> {
    let Object::String(bytes, _) = object else {
        return None;
    };
    let text = match encoding {
        Some(encoding) => Document::decode_text(encoding, bytes)
            .unwrap_or_else(|_| String::from_utf8_lossy(bytes).to_string()),
        None => String::from_utf8_lossy(bytes).to_string(),
    };
    let text = normalize_line(&text);
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn push_segment(segments: &mut Vec<Segment>, text: String, matrix: [f64; 6], font_size: f64) {
    let text = normalize_line(&text);
    if text.is_empty() {
        return;
    }
    segments.push(Segment {
        x: matrix[4],
        y: matrix[5],
        font_size: font_size.max(1.0),
        text,
    });
}

fn group_lines(mut segments: Vec<Segment>, page_number: u32) -> Vec<TextLine> {
    segments.sort_by(|left, right| {
        right
            .y
            .partial_cmp(&left.y)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                left.x
                    .partial_cmp(&right.x)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });
    let mut lines: Vec<TextLine> = Vec::new();
    let mut current: Vec<Segment> = Vec::new();
    let mut current_y: Option<f64> = None;
    let mut current_size = 12.0_f64;
    for segment in segments {
        let tolerance = (segment.font_size * 0.35).max(2.0);
        let same_line = current_y.is_some_and(|y| (y - segment.y).abs() <= tolerance);
        if current_y.is_some() && !same_line {
            flush_line(
                &mut lines,
                &mut current,
                page_number,
                current_y,
                current_size,
            );
            current_y = None;
        }
        if current_y.is_none() {
            current_y = Some(segment.y);
            current_size = segment.font_size;
        }
        current_size = current_size.max(segment.font_size);
        current.push(segment);
    }
    flush_line(
        &mut lines,
        &mut current,
        page_number,
        current_y,
        current_size,
    );
    lines
}

fn flush_line(
    lines: &mut Vec<TextLine>,
    current: &mut Vec<Segment>,
    page_number: u32,
    y: Option<f64>,
    font_size: f64,
) {
    if current.is_empty() {
        return;
    }
    current.sort_by(|left, right| {
        left.x
            .partial_cmp(&right.x)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let x = current.first().map(|segment| segment.x).unwrap_or(0.0);
    let text = current
        .iter()
        .map(|segment| segment.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let text = normalize_line(&text);
    current.clear();
    if text.is_empty() {
        return;
    }
    lines.push(TextLine {
        page_number,
        text,
        x,
        y: y.unwrap_or(0.0),
        font_size,
    });
}

fn page_encodings<'a>(
    doc: &'a Document,
    page_id: lopdf::ObjectId,
) -> BTreeMap<Vec<u8>, Encoding<'a>> {
    doc.get_page_fonts(page_id)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, font)| {
            font.get_font_encoding(doc)
                .ok()
                .map(|encoding| (name, encoding))
        })
        .collect()
}

fn fallback_lines(doc: &Document, page_number: u32) -> Vec<TextLine> {
    let Ok(text) = doc.extract_text(&[page_number]) else {
        return Vec::new();
    };
    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let text = normalize_line(line);
            if text.is_empty() {
                return None;
            }
            Some(TextLine {
                page_number,
                text,
                x: 0.0,
                y: 1000.0 - (index as f64 * 12.0),
                font_size: 12.0,
            })
        })
        .collect()
}

fn matrix(operands: &[Object]) -> Option<[f64; 6]> {
    if operands.len() < 6 {
        return None;
    }
    let mut out = [0.0; 6];
    for (index, value) in operands.iter().take(6).enumerate() {
        out[index] = match value {
            Object::Integer(number) => *number as f64,
            Object::Real(number) => *number as f64,
            _ => return None,
        };
    }
    Some(out)
}

fn pair(operands: &[Object]) -> Option<(f64, f64)> {
    if operands.len() < 2 {
        return None;
    }
    Some((number(operands.first())?, number(operands.get(1))?))
}

fn number(object: Option<&Object>) -> Option<f64> {
    match object? {
        Object::Integer(value) => Some(*value as f64),
        Object::Real(value) => Some(*value as f64),
        _ => None,
    }
}

fn normalize_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normalize_text(text: &str) -> String {
    text.lines()
        .map(normalize_line)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn bounded(text: &str, max_chars: usize) -> String {
    let text = normalize_text(text);
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push('…');
    out
}

fn same_text(left: &str, right: &str) -> bool {
    normalize_line(left) == normalize_line(right)
}

/// The vertical text band around the photo, with a line-order fallback.
fn nearby_lines(page: &PageContext, placement: Option<Placement>) -> Vec<String> {
    let Some(placement) = placement else {
        let mut lines: Vec<&TextLine> = page.lines.iter().take(6).collect();
        if page.lines.len() > 12 {
            lines.extend(page.lines.iter().rev().take(6));
        }
        return lines.into_iter().map(|line| line.text.clone()).collect();
    };
    let bottom = placement.y - 150.0;
    let top = placement.y + placement.height + 150.0;
    // Placement is the whole point of the nearby rule. Do not fall back to
    // page-wide text when nothing lies in the image's band: a caption on the
    // other half of the page belongs to a different image.
    page.lines
        .iter()
        .filter(|line| line.y >= bottom && line.y <= top)
        .map(|line| line.text.clone())
        .collect()
}

fn caption_lines(page: &PageContext, placement: Option<Placement>) -> Vec<String> {
    // Only a line that actually reads like a caption is a caption. A short
    // nearby label is handled by the nearby-text rule, where several separate
    // labels cannot become a group link without a group phrase.
    nearby_lines(page, placement)
        .into_iter()
        .filter(|line| is_caption_like(line))
        .collect()
}

fn detect_heading(page: &PageContext, placement: Option<Placement>) -> Option<TextLine> {
    let above = placement.map(|placement| placement.y + placement.height);
    let headings: Vec<&TextLine> = page
        .lines
        .iter()
        .filter(|line| is_heading_like(&line.text) && !is_caption_like(&line.text))
        .filter(|line| above.is_none_or(|top| line.y >= top))
        .collect();
    // A heading below the image is more likely a nearby label. Only a heading
    // above the image, or any heading when placement is unknown, is a section
    // heading.
    headings.last().map(|line| (*line).clone())
}

fn biography_lines(page: &PageContext, heading: &Option<TextLine>, caption: &str) -> Vec<String> {
    page.lines
        .iter()
        .filter(|line| {
            !is_caption_like(&line.text)
                && heading
                    .as_ref()
                    .is_none_or(|heading| !same_text(&heading.text, &line.text))
                && !same_text(&line.text, caption)
        })
        .map(|line| line.text.clone())
        .collect()
}

/// A short label under or beside a portrait, such as a name and its dates.
/// A biography sentence that happens to be short is still prose, not a label.
fn is_label_like(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 50 {
        return false;
    }
    if trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?') {
        return false;
    }
    if trimmed.split_whitespace().count() > 8 {
        return false;
    }
    let lower = format!(" {} ", trimmed.to_lowercase());
    const PROSE: &[&str] = &[
        " the ",
        " was ",
        " were ",
        " is ",
        " are ",
        " she ",
        " he ",
        " his ",
        " her ",
        " of ",
        " in ",
        " to ",
        " with ",
        " for ",
        " from ",
        " that ",
        " this ",
        " had ",
        " has ",
        " have ",
        " married ",
        " child ",
        " children ",
        " born ",
        " died ",
    ];
    !PROSE.iter().any(|word| lower.contains(word))
}

/// A short comma- or semicolon-separated list of names, as printed under a
/// group photograph. A biography sentence with commas is not a name list.
fn is_name_list_like(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 160 || trimmed.ends_with('.') {
        return false;
    }
    let parts: Vec<&str> = trimmed
        .split([',', ';'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() < 2 {
        return false;
    }
    parts.iter().all(|part| {
        let words = part.split_whitespace().count();
        let starts_uppercase = part
            .chars()
            .find(|ch| ch.is_alphabetic())
            .is_some_and(|ch| ch.is_uppercase());
        (1..=6).contains(&words) && starts_uppercase
    })
}

fn is_caption_like(text: &str) -> bool {
    let lower = text.to_lowercase();
    const MARKERS: &[&str] = &[
        "photo",
        "photograph",
        "picture",
        "pictured",
        "image",
        "figure",
        "plate",
        "left to right",
        "from left",
        "front row",
        "back row",
        "standing",
        "seated",
        "hình",
        "ảnh",
        "trái sang phải",
        "hàng trước",
        "hàng sau",
        "chụp cùng",
    ];
    MARKERS.iter().any(|marker| lower.contains(marker))
}

fn is_group_phrase(text: &str) -> bool {
    let lower = format!(" {} ", text.to_lowercase());
    const PHRASES: &[&str] = &[
        " left to right ",
        " from left ",
        " front row ",
        " back row ",
        " standing ",
        " seated ",
        " pictured with ",
        " together with ",
        " and ",
        " và ",
        " with ",
        " cùng ",
        " trái sang phải ",
        " hàng trước ",
        " hàng sau ",
        " , ",
        " ; ",
    ];
    PHRASES.iter().any(|phrase| lower.contains(phrase))
}

fn is_heading_like(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 90 {
        return false;
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if words.is_empty() || words.len() > 12 {
        return false;
    }
    if trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?') {
        return false;
    }
    let lower = trimmed.to_lowercase();
    if [
        "biography",
        "tiểu sử",
        "life story",
        "family history",
        "about ",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
    {
        return true;
    }
    let all_caps = trimmed
        .chars()
        .filter(|ch| ch.is_alphabetic())
        .all(|ch| ch.is_uppercase());
    let title_case = words
        .iter()
        .filter(|word| word.chars().next().is_some_and(|ch| ch.is_uppercase()))
        .count()
        >= words.len().saturating_sub(1).max(1);
    all_caps || title_case
}

fn first_sentence_naming(text: &str, hit: &NameHit) -> String {
    let folded_name = person_search::fold(&hit.matched_text);
    for sentence in text.split_inclusive(['.', '!', '?', '\n']) {
        let folded = person_search::fold(sentence);
        if folded.contains(&folded_name) {
            let sentence = normalize_line(sentence);
            if !sentence.is_empty() {
                return sentence;
            }
        }
    }
    text.to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Document, Object, Stream};

    fn synthetic_jpeg() -> Vec<u8> {
        use image::{ImageBuffer, ImageFormat, Rgb};
        use std::io::Cursor;
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(8, 8, Rgb([90, 140, 170]));
        let mut jpeg = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
            .expect("encode JPEG");
        jpeg
    }

    fn escape_pdf(text: &str) -> String {
        text.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    }

    /// A synthetic PDF with a framed image at `image_y` and text lines at the
    /// stated baselines.
    fn context_pdf(lines: &[(f64, &str)], image_y: Option<f64>) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 8,
                "Height" => 8,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            synthetic_jpeg(),
        ));
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
            "XObject" => dictionary! { "Im0" => image_id },
        });
        let mut content = Vec::new();
        if let Some(y) = image_y {
            content.extend_from_slice(format!("q 200 0 0 150 72 {y} cm /Im0 Do Q\n").as_bytes());
        }
        for (y, text) in lines {
            content.extend_from_slice(
                format!("BT /F1 12 Tf 72 {y} Td ({}) Tj ET\n", escape_pdf(text)).as_bytes(),
            );
        }
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save PDF");
        bytes
    }

    fn two_page_context_pdf(
        first: &[(f64, &str)],
        second: &[(f64, &str)],
        image_y: f64,
    ) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
            "Encoding" => "WinAnsiEncoding",
        });
        let image_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => 8,
                "Height" => 8,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            synthetic_jpeg(),
        ));
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
            "XObject" => dictionary! { "Im0" => image_id },
        });
        let mut first_content = format!("q 200 0 0 150 72 {image_y} cm /Im0 Do Q\n").into_bytes();
        for (y, text) in first {
            first_content.extend_from_slice(
                format!("BT /F1 12 Tf 72 {y} Td ({}) Tj ET\n", escape_pdf(text)).as_bytes(),
            );
        }
        let mut second_content = Vec::new();
        for (y, text) in second {
            second_content.extend_from_slice(
                format!("BT /F1 12 Tf 72 {y} Td ({}) Tj ET\n", escape_pdf(text)).as_bytes(),
            );
        }
        let first_content_id = doc.add_object(Stream::new(dictionary! {}, first_content));
        let second_content_id = doc.add_object(Stream::new(dictionary! {}, second_content));
        let first_page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => first_content_id,
        });
        let second_page = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => second_content_id,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![first_page.into(), second_page.into()],
                "Count" => 2,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("save PDF");
        bytes
    }

    fn person(key: &str, name: &str, years: &[&str]) -> PersonContext {
        PersonContext {
            person_key: key.to_string(),
            display_name: name.to_string(),
            aliases: Vec::new(),
            generation: None,
            evidence_status: "unverified".to_string(),
            dates: years.iter().map(|year| year.to_string()).collect(),
            parent_keys: Vec::new(),
            spouse_keys: Vec::new(),
        }
    }

    fn placement(y: f64) -> Placement {
        Placement {
            x: 72.0,
            y,
            width: 200.0,
            height: 150.0,
        }
    }

    #[test]
    fn layout_extraction_reads_positioned_text_lines() {
        let pdf = context_pdf(
            &[
                (700.0, "Biography of Synthetic Alpha"),
                (300.0, "Photo: Synthetic Alpha"),
            ],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let page = context.page(1).expect("page one");
        assert!(page.text().contains("Biography of Synthetic Alpha"));
        assert!(page.text().contains("Photo: Synthetic Alpha"));
        let first = page.lines.first().expect("first line");
        assert!(first.y > page.lines.last().expect("last line").y);
    }

    #[test]
    fn an_explicit_caption_is_the_strongest_candidate() {
        let pdf = context_pdf(
            &[
                (700.0, "Biography of Synthetic Alpha"),
                (300.0, "Photo: Synthetic Alpha"),
            ],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![person("alpha", "Synthetic Alpha", &["1940"])];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 1);
        let candidate = &proposal.candidates[0];
        assert_eq!(candidate.person_key, "alpha");
        assert_eq!(candidate.confidence, Confidence::High);
        assert_eq!(candidate.source_kind, SourceKind::Caption);
        assert_eq!(candidate.source_page, 1);
        assert!(candidate.supporting_text.contains("Photo: Synthetic Alpha"));
    }

    #[test]
    fn a_group_caption_proposes_each_named_person() {
        let pdf = context_pdf(
            &[(300.0, "Left to right: Synthetic Alpha and Synthetic Beta")],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![
            person("alpha", "Synthetic Alpha", &[]),
            person("beta", "Synthetic Beta", &[]),
        ];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 2);
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.source_kind == SourceKind::ExplicitGroup));
    }

    #[test]
    fn a_namesake_is_distinguished_by_a_stated_year_and_the_other_stays_ambiguous() {
        let pdf = context_pdf(
            &[
                (300.0, "Photo: Synthetic Alpha"),
                (280.0, "Synthetic Alpha was born in 1940."),
            ],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let mut older = person("alpha-1940", "Synthetic Alpha", &["1940"]);
        older.evidence_status = "verified".to_string();
        let younger = person("alpha-1950", "Synthetic Alpha", &["1950"]);
        let proposal =
            propose_layout_links(&context, 1, Some(placement(400.0)), None, &[older, younger]);
        assert_eq!(proposal.candidates.len(), 2);
        assert_eq!(proposal.candidates[0].person_key, "alpha-1940");
        assert!(proposal.candidates[0].confidence_basis.contains("1940"));
        assert!(proposal.candidates[1].ambiguity.is_some());
        assert_eq!(proposal.candidates[1].confidence, Confidence::Low);
    }

    #[test]
    fn a_heading_can_identify_a_biography_subject_without_a_caption() {
        let pdf = context_pdf(
            &[
                (700.0, "Biography of Synthetic Alpha"),
                (680.0, "Synthetic Alpha was born in 1940."),
            ],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![person("alpha", "Synthetic Alpha", &["1940"])];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 1);
        assert_eq!(
            proposal.candidates[0].source_kind,
            SourceKind::SectionHeading
        );
        assert_eq!(proposal.candidates[0].confidence, Confidence::High);
    }

    #[test]
    fn an_unsupported_photo_is_left_unlinked() {
        let pdf = context_pdf(
            &[
                (300.0, "Photo: family gathering"),
                (280.0, "The family gathered in 1962."),
            ],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![person("alpha", "Synthetic Alpha", &["1940"])];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert!(proposal.candidates.is_empty());
        assert!(proposal.unlinked_reason.is_some());
    }

    #[test]
    fn a_biography_with_several_names_does_not_link_them_all() {
        let pdf = context_pdf(
            &[
                (700.0, "Family history"),
                (
                    680.0,
                    "Synthetic Alpha and Synthetic Beta lived in the village.",
                ),
            ],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![
            person("alpha", "Synthetic Alpha", &[]),
            person("beta", "Synthetic Beta", &[]),
        ];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert!(proposal.candidates.is_empty());
        assert!(proposal
            .unlinked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("several recorded people")));
    }
    #[test]
    fn a_biography_continued_on_the_next_page_is_a_low_confidence_fallback() {
        let pdf = two_page_context_pdf(
            &[(700.0, "Synthetic photograph")],
            &[
                (700.0, "Biography of Synthetic Alpha"),
                (680.0, "Synthetic Alpha was born in 1940."),
            ],
            400.0,
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![person("alpha", "Synthetic Alpha", &["1940"])];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 1);
        assert_eq!(proposal.candidates[0].person_key, "alpha");
        assert_eq!(proposal.candidates[0].source_kind, SourceKind::AdjacentPage);
        assert_eq!(proposal.candidates[0].source_page, 2);
        assert_eq!(proposal.candidates[0].confidence, Confidence::Low);
    }
    #[test]
    fn a_caption_without_word_spaces_still_names_the_person() {
        let pdf = context_pdf(&[(300.0, "PhotosofSyntheticAlphaFamily")], Some(400.0));
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![person("alpha", "Synthetic Alpha", &[])];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 1);
        assert_eq!(proposal.candidates[0].person_key, "alpha");
        assert_eq!(proposal.candidates[0].match_kind, MatchKind::Partial);
        assert_eq!(proposal.candidates[0].source_kind, SourceKind::Caption);
        assert!(proposal.candidates[0]
            .confidence_basis
            .contains("word boundaries"));
    }

    #[test]
    fn a_comma_separated_name_list_matches_final_name_words() {
        let pdf = context_pdf(&[(300.0, "Synthetic Alpha, Synthetic Beta")], Some(400.0));
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![
            person("alpha", "Recorded Synthetic Alpha", &[]),
            person("beta", "Recorded Synthetic Beta", &[]),
        ];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 2);
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.source_kind == SourceKind::NearbyText));
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.match_kind == MatchKind::Partial));
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.confidence == Confidence::Medium));
    }

    #[test]
    fn a_shared_final_name_is_ambiguous_and_low_confidence() {
        let pdf = context_pdf(&[(300.0, "Synthetic Alpha, Other Person")], Some(400.0));
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![
            person("first", "Recorded Synthetic Alpha", &[]),
            person("second", "Second Synthetic Alpha", &[]),
        ];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert_eq!(proposal.candidates.len(), 2);
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.confidence == Confidence::Low));
        assert!(proposal
            .candidates
            .iter()
            .all(|candidate| candidate.ambiguity.is_some()));
    }

    #[test]
    fn separate_nearby_labels_do_not_become_a_group_link() {
        let pdf = context_pdf(
            &[(300.0, "Synthetic Alpha"), (280.0, "Synthetic Beta")],
            Some(400.0),
        );
        let context = extract_context(&pdf).expect("context");
        let catalog = vec![
            person("alpha", "Synthetic Alpha", &[]),
            person("beta", "Synthetic Beta", &[]),
        ];
        let proposal = propose_layout_links(&context, 1, Some(placement(400.0)), None, &catalog);
        assert!(proposal.candidates.is_empty());
        assert!(proposal
            .unlinked_reason
            .as_deref()
            .is_some_and(|reason| reason.contains("several recorded people")));
    }
}
