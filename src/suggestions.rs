//! Deterministic question suggestions drawn from approved archive passages.
//!
//! A suggestion can only name content inside the publication boundary: the caller
//! reads sections from `approved_documents`, the same view retrieval uses, so an
//! unapproved, restricted, or withdrawn passage can never appear. Structural
//! labels describe the document's layout rather than its content, so page
//! markers, tables of contents, and editorial notes are never used. When a
//! section has no usable heading, the topic is copied from the passage itself,
//! which keeps the quoted phrase searchable.

/// The number of suggestions the Ask page requests.
pub const SUGGESTION_LIMIT: usize = 6;

/// How many approved sections the read considers. Each section contributes at
/// most one topic, so this bounds the read without bounding the result: a corpus
/// with more sections than this still yields [`SUGGESTION_LIMIT`] suggestions as
/// long as any scanned section carries a usable topic.
pub const SUGGESTION_SCAN: usize = SUGGESTION_LIMIT * 8;

/// Longest proper-noun phrase accepted as a topic, in words.
const MAX_NAME_WORDS: usize = 5;

/// File suffixes that mark a section title as a filename, not a topic.
const FILE_SUFFIXES: [&str; 8] = [
    ".md",
    ".markdown",
    ".txt",
    ".pdf",
    ".docx",
    ".csv",
    ".json",
    ".rtf",
];

/// Title prefixes that are not part of a name.
const HONORIFICS: [&str; 7] = ["mr", "mrs", "ms", "miss", "dr", "sir", "lady"];

/// Closed-class words that cannot start or end a topic phrase.
const EDGE_WORDS: [&str; 46] = [
    "a",
    "an",
    "the",
    "this",
    "that",
    "these",
    "those",
    "and",
    "or",
    "but",
    "if",
    "of",
    "in",
    "on",
    "at",
    "to",
    "for",
    "from",
    "with",
    "by",
    "as",
    "is",
    "was",
    "were",
    "be",
    "been",
    "has",
    "have",
    "had",
    "also",
    "known",
    "note",
    "source",
    "person",
    "relationship",
    "she",
    "he",
    "they",
    "it",
    "his",
    "her",
    "their",
    "its",
    "we",
    "you",
    "i",
];

/// Words that describe the document's layout, never its content.
const STRUCTURAL_WORDS: [&str; 6] = ["pdf", "page", "contents", "editorial", "markdown", "draft"];

/// One approved section considered for a suggestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SuggestionSection {
    /// Section heading stored with the approved chunk; may be empty.
    pub section: String,
    /// Body of the section's first approved chunk.
    pub content: String,
}

/// Builds the question labels shown on the Ask page.
///
/// Sections are considered in the order the caller supplies them. A section that
/// carries no usable topic is skipped, and a topic that repeats or contains an
/// earlier one is dropped, so the page never shows two versions of the same
/// question.
pub fn questions(sections: &[SuggestionSection], limit: usize) -> Vec<String> {
    let mut topics: Vec<String> = Vec::new();
    for section in sections {
        if topics.len() >= limit {
            break;
        }
        let Some(topic) = topic(&section.section, &section.content) else {
            continue;
        };
        if topics.iter().any(|existing| related(existing, &topic)) {
            continue;
        }
        topics.push(topic);
    }
    topics
        .into_iter()
        .map(|topic| format!("What does the archive say about \"{topic}\"?"))
        .collect()
}

/// Chooses the topic for one approved section.
///
/// Returns `None` when the section carries no topic: an excluded structural
/// label, an empty body, or a body without a usable phrase.
pub fn topic(section: &str, content: &str) -> Option<String> {
    let heading = section.trim();
    if excluded_heading(heading) {
        return None;
    }
    if !heading.is_empty() && !structural_heading(heading) {
        return Some(heading.to_string());
    }
    content_topic(content)
}

/// True for the headings that are excluded from suggestions outright: they
/// describe the document's layout or its editorial rules, not its content.
fn excluded_heading(heading: &str) -> bool {
    let lower = heading.trim().to_lowercase();
    let lower = lower.trim_end_matches([':', '.']).trim();
    lower == "contents" || lower == "table of contents" || lower.starts_with("editorial")
}

