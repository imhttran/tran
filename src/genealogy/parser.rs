//! Catalog parser and candidate extraction (TRAN-002C).
//!
//! This module is pure and offline. It reads a private family catalog as plain
//! text, segments it into catalog entries, assigns each entry a stable
//! catalog-entry key, and emits candidate person, name-variant, branch, and
//! relationship assertions with evidence links.
//!
//! The parser never merges identities, never promotes an assertion to
//! `verified`, and never writes to the genealogy database. Persistence lives in
//! TRAN-002B; this module only produces an in-memory [`CatalogParse`].
//!
//! See `docs/plans/TRAN-002-genealogy-data-model.md` for the catalog design note.

use std::collections::BTreeMap;

use super::dates::{self, DateParse, DatePrecision, DateValue};

/// The smallest PDF page number accepted as evidence for this input.
pub const MIN_PAGE: u32 = 1;
/// The largest PDF page number accepted as evidence for this input.
pub const MAX_PAGE: u32 = 56;

/// The parse outcome of a single catalog entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseOutcome {
    /// The entry produced usable candidates.
    Parsed,
    /// The entry produced candidates but carries an unresolved ambiguity.
    Ambiguous,
    /// The entry could not be parsed and is rejected with a reason.
    Rejected,
}

/// A reason code attached to an ambiguity or rejection.
///
/// Some variants are reserved for later ambiguity rules and are not all
/// constructed by the current parser.
#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasonCode {
    /// The entry had no displayable name text.
    MissingName,
    /// A page reference fell outside the accepted 1-56 range.
    PageOutOfRange,
    /// Two different names appear for what looks like the same slot.
    ConflictingName,
    /// More than one entry shares the same name (distinct candidates).
    SameName,
    /// Parentage is stated without a resolvable parent.
    UnclearParentage,
    /// A relationship rests only on same-name inference.
    UnsourcedRelationship,
    /// A date is stated but cannot be classified without guessing.
    AmbiguousDate,
}

/// A single evidence locator attached to a candidate assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Evidence {
    /// A resolved source page within the accepted range.
    Page(u32),
    /// An unresolved locator. The page is not guessed.
    Unresolved(String),
}

/// The kind of candidate assertion derived from an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionKind {
    /// The entry's own display name.
    DisplayName,
    /// An additional name spelling or alias.
    NameVariant,
    /// The family branch the entry belongs to.
    Branch,
    /// A parent-child relationship.
    ParentChild,
    /// A spouse relationship.
    Spouse,
    /// An explicitly stated birth date.
    BirthDate,
    /// An explicitly stated death date.
    DeathDate,
    /// Date-like text that cannot be classified without guessing.
    DateUnresolved,
}

/// The review status of a candidate assertion. The parser only ever emits
/// `Candidate`; it never emits `Verified`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewStatus {
    /// The parser produced this as an unverified candidate.
    Candidate,
}

/// A proposed assertion produced by the parser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateAssertion {
    /// The catalog-entry key that owns this assertion.
    pub catalog_entry_key: String,
    /// The assertion kind.
    pub kind: AssertionKind,
    /// A stable key within (entry, kind). Derived from the value text.
    pub assertion_key: String,
    /// The value text (original spelling preserved).
    pub value_text: String,
    /// The review status. Always [`ReviewStatus::Candidate`].
    pub review_status: ReviewStatus,
    /// Evidence locators supporting the assertion.
    pub evidence: Vec<Evidence>,
    /// Optional reason code when the assertion is unsourced or ambiguous.
    pub reason: Option<ReasonCode>,
    /// The structured date, for `birth_date` and `death_date` assertions only.
    ///
    /// `None` on every other kind, and on `date_unresolved`, which keeps the
    /// source wording but must not be read as a birth or death date.
    pub date: Option<DateValue>,
}

/// The result of parsing one catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    /// The stable catalog-entry key.
    pub key: String,
    /// The one-based position of the entry in the catalog text.
    pub ordinal: usize,
    /// The parse outcome.
    pub outcome: ParseOutcome,
    /// The reason code when the entry is ambiguous or rejected.
    pub reason: Option<ReasonCode>,
    /// The display name candidate, if one was found.
    pub display_name: Option<String>,
    /// The proposed assertions for this entry, in a deterministic order.
    pub assertions: Vec<CandidateAssertion>,
}

/// The full parse result for a catalog text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CatalogParse {
    /// Every catalog entry, in catalog order.
    pub entries: Vec<CatalogEntry>,
}

impl CatalogParse {
    fn new(entries: Vec<CatalogEntry>) -> Self {
        Self { entries }
    }

    /// The number of parsed entries.
    pub fn parsed_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.outcome == ParseOutcome::Parsed)
            .count()
    }

    /// The number of ambiguous entries.
    pub fn ambiguous_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.outcome == ParseOutcome::Ambiguous)
            .count()
    }

    /// The number of rejected entries.
    pub fn rejected_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.outcome == ParseOutcome::Rejected)
            .count()
    }

    /// The number of entries with an unresolved locator.
    pub fn unmatched_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| {
                e.assertions.iter().any(|a| {
                    a.evidence
                        .iter()
                        .any(|ev| matches!(ev, Evidence::Unresolved(_)))
                })
            })
            .count()
    }

    /// The stable keys of every entry, in catalog order.
    pub fn keys(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.key.clone()).collect()
    }
}

