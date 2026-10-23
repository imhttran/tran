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

/// Normalizes a source for comparison: trim, lowercase, drop a trailing
/// `.md` suffix, and drop the lifecycle version suffix used to keep two
/// versions of the same document distinct in storage.
///
/// The model may cite the human-readable filename without the internal
/// `(shortkey) (vN)` suffix, so both the passage source and the citation source
/// are normalized before comparison.
fn normalize_source(source: &str) -> String {
    let trimmed = source.trim().to_lowercase();
    let mut base = trimmed.as_str();
    // Drop a trailing " (vN)" only when it follows a parenthesised document
    // key, which is the only form the lifecycle generates. A real filename that
    // happens to end in " (v2)" is left intact.
    if base.ends_with(')') {
        if let Some(index) = base.rfind(" (v") {
            let inner = &base[index + 3..base.len() - 1];
            if !inner.is_empty()
                && inner.chars().all(|ch| ch.is_ascii_digit())
                && base[..index].trim_end().ends_with(')')
            {
                base = base[..index].trim_end();
            }
        }
    }
    // Drop a trailing document key: the generated " (8 hex)", the submission
    // " (slug-8 hex)", or the operator " (slug-with-dash)" form. A plain
    // single word in parentheses is not a generated key.
    if base.ends_with(')') {
        if let Some(index) = base.rfind(" (") {
            let inner = &base[index + 2..base.len() - 1];
            let hex_tail = inner.len() >= 8
                && inner[inner.len() - 8..]
                    .chars()
                    .all(|ch| ch.is_ascii_hexdigit());
            let plain_key = inner.len() == 8 && hex_tail;
            let prefixed_key =
                inner.len() > 8 && inner.as_bytes()[inner.len() - 9] == b'-' && hex_tail;
            let operator_key = inner.contains('-')
                && inner
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
            if plain_key || prefixed_key || operator_key {
                base = base[..index].trim_end();
            }
        }
    }
    base.strip_suffix(".md").unwrap_or(base).to_string()
}

