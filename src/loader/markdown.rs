//! Markdown loader.
//!
//! Ported from the Go baseline Markdown loader. Markdown is read as UTF-8 text
//! and split with the shared `## ` section parser; every section is page-less.

use crate::document::Section;
use crate::section::parse_sections;

/// Loads Markdown `content` into page-less sections using the baseline rule.
pub fn load_markdown(content: &str) -> Vec<Section> {
    parse_sections(content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markdown_sections_are_page_less() {
        let sections = load_markdown("## A\nbody\n## B\nmore");
        assert_eq!(sections.len(), 2);
        assert!(sections.iter().all(|s| s.page.is_none()));
    }

    #[test]
    fn empty_markdown_has_no_sections() {
        assert!(load_markdown("").is_empty());
    }

    #[test]
    fn vietnamese_markdown_round_trips() {
        let sections = load_markdown("## Tiêu đề\nNội dung tiếng Việt");
        assert_eq!(sections[0].title, "Tiêu đề");
        assert_eq!(sections[0].content, "Nội dung tiếng Việt");
    }
}