// ---------------------------------------------------------------------------
// Key derivation
// ---------------------------------------------------------------------------

/// Derives a stable catalog-entry key from the entry ordinal and the entry's
/// normalized raw text. The key is stable across parses of unchanged input.
pub fn catalog_entry_key(ordinal: usize, raw_text: &str) -> String {
    let normalized = normalize_for_key(raw_text);
    let digest = fnv1a64(normalized.as_bytes());
    format!("ce-{ordinal:04}-{digest:016x}")
}

fn normalize_for_key(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// Segmentation
// ---------------------------------------------------------------------------

/// Segments a catalog text into raw entry blocks. A new entry starts at a line
/// that begins with a numbered heading (`1.`, `12)`, etc.) or a markdown
/// heading (`#`, `##`). Blank lines do not split entries; non-heading lines
/// are appended to the current entry.
pub fn segment_entries(text: &str) -> Vec<String> {
    // The real catalog is a Markdown table under "Entity index". When table
    // rows are present, each row is one catalog entry; section headings and
    // prose are not entries.
    if text.lines().any(is_catalog_table_row) {
        return text
            .lines()
            .filter(|line| is_catalog_table_row(line))
            .map(|line| line.trim_end().to_string())
            .filter(|entry| !entry.trim().is_empty())
            .collect();
    }

    let mut entries: Vec<String> = Vec::new();
    let mut current: Vec<&str> = Vec::new();

    for line in text.lines() {
        if is_entry_heading(line) && !current.is_empty() {
            entries.push(current.join("\n"));
            current.clear();
        }
        if !current.is_empty() || is_entry_heading(line) {
            current.push(line);
        } else if !line.trim().is_empty() && entries.is_empty() {
            // Text before the first heading forms its own unheaded entry.
            current.push(line);
        }
    }
    if !current.is_empty() {
        entries.push(current.join("\n"));
    }

    entries
        .into_iter()
        .map(|entry| entry.trim_end().to_string())
        .filter(|entry| !entry.trim().is_empty())
        .collect()
}

fn is_entry_heading(line: &str) -> bool {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return true;
    }
    let chars = trimmed.chars();
    let mut saw_digit = false;
    for c in chars {
        if c.is_ascii_digit() {
            saw_digit = true;
            continue;
        }
        if saw_digit && (c == '.' || c == ')') {
            return true;
        }
        break;
    }
    false
}

// ---------------------------------------------------------------------------
// Field extraction
// ---------------------------------------------------------------------------

const PAGE_MARKERS: [&str; 4] = ["trang", "page", "p.", "pdf"];

/// Extracts page references from a text. Returns resolved pages and unresolved
/// locator strings. Out-of-range pages are reported through the caller.
fn extract_pages(text: &str) -> (Vec<u32>, Vec<String>) {
    let mut pages = Vec::new();
    let mut unresolved = Vec::new();
    // The markers are ASCII, so use the length-preserving lowering. A full
    // `to_lowercase` can change byte length (for example `İ`), which would make
    // `abs` an invalid boundary in `text`.
    let lower = text.to_ascii_lowercase();
    for marker in PAGE_MARKERS {
        let mut search = 0;
        while let Some(pos) = lower[search..].find(marker) {
            let abs = search + pos;
            let rest = &text[abs + marker.len()..];
            let digits: String = rest
                .chars()
                .take_while(|c| c.is_whitespace() || c.is_ascii_digit() || *c == ':' || *c == '=')
                .filter(|c| c.is_ascii_digit())
                .collect();
            if digits.is_empty() {
                unresolved.push(marker.to_string());
            } else if let Ok(page) = digits.parse::<u32>() {
                pages.push(page);
            }
            search = abs + marker.len();
        }
    }
    (pages, unresolved)
}