/// True for a heading that must not be quoted as a topic: a page marker, a
/// filename, or a label without words.
fn structural_heading(heading: &str) -> bool {
    page_marker(heading)
        || FILE_SUFFIXES
            .iter()
            .any(|suffix| heading.to_lowercase().ends_with(suffix))
        || !heading.chars().any(char::is_alphabetic)
}

/// True for headings such as `PDF page 12`, `page 12`, `p. 12`, or `PDF 12-13`.
fn page_marker(heading: &str) -> bool {
    let mut marker = false;
    for token in heading.split(|c: char| !c.is_alphanumeric()) {
        if token.is_empty() || token.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        match token.to_lowercase().as_str() {
            "pdf" | "page" | "pages" | "p" | "pp" => marker = true,
            _ => return false,
        }
    }
    marker
}

/// Derives a topic from the passage when the heading cannot be quoted.
///
/// Only a capitalised phrase qualifies. A sentence fragment does not read as a
/// topic, and quoting one would turn prose into a misleading question, so a
/// passage without a name or place contributes nothing.
fn content_topic(content: &str) -> Option<String> {
    proper_noun_phrase(&strip_markup(content))
}

/// Replaces markup punctuation with spaces and collapses whitespace, so a phrase
/// copied from the result matches the passage text. Bracketed spans are editorial
/// notes about the document ("[Cover illustration: ...]") rather than passages,
/// so they never supply a topic.
fn strip_markup(content: &str) -> String {
    let mut kept = String::with_capacity(content.len());
    let mut depth = 0usize;
    for ch in content.chars() {
        match ch {
            '[' => depth += 1,
            ']' if depth > 0 => depth -= 1,
            ']' => kept.push(' '),
            _ if depth > 0 => {}
            '#' | '*' | '_' | '`' | '>' | '|' => kept.push(' '),
            other => kept.push(other),
        }
    }
    kept.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Finds the first run of two or more capitalised words, such as a person or a
/// place, and trims it to a quoted topic.
fn proper_noun_phrase(text: &str) -> Option<String> {
    let mut found: Option<String> = None;
    let mut run: Vec<&str> = Vec::new();
    for raw in text.split_whitespace() {
        if name_word(raw) {
            run.push(raw);
            continue;
        }
        if found.is_none() {
            found = accepted_run(&run);
        }
        run.clear();
    }
    found.or_else(|| accepted_run(&run))
}

/// True for a capitalised word that is not an acronym: `Trần` yes, `PDF` no.
fn name_word(raw: &str) -> bool {
    let word = trim_word(raw);
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_uppercase()
        && word.chars().any(char::is_alphabetic)
        && !word.chars().all(|c| c.is_uppercase())
}

/// Turns a capitalised run into a topic, dropping title prefixes, trailing
/// function words, and any run that names document structure.
fn accepted_run(run: &[&str]) -> Option<String> {
    let mut words: Vec<String> = run.iter().map(|raw| trim_word(raw)).collect();
    words.retain(|word| !word.is_empty());
    while words
        .first()
        .is_some_and(|word| edge_word(word) || honorific(word))
    {
        words.remove(0);
    }
    while words.last().is_some_and(|word| edge_word(word)) {
        words.pop();
    }
    if words.len() < 2 {
        return None;
    }
    words.truncate(MAX_NAME_WORDS);
    if words.iter().any(|word| structural_word(word)) {
        return None;
    }
    Some(words.join(" "))
}

/// Trims surrounding punctuation from one word.
fn trim_word(raw: &str) -> String {
    raw.trim_matches(|c: char| !c.is_alphanumeric()).to_string()
}

fn edge_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    EDGE_WORDS.contains(&lower.as_str())
}

fn honorific(word: &str) -> bool {
    let lower = word.to_lowercase();
    HONORIFICS.contains(&lower.as_str())
}

fn structural_word(word: &str) -> bool {
    let lower = word.to_lowercase();
    STRUCTURAL_WORDS.contains(&lower.as_str())
}

