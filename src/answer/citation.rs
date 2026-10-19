//! Citation parsing, validation, and repair.
//!
//! Ported from the Go baseline (`internal/citations`). Citations use the form
//! `[source - section]` or `[source - section - p.N]`; parsing is lexical, and
//! repair strips only citations that do not resolve to the retrieved passages.

use super::context_budget::ContextPassage;

/// A parsed `[source - section]` citation with an optional page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Citation {
    /// Source document name.
    pub source: String,
    /// Section title.
    pub section: String,
    /// Optional 1-based page.
    pub page: Option<u32>,
}

/// A citation together with its byte span in the parsed answer.
struct Span {
    citation: Citation,
    start: usize,
    end: usize,
}

/// Normalizes en/em dashes so a citation can be split on a single separator.
fn normalize(citation: &str) -> String {
    citation.replace(['\u{2013}', '\u{2014}'], "-")
}

/// Normalizes a source for comparison: trim, lowercase, and drop a trailing
/// `.md` suffix, matching the Go baseline.
fn normalize_source(source: &str) -> String {
    let trimmed = source.trim().to_lowercase();
    trimmed.strip_suffix(".md").unwrap_or(&trimmed).to_string()
}

fn parse_page_segment(segment: &str) -> Option<u32> {
    let digits = segment.trim().strip_prefix("p.")?;
    let page = digits.trim().parse::<u32>().ok()?;
    (page > 0).then_some(page)
}

fn parse_inner(raw: &str) -> Option<Citation> {
    // Do not trim the bracket's inner text before splitting: the Go baseline
    // cuts on " - " in the raw inner text, so a trailing separator such as
    // "[Source: doc.md - ]" is still parsed (with an empty section) and then
    // rejected as invalid. Trimming first would hide it from the parser.
    let inner = raw.strip_prefix('[')?.strip_suffix(']')?;
    let normalized = normalize(inner);
    let (source, rest) = normalized.split_once(" - ")?;
    let mut citation = Citation {
        source: source.trim().to_string(),
        section: rest.trim().to_string(),
        page: None,
    };
    if let Some((section, page_segment)) = rest.split_once(" - ") {
        if let Some(page) = parse_page_segment(page_segment) {
            citation.section = section.trim().to_string();
            citation.page = Some(page);
        }
    }
    Some(citation)
}

fn spans(answer: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let bytes = answer.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'[' {
            if let Some(close) = answer[i..].find(']') {
                let end = i + close + 1;
                if let Some(citation) = parse_inner(&answer[i..end]) {
                    spans.push(Span {
                        citation,
                        start: i,
                        end,
                    });
                }
                i = end;
                continue;
            }
        }
        i += answer[i..].chars().next().map(char::len_utf8).unwrap_or(1);
    }
    spans
}

/// Parses all citations from `answer`, in order.
pub fn parse(answer: &str) -> Vec<Citation> {
    spans(answer)
        .into_iter()
        .map(|span| span.citation)
        .collect()
}

/// Reports whether `citation` resolves to `passage`: normalized source, exact
/// section, and, when the citation claims a page, the same page.
pub fn matches(citation: &Citation, passage: &ContextPassage) -> bool {
    if normalize_source(&passage.source) != normalize_source(&citation.source) {
        return false;
    }
    if passage.section != citation.section {
        return false;
    }
    match citation.page {
        None => true,
        Some(page) => passage.page == Some(page),
    }
}

/// Returns true when `citation` matches any retrieved passage.
pub fn is_valid(citation: &Citation, passages: &[ContextPassage]) -> bool {
    passages.iter().any(|passage| matches(citation, passage))
}

/// Counts the citations in `answer` and how many of them resolve to a
/// retrieved passage, in the same terms as the Go baseline's `Validity`.
pub fn validity(answer: &str, passages: &[ContextPassage]) -> (usize, usize) {
    let citations = parse(answer);
    let total = citations.len();
    let valid = citations
        .iter()
        .filter(|citation| is_valid(citation, passages))
        .count();
    (valid, total)
}