/// Parses a full catalog text into a deterministic [`CatalogParse`].
///
/// Every entry is classified exactly once as parsed, ambiguous, or rejected.
/// Duplicate names never merge: each entry gets its own key. No assertion is
/// ever marked verified.
pub fn parse_catalog(text: &str) -> CatalogParse {
    let raw_entries = segment_entries(text);

    // First pass: build entries and collect names for same-name detection.
    let mut entries: Vec<CatalogEntry> = Vec::with_capacity(raw_entries.len());
    for (index, raw) in raw_entries.iter().enumerate() {
        let ordinal = index + 1;
        let key = catalog_entry_key(ordinal, raw);
        let display_name = extract_display_name(raw);
        let (pages, _unresolved_markers) = extract_pages(raw);
        let out_of_range = pages.iter().any(|p| *p < MIN_PAGE || *p > MAX_PAGE);

        let mut assertions = Vec::new();
        let mut reason = None;

        let outcome = if display_name.is_none() {
            reason = Some(ReasonCode::MissingName);
            ParseOutcome::Rejected
        } else if out_of_range {
            reason = Some(ReasonCode::PageOutOfRange);
            ParseOutcome::Rejected
        } else {
            ParseOutcome::Parsed
        };

        let evidence = page_evidence(&pages);

        if let Some(name) = &display_name {
            assertions.push(CandidateAssertion {
                catalog_entry_key: key.clone(),
                kind: AssertionKind::DisplayName,
                assertion_key: format!("display:{}", fnv1a64(name.as_bytes())),
                value_text: name.clone(),
                review_status: ReviewStatus::Candidate,
                evidence: evidence.clone(),
                reason: reason.clone(),
                date: None,
            });
        }

        // Name variants and aliases preserve original spelling.
        for variant in extract_name_variants(raw) {
            assertions.push(CandidateAssertion {
                catalog_entry_key: key.clone(),
                kind: AssertionKind::NameVariant,
                assertion_key: format!("variant:{}", fnv1a64(variant.as_bytes())),
                value_text: variant,
                review_status: ReviewStatus::Candidate,
                evidence: evidence.clone(),
                reason: None,
                date: None,
            });
        }

        // Branch assertions.
        if let Some(branch) = extract_branch(raw) {
            assertions.push(CandidateAssertion {
                catalog_entry_key: key.clone(),
                kind: AssertionKind::Branch,
                assertion_key: format!("branch:{}", fnv1a64(branch.as_bytes())),
                value_text: branch,
                review_status: ReviewStatus::Candidate,
                evidence: evidence.clone(),
                reason: None,
                date: None,
            });
        }

        // Relationship assertions are always candidates. When they rest only on
        // a same-name mention, they carry an unsourced reason code.
        for rel in extract_relationships(raw) {
            assertions.push(CandidateAssertion {
                catalog_entry_key: key.clone(),
                kind: rel.kind,
                assertion_key: format!(
                    "{}:{}",
                    match rel.kind {
                        AssertionKind::ParentChild => "parent_child",
                        AssertionKind::Spouse => "spouse",
                        _ => "rel",
                    },
                    fnv1a64(rel.value.as_bytes())
                ),
                value_text: rel.value,
                review_status: ReviewStatus::Candidate,
                evidence: evidence.clone(),
                reason: rel.reason,
                date: None,
            });
        }

        // Birth and death dates are candidates until a reviewer accepts them.
        // An unclassifiable date keeps its wording as `date_unresolved` so the
        // claim stays visible without pretending to be a known date.
        for date in extract_dates(raw) {
            assertions.push(CandidateAssertion {
                catalog_entry_key: key.clone(),
                kind: date.kind,
                assertion_key: date.key,
                value_text: date.raw,
                review_status: ReviewStatus::Candidate,
                evidence: evidence.clone(),
                reason: date.reason,
                date: date.date,
            });
        }

        if matches!(outcome, ParseOutcome::Rejected) {
            // Rejected entries keep their evidence for the report but produce no
            // usable candidates beyond what was collected.
            assertions.clear();
        }

        entries.push(CatalogEntry {
            key,
            ordinal,
            outcome,
            reason,
            display_name,
            assertions,
        });
    }

    // Second pass: mark same-name entries ambiguous (never merged).
    let mut name_counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &entries {
        if let Some(name) = &entry.display_name {
            *name_counts.entry(name.clone()).or_insert(0) += 1;
        }
    }
    for entry in &mut entries {
        if let Some(name) = &entry.display_name {
            if name_counts.get(name).copied().unwrap_or(0) > 1
                && entry.outcome == ParseOutcome::Parsed
            {
                entry.outcome = ParseOutcome::Ambiguous;
                entry.reason = Some(ReasonCode::SameName);
            }
        }
    }

    CatalogParse::new(entries)
}

fn page_evidence(pages: &[u32]) -> Vec<Evidence> {
    if pages.is_empty() {
        vec![Evidence::Unresolved("no page reference".to_string())]
    } else {
        let mut sorted: Vec<u32> = pages.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        sorted.into_iter().map(Evidence::Page).collect()
    }
}

fn is_catalog_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.starts_with('|') {
        return false;
    }
    let mut cells = trimmed.split('|');
    cells.next();
    let first = cells.next().unwrap_or("").trim();
    first
        .strip_prefix('P')
        .is_some_and(|rest| rest.len() == 3 && rest.chars().all(|c| c.is_ascii_digit()))
}

fn extract_display_name(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('|') {
            let mut cells = trimmed.split('|');
            cells.next();
            cells.next();
            if let Some(name) = cells.next() {
                let name = name.trim();
                if !name.is_empty() {
                    return Some(name.to_string());
                }
            }
        }
        let without_heading = strip_heading(line);
        let candidate = without_heading
            .split([':', '|', '\u{2013}', '\u{2014}'])
            .next()
            .unwrap_or("")
            .trim();
        if candidate.is_empty() {
            continue;
        }
        let lower = candidate.to_lowercase();
        if lower.contains("trang") || lower.contains("page") {
            continue;
        }
        return Some(candidate.to_string());
    }
    None
}