fn normalize_section(section: &str) -> String {
    let trimmed = section.trim();
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("section:") {
        trimmed[8..].trim().to_string()
    } else {
        trimmed.to_string()
    }
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

/// Resolves `citation` to the single passage it names.
///
/// Resolution is exact-source first. Only when no passage carries the exact
/// stored source does it fall back to normalized filename matching.
///
/// A citation names a *publication*, identified by what the prompt exposes:
/// source, section, and page. Several chunks of the same source, section, and
/// page are therefore one citeable publication, not an ambiguity; the first
/// matching passage represents it. Passages that differ in exact source,
/// section, or page are different publications, and the citation is ambiguous
/// and resolves to `None`. This keeps two documents with the same filename from
/// being confused and makes an ambiguous model citation fail closed instead of
/// being attributed to the first passage.
pub fn resolve(citation: &Citation, passages: &[ContextPassage]) -> Option<usize> {
    let section_matches = |passage: &ContextPassage| {
        normalize_section(&passage.section) == normalize_section(&citation.section)
    };
    let page_matches = |passage: &ContextPassage| match citation.page {
        None => true,
        Some(page) => passage.page == Some(page),
    };

    let exact: Vec<usize> = passages
        .iter()
        .enumerate()
        .filter(|(_, passage)| {
            passage.source == citation.source && section_matches(passage) && page_matches(passage)
        })
        .map(|(index, _)| index)
        .collect();
    if !exact.is_empty() {
        return one_publication(&exact, passages);
    }

    let normalized: Vec<usize> = passages
        .iter()
        .enumerate()
        .filter(|(_, passage)| {
            normalize_source(&passage.source) == normalize_source(&citation.source)
                && section_matches(passage)
                && page_matches(passage)
        })
        .map(|(index, _)| index)
        .collect();
    one_publication(&normalized, passages)
}

/// Returns the first matching passage when every match names the same
/// publication (exact source, section, page); `None` when matches name more
/// than one publication.
fn one_publication(matches: &[usize], passages: &[ContextPassage]) -> Option<usize> {
    let first = *matches.first()?;
    let reference = &passages[first];
    let same_publication = matches.iter().all(|index| {
        let passage = &passages[*index];
        passage.source == reference.source
            && normalize_section(&passage.section) == normalize_section(&reference.section)
            && passage.page == reference.page
    });
    if same_publication {
        Some(first)
    } else {
        None
    }
}

/// Reports whether `citation` resolves to exactly one passage.
pub fn matches(citation: &Citation, passage: &ContextPassage) -> bool {
    resolve(citation, std::slice::from_ref(passage)) == Some(0)
}

/// Returns true when `citation` resolves to any retrieved passage.
pub fn is_valid(citation: &Citation, passages: &[ContextPassage]) -> bool {
    resolve(citation, passages).is_some()
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
            identity: crate::answer::context_budget::PassageIdentity::default(),
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

    #[test]
    fn same_filename_documents_resolve_by_exact_source() {
        let passages = vec![
            passage("report.md (report-md-aaaa1111)", "Findings", None),
            passage("report.md (report-md-bbbb2222)", "Findings", None),
        ];
        let cited = parse("[report.md (report-md-bbbb2222) - Findings]");
        assert_eq!(resolve(&cited[0], &passages), Some(1));
        let other = parse("[report.md (report-md-aaaa1111) - Findings]");
        assert_eq!(resolve(&other[0], &passages), Some(0));
    }

    #[test]
    fn ambiguous_normalized_citation_is_rejected() {
        let passages = vec![
            passage("report.md (report-md-aaaa1111)", "Findings", None),
            passage("report.md (report-md-bbbb2222)", "Findings", None),
        ];
        // The human-readable filename alone names two different documents.
        let cited = parse("[report.md - Findings]");
        assert_eq!(resolve(&cited[0], &passages), None);
        assert!(!is_valid(&cited[0], &passages));
        assert_eq!(repair("[report.md - Findings]", &passages), "");
    }

    #[test]
    fn unique_human_filename_still_resolves() {
        let passages = vec![
            passage("report.md (report-md-aaaa1111)", "Findings", None),
            passage("notes.md (notes-md-cccc3333)", "Findings", None),
        ];
        let cited = parse("[report.md - Findings]");
        assert!(is_valid(&cited[0], &passages));
        assert_eq!(resolve(&cited[0], &passages), Some(0));
    }

    #[test]
    fn exact_source_is_not_overridden_by_a_normalized_match() {
        let passages = vec![
            passage("report.md (report-md-aaaa1111)", "Findings", None),
            passage("report.md (report-md-bbbb2222)", "Findings", None),
        ];
        let cited = parse("[report.md - Findings]");
        // Ambiguity is not "pick the first": it is invalid.
        assert!(!is_valid(&cited[0], &passages));
    }

    #[test]
    fn several_chunks_of_one_section_are_one_citeable_publication() {
        // Section expansion routinely places several chunks of the same
        // source, section, and page into the context. The model cannot cite a
        // chunk, so this is one publication, not an ambiguity.
        let passages = vec![
            passage("doc.md", "Payments", Some(2)),
            passage("doc.md", "Payments", Some(2)),
            passage("doc.md", "Payments", Some(2)),
        ];
        let cited = parse("[doc.md - Payments - p.2]");
        assert_eq!(resolve(&cited[0], &passages), Some(0));
        assert!(is_valid(&cited[0], &passages));
        assert!(is_clean("[doc.md - Payments - p.2]", &passages));
        assert_eq!(
            repair("[doc.md - Payments - p.2]", &passages),
            "[doc.md - Payments - p.2]"
        );
    }

    #[test]
    fn several_pages_of_one_section_without_a_page_stay_ambiguous() {
        let passages = vec![
            passage("doc.md", "Payments", Some(1)),
            passage("doc.md", "Payments", Some(2)),
        ];
        let cited = parse("[doc.md - Payments]");
        assert_eq!(resolve(&cited[0], &passages), None);
        assert!(!is_valid(&cited[0], &passages));
    }
}
