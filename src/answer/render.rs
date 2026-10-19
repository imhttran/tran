//! Page/section reference rendering.
//!
//! Renders a passage's citation reference in the Go baseline format:
//! `source - section` plus ` - p.N` when the passage carries a page.

use super::context_budget::ContextPassage;

/// Renders the reference for `passage`.
pub fn render_page(passage: &ContextPassage) -> String {
    match passage.page {
        Some(page) => format!("{} - {} - p.{}", passage.source, passage.section, page),
        None => format!("{} - {}", passage.source, passage.section),
    }
}

/// Renders the reference for each passage, in order.
pub fn render_pages(passages: &[ContextPassage]) -> Vec<String> {
    passages.iter().map(render_page).collect()
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
    fn renders_source_section_and_page() {
        assert_eq!(
            render_page(&passage("doc.md", "Payments", Some(4))),
            "doc.md - Payments - p.4"
        );
    }

    #[test]
    fn renders_source_and_section_without_page() {
        assert_eq!(
            render_page(&passage("doc.md", "Payments", None)),
            "doc.md - Payments"
        );
    }

    #[test]
    fn renders_each_passage() {
        let out = render_pages(&[passage("a.md", "A", Some(1)), passage("b.md", "B", None)]);
        assert_eq!(out, vec!["a.md - A - p.1", "b.md - B"]);
    }
}