fn strip_heading(line: &str) -> String {
    let mut trimmed = line.trim();
    if trimmed.starts_with('#') {
        trimmed = trimmed.trim_start_matches('#').trim();
    }
    let mut chars = trimmed.chars().peekable();
    let mut prefix_len = 0;
    let mut saw_digit = false;
    while let Some(&c) = chars.peek() {
        if c.is_ascii_digit() {
            saw_digit = true;
            prefix_len += c.len_utf8();
            chars.next();
            continue;
        }
        if saw_digit && (c == '.' || c == ')') {
            prefix_len += c.len_utf8();
            chars.next();
            break;
        }
        break;
    }
    trimmed[prefix_len..].trim().to_string()
}

fn extract_name_variants(raw: &str) -> Vec<String> {
    let mut variants = Vec::new();
    for line in raw.lines() {
        // `idx` comes from the ASCII lowering and is used to slice `line`, so it
        // must be byte-length preserving. The Unicode keyword check below keeps
        // its own full lowercasing.
        let ascii_lower = line.to_ascii_lowercase();
        let lower = line.to_lowercase();
        if let Some(idx) = ascii_lower.find("alias") {
            if let Some(pos) = line[idx..].find([':', '=']) {
                let value = line[idx + pos + 1..].trim();
                if !value.is_empty() {
                    variants.push(value.to_string());
                }
            }
        }
        if lower.contains("tên khác") || lower.contains("ten khac") {
            if let Some(pos) = line.find([':', '=']) {
                let value = line[pos + 1..].trim();
                if !value.is_empty() {
                    variants.push(value.to_string());
                }
            }
        }
    }
    variants.sort();
    variants.dedup();
    variants
}

