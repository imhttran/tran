//! Section parsing: split raw document text into `## ` sections.
//!
//! Ported from the Go baseline section parser
//! (`internal/document/document.go`, `ParseSections`). The baseline rule:
//!
//! * A line begins a new section when it starts with exactly `## ` (level-two
//!   Markdown heading). The heading text after the marker is the section title,
//!   trimmed.
//! * All subsequent lines up to (but excluding) the next heading form the
//!   section content, trimmed of leading and trailing whitespace.
//! * Text before the first `## ` heading is **ignored** (the baseline splits on
//!   `"\n## "` and drops the block before the first separator).
//! * Empty input yields no sections.
//! * Every produced section has `page: None`; page provenance is assigned only
//!   by the PDF loader.

use crate::document::Section;

/// Splits `text` into `## ` sections using the baseline rule.
///
/// The returned sections preserve document order and each has `page: None`.
/// Any text before the first `## ` heading is ignored, matching the Go
/// reference.
///
/// # Examples
///
/// ```
/// use rag_foundation::section::parse_sections;
///
/// let sections = parse_sections("## Intro\nhello");
/// assert_eq!(sections.len(), 1);
/// assert_eq!(sections[0].title, "Intro");
/// assert_eq!(sections[0].content, "hello");
/// ```
pub fn parse_sections(text: &str) -> Vec<Section> {
    if text.trim().is_empty() {
        return Vec::new();
    }

    let mut sections: Vec<Section> = Vec::new();
    // `None` until the first heading. Lines seen before it are ignored, so a
    // leading pre-heading block is dropped exactly as in the baseline.
    let mut current: Option<(String, Vec<&str>)> = None;

    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            if let Some((title, body)) = current.take() {
                sections.push(Section::new(title.trim(), body.join("\n").trim()));
            }
            current = Some((heading.to_string(), Vec::new()));
        } else if let Some((_, body)) = current.as_mut() {
            body.push(line);
        }
    }

    if let Some((title, body)) = current.take() {
        sections.push(Section::new(title.trim(), body.join("\n").trim()));
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_has_no_sections() {
        assert!(parse_sections("").is_empty());
        assert!(parse_sections("   \n\t\n").is_empty());
    }

    #[test]
    fn single_section_with_trimmed_content() {
        let sections = parse_sections("## Title\n\n  body line  \n\n");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Title");
        assert_eq!(sections[0].content, "body line");
        assert_eq!(sections[0].page, None);
    }

    #[test]
    fn multiple_sections_preserve_order() {
        let sections = parse_sections("## A\none\n## B\ntwo\n## C\nthree");
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, vec!["A", "B", "C"]);
        assert_eq!(sections[1].content, "two");
    }

    #[test]
    fn leading_block_before_first_heading_is_ignored() {
        // Baseline drops everything before the first "\n## ".
        let sections = parse_sections("preamble\n## A\none");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "A");
        assert_eq!(sections[0].content, "one");
    }

    #[test]
    fn non_level_two_headings_are_content() {
        let sections = parse_sections("## A\n# not a section\n### also not");
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].content, "# not a section\n### also not");
    }

    #[test]
    fn unicode_and_vietnamese_round_trip() {
        let text = "## Giới thiệu\nXin chào thế giới — 日本語テスト café";
        let sections = parse_sections(text);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "Giới thiệu");
        assert_eq!(sections[0].content, "Xin chào thế giới — 日本語テスト café");
    }
}
