//! Plain-text loader.
//!
//! Ported from the Go baseline plain-text loader (`internal/loader/loader.go`,
//! `TextLoader`). A non-empty file becomes a single page-less section titled
//! with the file's base name; an empty or whitespace-only file yields no
//! sections. Plain text is **not** split on `## ` headings — the whole file is
//! one section, exactly as in the baseline.

use std::path::Path;

use crate::document::Section;

/// Loads the plain-text file at `path` into a single page-less section.
///
/// The section title is the file's base name and its content is the file's
/// trimmed text. A file whose text is empty (or whitespace-only) yields no
/// sections, preserving the baseline's "no sections found" behavior.
pub fn load_text(path: impl AsRef<Path>) -> std::io::Result<Vec<Section>> {
    let path = path.as_ref();
    let data = std::fs::read_to_string(path)?;
    let content = data.trim();
    if content.is_empty() {
        return Ok(Vec::new());
    }

    let title = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();

    Ok(vec![Section::new(title, content)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_section_titled_by_file_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.txt");
        std::fs::write(&path, "Xin chào\n\nĐây là văn bản thuần túy.\n").unwrap();

        let sections = load_text(&path).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].title, "plain.txt");
        assert_eq!(sections[0].content, "Xin chào\n\nĐây là văn bản thuần túy.");
        assert_eq!(sections[0].page, None);
    }

    #[test]
    fn empty_file_has_no_sections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.txt");
        std::fs::write(&path, "   \n\t\n").unwrap();
        assert!(load_text(&path).unwrap().is_empty());
    }

    #[test]
    fn headings_are_not_section_delimiters_in_plain_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "## A\nbody\n## B\nmore").unwrap();
        let sections = load_text(&path).unwrap();
        assert_eq!(
            sections.len(),
            1,
            "plain text is one section, not ## sections"
        );
        assert_eq!(sections[0].content, "## A\nbody\n## B\nmore");
    }
}