fn extract_branch(raw: &str) -> Option<String> {
    for line in raw.lines() {
        let lower = line.to_lowercase();
        if lower.contains("nhánh") || lower.contains("nhanh") || lower.contains("branch") {
            if let Some(pos) = line.find([':', '=']) {
                let value = line[pos + 1..].trim();
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
}

struct RawRelationship {
    kind: AssertionKind,
    value: String,
    reason: Option<ReasonCode>,
}

/// One relationship target extracted from a catalog statement.
struct Target {
    text: String,
    /// The statement names a person the catalog cannot identify (for example
    /// "unnamed fourth wife"). Kept as an assertion so review can see it.
    unresolved: bool,
}

/// Extracts relationship statements from either catalog shape.
///
/// The real catalog is a Markdown table whose *description* cell carries prose
/// facts ("Child of X and Y", "Spouse: Z", "Spouse-column match to W"). The
/// older heading shape states facts as `Label: value` lines. Both are handled;
/// a table row never reaches the label parser, because its first colon is inside
/// prose and would swallow the rest of the row as one value.
fn extract_relationships(raw: &str) -> Vec<RawRelationship> {
    let mut out = Vec::new();
    let mut table_rows = 0usize;
    for line in raw.lines() {
        let trimmed = line.trim();
        if is_catalog_table_row(trimmed) {
            table_rows += 1;
            if let Some(description) = catalog_description_cell(trimmed) {
                out.extend(relationships_from_description(&description));
            }
        }
    }
    if table_rows == 0 {
        out.extend(label_relationships(raw));
    }
    out.sort_by(|a, b| a.value.cmp(&b.value));
    out.dedup_by(|a, b| a.kind == b.kind && a.value == b.value);
    out
}

/// The description cell of `| key | name | description | pages |`.
fn catalog_description_cell(line: &str) -> Option<String> {
    let mut cells = line.trim().split('|');
    cells.next(); // empty before the first separator
    cells.next(); // key
    cells.next(); // name
    cells.next().map(|cell| cell.trim().to_string())
}

/// Splits one description cell into relationship statements.
///
/// The catalog separates facts with `;`, so one row commonly states several
/// relationships. Every statement is evaluated independently; nothing is
/// inferred from ordering, and no target is resolved here.
fn relationships_from_description(description: &str) -> Vec<RawRelationship> {
    let mut out = Vec::new();
    for clause in description.split(';') {
        // The keyword can live inside a bracketed annotation ("(LAYOUT
        // INFERENCE)"), so detect it before annotations are stripped.
        let raw_lower = clause.to_ascii_lowercase();
        let unsourced = raw_lower.contains("layout inference")
            || raw_lower.contains("same name")
            || raw_lower.contains("namesake")
            || raw_lower.contains("cùng tên")
            || raw_lower.contains("cung ten")
            || raw_lower.contains("probable")
            || raw_lower.contains("likely")
            || raw_lower.contains("possibly")
            || raw_lower.contains("presumed")
            || raw_lower.contains("assumed")
            || raw_lower.contains("có thể")
            || raw_lower.contains("co the")
            || raw_lower.contains("đoán")
            || raw_lower.contains("suy");
        let cleaned = strip_annotations(clause);
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }

        if let Some(rest) = leading_rest(cleaned, "child of")
            .or_else(|| leading_rest(cleaned, "con của"))
            .or_else(|| leading_rest(cleaned, "con cua"))
        {
            for target in split_parents(rest) {
                let reason = if target.unresolved {
                    Some(ReasonCode::UnclearParentage)
                } else if unsourced {
                    Some(ReasonCode::UnsourcedRelationship)
                } else {
                    None
                };
                out.push(RawRelationship {
                    kind: AssertionKind::ParentChild,
                    value: target.text,
                    reason,
                });
            }
            continue;
        }

        if let Some(rest) = spouse_rest(cleaned) {
            if let Some(target) = clean_parent(rest) {
                let reason = if target.unresolved {
                    Some(ReasonCode::UnclearParentage)
                } else if unsourced {
                    Some(ReasonCode::UnsourcedRelationship)
                } else {
                    None
                };
                out.push(RawRelationship {
                    kind: AssertionKind::Spouse,
                    value: target.text,
                    reason,
                });
            }
        }
    }
    out
}

/// The old `Label: value` shape, kept for heading-style catalogs.
fn label_relationships(raw: &str) -> Vec<RawRelationship> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let lower = line.to_lowercase();
        let Some(pos) = line.find([':', '=']) else {
            continue;
        };
        // Match the field label, not a substring of the whole line: `contains("con")`
        // would turn "second marriage" into a parent-child assertion, and
        // `contains("cha")` would match "Chandler".
        let label = line[..pos]
            .rsplit('|')
            .next()
            .unwrap_or_default()
            .trim()
            .trim_matches(|c: char| !c.is_alphanumeric())
            .to_lowercase();
        let label_matches =
            |keyword: &str| label == keyword || label.ends_with(&format!(" {keyword}"));
        let is_parent = [
            "cha", "mẹ", "me", "parent", "con", "child", "father", "mother",
        ]
        .iter()
        .any(|keyword| label_matches(keyword));
        let is_spouse = ["vợ", "vo", "chồng", "chong", "spouse", "wife", "husband"]
            .iter()
            .any(|keyword| label_matches(keyword));
        if !(is_parent || is_spouse) {
            continue;
        }
        let value = line[pos + 1..].trim();
        if value.is_empty() {
            continue;
        }
        let kind = if is_spouse {
            AssertionKind::Spouse
        } else {
            AssertionKind::ParentChild
        };
        // A relationship that names no distinct person and only repeats a
        // same-name mention is unsourced.
        let reason = if lower.contains("cùng tên")
            || lower.contains("cung ten")
            || lower.contains("same name")
            || lower.contains("đoán")
            || lower.contains("doan")
            || lower.contains("suy")
        {
            Some(ReasonCode::UnsourcedRelationship)
        } else {
            None
        };
        out.push(RawRelationship {
            kind,
            value: value.to_string(),
            reason,
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Date extraction
// ---------------------------------------------------------------------------

/// One date candidate extracted from a catalog statement.
struct RawDate {
    kind: AssertionKind,
    key: String,
    raw: String,
    date: Option<DateValue>,
    reason: Option<ReasonCode>,
}

impl RawDate {
    /// A birth or death date with structured components.
    fn dated(kind: AssertionKind, value: DateValue) -> Self {
        let signature = format!(
            "{}|{}|{}",
            value.start,
            value.end.clone().unwrap_or_default(),
            value.precision.as_str()
        );
        Self {
            key: format!("{}:{}", date_kind_slug(kind), fnv1a64(signature.as_bytes())),
            raw: value.raw.clone(),
            date: Some(value),
            reason: None,
            kind,
        }
    }

    /// Date-like text that must not be read as a birth or death date.
    fn unresolved(raw: String) -> Self {
        let normalized = normalize_for_key(&raw);
        Self {
            key: format!(
                "{}:{}",
                date_kind_slug(AssertionKind::DateUnresolved),
                fnv1a64(normalized.as_bytes())
            ),
            raw,
            date: None,
            reason: Some(ReasonCode::AmbiguousDate),
            kind: AssertionKind::DateUnresolved,
        }
    }
}

fn date_kind_slug(kind: AssertionKind) -> &'static str {
    match kind {
        AssertionKind::BirthDate => "birth_date",
        AssertionKind::DeathDate => "death_date",
        _ => "date_unresolved",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DateRole {
    Birth,
    Death,
    Lifespan,
}

/// Extracts explicitly stated birth, death, and lifespan dates from either
/// catalog shape.
///
/// Nothing is inferred. A lone unlabelled year is kept as `date_unresolved`
/// because the source does not say whether it is a birth or a death, and an
/// ambiguous day/month order is kept the same way.
fn extract_dates(raw: &str) -> Vec<RawDate> {
    let mut out = Vec::new();
    let mut table_rows = 0usize;
    for line in raw.lines() {
        let trimmed = line.trim();
        if is_catalog_table_row(trimmed) {
            table_rows += 1;
            if let Some(description) = catalog_description_cell(trimmed) {
                out.extend(dates_from_description(&description));
            }
        }
    }
    if table_rows == 0 {
        out.extend(label_dates(raw));
    }
    out.sort_by(|left, right| left.key.cmp(&right.key));
    out.dedup_by(|left, right| left.kind == right.kind && left.key == right.key);
    out
}

/// Splits one description cell into date statements.
///
/// A relationship clause is skipped first, so the dates in `Spouse: X
/// (1878–1940)` are never attributed to the entry itself.
fn dates_from_description(description: &str) -> Vec<RawDate> {
    let mut out = Vec::new();
    for clause in description.split(';') {
        let cleaned = strip_annotations(clause);
        let cleaned = cleaned.trim();
        if cleaned.is_empty() {
            continue;
        }
        if is_relationship_clause(cleaned) {
            continue;
        }
        if let Some((role, rest)) = date_role(cleaned) {
            out.extend(push_date_role(role, rest));
        } else if is_date_only_clause(cleaned) {
            out.extend(push_date_role(DateRole::Lifespan, cleaned));
        }
    }
    out
}

/// The older `Label: value` shape, plus a trailing `(1877–1922)` lifespan.
fn label_dates(raw: &str) -> Vec<RawDate> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((role, rest)) = date_role(trimmed) {
            out.extend(push_date_role(role, rest));
            continue;
        }
        if let Some(group) = trailing_parenthetical(trimmed) {
            if is_date_only_clause(group) {
                out.extend(push_date_role(DateRole::Lifespan, group));
            }
        }
    }
    out
}

fn is_relationship_clause(clause: &str) -> bool {
    let lower = clause.to_lowercase();
    [
        "child of",
        "con của",
        "con cua",
        "spouse",
        "wife",
        "husband",
        "vợ",
        "vo",
        "chồng",
        "chong",
    ]
    .iter()
    .any(|marker| lower.starts_with(marker))
}

/// A clause that is only a date expression, such as `1877–1922` or `c. 1914`.
///
/// A leading certainty marker is removed first, so an approximate bare year is
/// still kept (as unresolved) instead of being dropped silently.
fn is_date_only_clause(text: &str) -> bool {
    let (_, body) = dates::split_qualifier(text);
    let body = body.trim();
    !body.is_empty()
        && body.chars().all(|ch| {
            ch.is_ascii_digit() || matches!(ch, ' ' | '\t' | '-' | '–' | '—' | '.' | '/' | '?')
        })
}

/// The last parenthesised group of a line, used for `Name (1877–1922)`.
fn trailing_parenthetical(line: &str) -> Option<&str> {
    let end = line.rfind(')')?;
    let start = line[..end].rfind('(')?;
    Some(line[start + 1..end].trim())
}

/// Reads a birth, death, or lifespan label at the start of a clause.
fn date_role(clause: &str) -> Option<(DateRole, &str)> {
    let lifespan = [
        "dates as printed",
        "dates as recorded",
        "life dates",
        "lifespan",
        "cuộc đời",
        "cuoc doi",
        "dates",
    ];
    let birth = [
        "date of birth",
        "birth date",
        "ngày sinh",
        "ngay sinh",
        "năm sinh",
        "nam sinh",
        "birth",
        "born",
        "sinh",
    ];
    let death = [
        "date of death",
        "death date",
        "ngày mất",
        "ngay mat",
        "năm mất",
        "nam mat",
        "qua đời",
        "qua doi",
        "death",
        "died",
        "mất",
        "mat",
    ];
    for (role, labels) in [
        (DateRole::Lifespan, lifespan.as_slice()),
        (DateRole::Birth, birth.as_slice()),
        (DateRole::Death, death.as_slice()),
    ] {
        for label in labels {
            if let Some(rest) = labelled_rest(clause, label) {
                if label_is_a_qualifier(rest) {
                    continue;
                }
                return Some((role, rest));
            }
        }
    }
    None
}

/// `birth order` and `birth place` are not dates.
fn label_is_a_qualifier(rest: &str) -> bool {
    let lower = rest.to_lowercase();
    lower.starts_with("order")
        || lower.starts_with("place")
        || lower.starts_with("year")
        || lower.starts_with("thứ")
        || lower.starts_with("noi sinh")
        || lower.starts_with("nơi sinh")
}

/// Returns the value after a label, which must end at `:` or whitespace.
fn labelled_rest<'a>(clause: &'a str, label: &str) -> Option<&'a str> {
    let rest = strip_prefix_ci(clause, label)?;
    if let Some(after) = rest.trim_start().strip_prefix(':') {
        return Some(after.trim());
    }
    if rest.starts_with([' ', '\t']) {
        return Some(rest.trim_start());
    }
    None
}

fn push_date_role(role: DateRole, text: &str) -> Vec<RawDate> {
    match dates::parse_date_expression(text) {
        DateParse::Date(value) => match role {
            // A printed lifespan is a pair: the first year is the birth and the
            // second is the death. A single value has no such meaning.
            DateRole::Lifespan if value.precision == DatePrecision::Range => {
                let end = value.end.clone().unwrap_or_default();
                vec![
                    RawDate::dated(AssertionKind::BirthDate, endpoint(&value, &value.start)),
                    RawDate::dated(AssertionKind::DeathDate, endpoint(&value, &end)),
                ]
            }
            DateRole::Lifespan => vec![RawDate::unresolved(value.raw)],
            DateRole::Birth => vec![RawDate::dated(AssertionKind::BirthDate, value)],
            DateRole::Death => vec![RawDate::dated(AssertionKind::DeathDate, value)],
        },
        DateParse::Ambiguous { raw, .. } => vec![RawDate::unresolved(raw)],
        DateParse::None => Vec::new(),
    }
}

/// One endpoint of a printed lifespan, keeping the range's provenance.
fn endpoint(value: &DateValue, iso: &str) -> DateValue {
    DateValue {
        start: iso.to_string(),
        end: None,
        precision: DatePrecision::of_iso(iso),
        qualifier: value.qualifier,
        raw: value.raw.clone(),
    }
}

/// Returns the text after `keyword` when the keyword opens the clause. An ordinal
/// prefix ("third child of …") is allowed; any other prefix means the clause is
/// about something else.
fn leading_rest<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let lower = text.to_ascii_lowercase();
    let idx = lower.find(keyword)?;
    let prefix = lower[..idx].trim();
    if !prefix.is_empty()
        && !prefix.split_whitespace().all(|word| {
            matches!(
                word,
                "first"
                    | "second"
                    | "third"
                    | "fourth"
                    | "fifth"
                    | "sixth"
                    | "seventh"
                    | "eighth"
                    | "ninth"
                    | "tenth"
                    | "eleventh"
                    | "twelfth"
                    | "eldest"
                    | "youngest"
                    | "adopted"
                    | "only"
                    | "firstborn"
                    | "first-born"
                    | "probable"
                    | "likely"
                    | "possibly"
                    | "presumed"
                    | "assumed"
                    | "the"
            )
        })
    {
        return None;
    }
    Some(
        text[idx + keyword.len()..]
            .trim_start_matches([':', ' '])
            .trim(),
    )
}