/// True when two topics name the same thing for display purposes.
fn related(left: &str, right: &str) -> bool {
    let left = left.to_lowercase();
    let right = right.to_lowercase();
    left == right || left.contains(&right) || right.contains(&left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(section: &str, content: &str) -> SuggestionSection {
        SuggestionSection {
            section: section.to_string(),
            content: content.to_string(),
        }
    }

    #[test]
    fn a_substantive_heading_is_the_topic() {
        assert_eq!(
            topic("Archive Test Section", "The synthetic codeword is ZX-1."),
            Some("Archive Test Section".to_string())
        );
    }

    #[test]
    fn page_markers_contents_and_editorial_notes_are_excluded() {
        for heading in [
            "PDF page 12",
            "page 7",
            "p. 3",
            "PDF 41-42",
            "Contents",
            "Table of contents",
            "Editorial method",
            "Editorial note",
        ] {
            assert_eq!(
                topic(heading, ""),
                None,
                "{heading:?} must not become a topic"
            );
        }
    }

    #[test]
    fn a_page_section_falls_back_to_a_phrase_inside_the_passage() {
        let content = "### TRAN FAMILY #### GENEALOGY ### Original Birthplace \
                       Phượng Lâu Village Thanh Cù (Hoài Thanh) County";
        let topic = topic("PDF page 5", content).expect("a content phrase");
        assert_eq!(topic, "Original Birthplace Phượng Lâu Village");
        assert!(
            content.contains(&topic),
            "the quoted phrase must appear verbatim in the passage"
        );
    }

    #[test]
    fn honorifics_and_function_words_are_trimmed() {
        let content = "### Generation 11: TRẦN TỐI LẠC (1845–1916) Mr Trần Tối Lạc was the \
                       fourth child of Mr. Trần Huy Nghìn";
        assert_eq!(
            topic("PDF page 6", content),
            Some("Trần Tối Lạc".to_string())
        );
    }

    #[test]
    fn a_passage_without_a_name_contributes_nothing() {
        let content = "This is a moral reminder to appreciate and make good use of what we have.";
        assert_eq!(topic("PDF page 12", content), None);
    }

    #[test]
    fn filenames_are_not_topics() {
        assert_eq!(
            topic(
                "tran-document-41310-1792882440861114000.txt",
                "Person: Ethan Minh Tran"
            ),
            Some("Ethan Minh Tran".to_string())
        );
    }

    #[test]
    fn document_furniture_is_not_a_topic() {
        let content = "### GENEALOGY #### SUMMARY OF FAMILIES \
                       [Cover illustration: a large tree in a landscape]";
        assert_eq!(topic("PDF page 1", content), None);
    }

    #[test]
    fn bracketed_editorial_notes_never_supply_a_topic() {
        let content = "[Section title page with ornamental borders.] Mr Trần Tối Lạc was the \
                       fourth child.";
        assert_eq!(
            topic("PDF page 6", content),
            Some("Trần Tối Lạc".to_string())
        );
    }

    #[test]
    fn questions_are_derived_in_order_and_deduplicated() {
        let sections = [
            section("Migration", "Migration evidence"),
            section(
                "PDF page 2",
                "Family Genealogy This genealogy book was translated.",
            ),
            section("Migration notes", "Duplicate topic"),
            section("Contents", "PDF 1-5: introduction"),
        ];
        assert_eq!(
            questions(&sections, 6),
            vec![
                "What does the archive say about \"Migration\"?".to_string(),
                "What does the archive say about \"Family Genealogy\"?".to_string(),
            ]
        );
    }

    #[test]
    fn empty_sections_and_a_zero_limit_produce_nothing() {
        assert!(questions(&[], 6).is_empty());
        assert!(questions(&[section("", "")], 6).is_empty());
        assert!(questions(&[section("Migration", "body")], 0).is_empty());
    }

    #[test]
    fn the_limit_counts_usable_topics_not_scanned_sections() {
        let sections = [
            section("PDF page 1", ""),
            section("Contents", "PDF 1-5: introduction"),
            section("Migration", "Migration evidence"),
        ];
        assert_eq!(questions(&sections, 1).len(), 1);
    }
}