/// Reports whether `answer` needs no repair: it makes at least one citation
/// and every citation resolves to a retrieved passage.
///
/// An answer that cites nothing is not clean, matching the Go baseline: a
/// missing citation is treated like an invalid one and takes the single bounded
/// repair path.
pub fn is_clean(answer: &str, passages: &[ContextPassage]) -> bool {
    let (valid, total) = validity(answer, passages);
    total > 0 && valid == total
}

fn needs_separator(prefix: &str, suffix: &str) -> bool {
    if prefix.is_empty() || suffix.is_empty() {
        return false;
    }
    let last = prefix.chars().last().unwrap();
    let first = suffix.chars().next().unwrap();
    last.is_alphanumeric() && first.is_alphanumeric()
}

/// Removes every citation that does not resolve to a retrieved passage,
/// preserving valid citations and all prose. A separating space is inserted
/// when removal would fuse two words.
pub fn repair(answer: &str, passages: &[ContextPassage]) -> String {
    let spans = spans(answer);
    if spans.is_empty() {
        return answer.to_string();
    }
    let mut out = String::new();
    let mut cursor = 0;
    for span in spans {
        if is_valid(&span.citation, passages) {
            continue;
        }
        out.push_str(&answer[cursor..span.start]);
        if needs_separator(&out, &answer[span.end..]) {
            out.push(' ');
        }
        cursor = span.end;
    }
    out.push_str(&answer[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(source: &str, section: &str, page: Option<u32>) -> ContextPassage {
        ContextPassage {
            source: source.to_string(),
            section: section.to_string(),
            page,
            content: "body".to_string(),
        }
    }

    #[test]
    fn parses_source_section() {
        let parsed = parse("See [doc.md - Payments].");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].source, "doc.md");
        assert_eq!(parsed[0].section, "Payments");
        assert_eq!(parsed[0].page, None);
    }

    #[test]
    fn parses_source_section_page() {
        let parsed = parse("[doc.md - Payments - p.2]");
        assert_eq!(parsed[0].section, "Payments");
        assert_eq!(parsed[0].page, Some(2));
    }

    #[test]
    fn section_with_dash_is_preserved() {
        let parsed = parse("[doc.md - Late Payments - Grace Period]");
        assert_eq!(parsed[0].section, "Late Payments - Grace Period");
        assert_eq!(parsed[0].page, None);
    }

    #[test]
    fn validates_source_section_and_page() {
        let passages = vec![passage("doc.md", "Payments", Some(2))];
        assert!(is_valid(&parse("[doc.md - Payments]")[0], &passages));
        assert!(is_valid(&parse("[doc.md - Payments - p.2]")[0], &passages));
        assert!(!is_valid(&parse("[doc.md - Payments - p.9]")[0], &passages));
        assert!(!is_valid(&parse("[other.md - Payments]")[0], &passages));
    }

    #[test]
    fn repair_drops_only_invalid_citations() {
        let passages = vec![passage("doc.md", "Payments", Some(2))];
        let answer = "A [doc.md - Payments] B [doc.md - Other] C";
        assert_eq!(repair(answer, &passages), "A [doc.md - Payments] B  C");
    }

    #[test]
    fn trailing_separator_without_section_is_parsed_then_rejected() {
        let passages = vec![passage("doc.md", "Payments", Some(2))];
        let answer = "Fact [Source: doc.md - ]";
        let parsed = parse(answer);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].source, "Source: doc.md");
        assert_eq!(parsed[0].section, "");
        assert!(!is_valid(&parsed[0], &passages));
        assert_eq!(repair(answer, &passages), "Fact ");
    }

    #[test]
    fn validity_counts_valid_and_total() {
        let passages = vec![passage("doc.md", "Payments", Some(2))];
        assert_eq!(
            validity("A [doc.md - Payments] B [doc.md - Other]", &passages),
            (1, 2)
        );
        assert_eq!(validity("no citations", &passages), (0, 0));
    }

    #[test]
    fn clean_requires_at_least_one_valid_citation() {
        let passages = vec![passage("doc.md", "Payments", Some(2))];
        assert!(is_clean("[doc.md - Payments]", &passages));
        assert!(is_clean("[doc.md - Payments - p.2]", &passages));
        assert!(!is_clean("[doc.md - Other]", &passages));
        assert!(!is_clean("uncited prose", &passages));
    }
}