/// The text after a clause that starts with `Spouse`, however the catalog writes
/// the separator (`Spouse:`, `Spouse of`, `Spouse-column match to`).
fn spouse_rest(text: &str) -> Option<&str> {
    let trimmed = text.trim_start();
    let after = strip_prefix_ci(trimmed, "spouse")?;
    let mut rest = after.trim_start();
    for prefix in [
        "-column match to",
        "-column match",
        "-column",
        "column match to",
        "column match",
        "column",
        "of",
        "to",
        ":",
        "-",
    ] {
        if let Some(next) = strip_prefix_ci(rest, prefix) {
            rest = next.trim_start();
        }
    }
    Some(rest)
}

fn strip_prefix_ci<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    // A non-ASCII leading character means `prefix.len()` can fall inside a
    // character; `get` returns `None` instead of panicking.
    let head = text.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&text[prefix.len()..])
    } else {
        None
    }
}

/// Splits `A and B` parent lists. `and` is ASCII, so the positions stay valid
/// for slicing the original text.
fn split_parents(rest: &str) -> Vec<Target> {
    let lower = rest.to_ascii_lowercase();
    let mut parts = Vec::new();
    let mut start = 0usize;
    while let Some(idx) = lower[start..].find(" and ") {
        parts.push(&rest[start..start + idx]);
        start += idx + " and ".len();
    }
    parts.push(&rest[start..]);
    parts.into_iter().filter_map(clean_parent).collect()
}

/// Cleans one relationship target. A target the catalog cannot identify stays as
/// an unresolved assertion rather than being dropped or guessed.
fn clean_parent(raw: &str) -> Option<Target> {
    let text = strip_annotations(raw);
    let text = text.trim().trim_matches(|c: char| c == ',' || c == '.');
    if text.is_empty() {
        return None;
    }
    let lower = text.to_ascii_lowercase();
    if lower.starts_with("unnamed") || lower.contains("chưa rõ") || lower.contains("chua ro") {
        return Some(Target {
            text: text.to_string(),
            unresolved: true,
        });
    }
    let mut name = text;
    for qualifier in [
        "second wife",
        "third wife",
        "fourth wife",
        "first wife",
        "wife",
        "husband",
        "spouse",
    ] {
        if let Some(rest) = strip_prefix_ci(name, qualifier) {
            name = rest.trim_start();
        }
    }
    let name = name.trim_matches(|c: char| c == ',' || c == '.').trim();
    if name.is_empty() {
        None
    } else {
        Some(Target {
            text: name.to_string(),
            unresolved: false,
        })
    }
}

/// Removes bracketed annotations (`[p. 7]`, `(1845–1929)`) from a clause.
fn strip_annotations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut square = 0usize;
    let mut paren = 0usize;
    for ch in text.chars() {
        match ch {
            '[' => square += 1,
            ']' => square = square.saturating_sub(1),
            '(' => paren += 1,
            ')' => paren = paren.saturating_sub(1),
            _ if square == 0 && paren == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Dry-run report
// ---------------------------------------------------------------------------

/// A dry-run extraction report. The five counts partition the processed
/// entries: each entry is in exactly one of parsed, ambiguous, or rejected, and
/// `unmatched` is a cross-cutting count of entries with unresolved locators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DryRunReport {
    /// Total entries processed.
    pub processed: usize,
    /// Entries classified as parsed.
    pub parsed: usize,
    /// Entries classified as ambiguous.
    pub ambiguous: usize,
    /// Entries classified as rejected.
    pub rejected: usize,
    /// Birth-date candidates extracted.
    pub birth_dates: usize,
    /// Death-date candidates extracted.
    pub death_dates: usize,
    /// Date statements kept unresolved for review.
    pub unresolved_dates: usize,
    /// Entries with at least one unresolved locator.
    pub unmatched: usize,
    /// Entry keys with unresolved locators.
    pub unmatched_keys: Vec<String>,
    /// Entry keys classified as ambiguous.
    pub ambiguous_keys: Vec<String>,
    /// Rejected entry keys with reasons.
    pub rejected_keys: Vec<(String, String)>,
}

impl DryRunReport {
    /// Builds a report from a parse result.
    pub fn from_parse(parse: &CatalogParse) -> Self {
        DryRunReport {
            processed: parse.entries.len(),
            parsed: parse.parsed_count(),
            ambiguous: parse.ambiguous_count(),
            rejected: parse.rejected_count(),
            birth_dates: count_kind(parse, AssertionKind::BirthDate),
            death_dates: count_kind(parse, AssertionKind::DeathDate),
            unresolved_dates: count_kind(parse, AssertionKind::DateUnresolved),
            unmatched: parse.unmatched_count(),
            unmatched_keys: parse
                .entries
                .iter()
                .filter(|e| {
                    e.assertions.iter().any(|a| {
                        a.evidence
                            .iter()
                            .any(|ev| matches!(ev, Evidence::Unresolved(_)))
                    })
                })
                .map(|e| e.key.clone())
                .collect(),
            ambiguous_keys: parse
                .entries
                .iter()
                .filter(|e| e.outcome == ParseOutcome::Ambiguous)
                .map(|e| e.key.clone())
                .collect(),
            rejected_keys: parse
                .entries
                .iter()
                .filter(|e| e.outcome == ParseOutcome::Rejected)
                .map(|e| (e.key.clone(), format_reason(e.reason.clone())))
                .collect(),
        }
    }
}

fn format_reason(reason: Option<ReasonCode>) -> String {
    match reason {
        Some(ReasonCode::MissingName) => "missing_name".to_string(),
        Some(ReasonCode::PageOutOfRange) => "page_out_of_range".to_string(),
        Some(ReasonCode::ConflictingName) => "conflicting_name".to_string(),
        Some(ReasonCode::SameName) => "same_name".to_string(),
        Some(ReasonCode::UnclearParentage) => "unclear_parentage".to_string(),
        Some(ReasonCode::UnsourcedRelationship) => "unsourced_relationship".to_string(),
        Some(ReasonCode::AmbiguousDate) => "ambiguous_date".to_string(),
        None => "none".to_string(),
    }
}

/// Counts the assertions of one kind across every entry.
fn count_kind(parse: &CatalogParse, kind: AssertionKind) -> usize {
    parse
        .entries
        .iter()
        .flat_map(|entry| entry.assertions.iter())
        .filter(|assertion| assertion.kind == kind)
        .count()
}

/// Renders a dry-run report as Markdown.
pub fn render_report(report: &DryRunReport, catalog_sha256: Option<&str>) -> String {
    let mut out = String::new();
    out.push_str("# TRAN-002C catalog dry-run extraction\n\n");
    out.push_str("This report is private and is never committed.\n\n");
    out.push_str(&format!("- processed: {}\n", report.processed));
    out.push_str(&format!("- parsed: {}\n", report.parsed));
    out.push_str(&format!("- ambiguous: {}\n", report.ambiguous));
    out.push_str(&format!("- rejected: {}\n", report.rejected));
    out.push_str(&format!("- unmatched: {}\n", report.unmatched));
    out.push_str(&format!("- birth_dates: {}\n", report.birth_dates));
    out.push_str(&format!("- death_dates: {}\n", report.death_dates));
    out.push_str(&format!(
        "- unresolved_dates: {}\n",
        report.unresolved_dates
    ));
    if let Some(hash) = catalog_sha256 {
        out.push_str(&format!("- catalog_sha256: {hash}\n"));
    }
    out.push_str("\n## Ambiguous entries\n\n");
    if report.ambiguous_keys.is_empty() {
        out.push_str("None.\n");
    } else {
        for key in &report.ambiguous_keys {
            out.push_str(&format!("- {key}\n"));
        }
    }
    out.push_str("\n## Rejected entries\n\n");
    if report.rejected_keys.is_empty() {
        out.push_str("None.\n");
    } else {
        for (key, reason) in &report.rejected_keys {
            out.push_str(&format!("- {key}: {reason}\n"));
        }
    }
    out.push_str("\n## Unmatched entries\n\n");
    if report.unmatched_keys.is_empty() {
        out.push_str("None.\n");
    } else {
        for key in &report.unmatched_keys {
            out.push_str(&format!("- {key}\n"));
        }
    }
    out
}

/// Runs a dry-run parse over the catalog at `input_path` and writes the report
/// to `output_path`. This function reads only text files and writes only
/// Markdown. It never opens the genealogy database.
pub fn run_dry_run(
    input_path: &std::path::Path,
    output_path: &std::path::Path,
) -> std::io::Result<DryRunReport> {
    let text = std::fs::read_to_string(input_path)?;
    let parse = parse_catalog(&text);
    let report = DryRunReport::from_parse(&parse);
    let rendered = render_report(&report, None);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_path, rendered)?;
    Ok(report)
}

#[cfg(test)]
#[path = "parser_tests.rs"]
mod tests;
