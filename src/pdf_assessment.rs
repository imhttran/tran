//! TRAN-001B per-page PDF assessment.
//!
//! Builds a per-page assessment of a PDF text layer and records which pages a
//! Vietnamese transcription already covers. The module is read-only: it opens
//! the PDF and the transcription for reading only and never writes a source
//! file. The binary `src/bin/pdf_assessment.rs` writes the local report.
//!
//! Classification rules (B3):
//!
//! * a page in [`PdfExtraction::sections`] has a text layer (`text`);
//! * a page in [`PdfExtraction::incomplete_pages`] needs OCR (`no-text`);
//! * a page in neither set is `uncertain`;
//! * [`PdfError::Scanned`] marks every page as needing OCR;
//! * [`PdfError::Invalid`] marks every page as `uncertain`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::inventory::{build_record, sha256_hex, SourceRecord};
use crate::loader::pdf::{load_pdf, PdfError, PdfExtraction};

/// The prefix that a transcription page marker uses in the Markdown source.
const TRANSCRIPTION_PAGE_PREFIX: &str = "Trang PDF ";

/// Text-layer status of one PDF page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextLayerStatus {
    /// The page has an extractable text layer.
    Text,
    /// The page has no text layer and needs OCR.
    NoText,
    /// The page is not accounted for by the extraction; treat as a gap.
    Uncertain,
}

impl TextLayerStatus {
    /// True when the page needs OCR (no text, or uncertain coverage).
    pub fn needs_ocr(self) -> bool {
        matches!(self, TextLayerStatus::NoText | TextLayerStatus::Uncertain)
    }
}

/// Transcription coverage status of one PDF page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "status")]
pub enum TranscriptionCoverage {
    /// The transcription names the page with an unambiguous marker.
    Covered { file: String, reference: String },
    /// The transcription has no page reference for this page.
    Unmapped,
    /// The transcription has an ambiguous page reference for this page.
    Uncertain { file: String, reference: String },
}

impl TranscriptionCoverage {
    /// True when the page is neither covered nor uncertain.
    pub fn is_unmapped(&self) -> bool {
        matches!(self, TranscriptionCoverage::Unmapped)
    }
}

/// The assessment of one PDF page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageAssessment {
    /// 1-based page number in document order.
    pub page: u32,
    /// Text-layer status derived from the loader.
    pub text_layer: TextLayerStatus,
    /// Count of extracted characters when the page has text, else `None`.
    pub char_count: Option<usize>,
    /// Transcription coverage derived from the Markdown source.
    pub transcription: TranscriptionCoverage,
}

/// The whole-PDF assessment: one [`PageAssessment`] per page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfAssessment {
    /// Every page, ordered by `page`, each named exactly once.
    pub pages: Vec<PageAssessment>,
}

impl PdfAssessment {
    /// The 1-based page numbers that need OCR.
    pub fn ocr_gap_pages(&self) -> Vec<u32> {
        self.pages
            .iter()
            .filter(|page| page.text_layer.needs_ocr())
            .map(|page| page.page)
            .collect()
    }

    /// The total page count in this assessment.
    pub fn page_count(&self) -> u32 {
        self.pages.last().map(|page| page.page).unwrap_or(0)
    }
}

/// Scans a transcription Markdown for `Trang PDF <n>` page markers.
///
/// A marker is a level-2 heading line that starts with `## Trang PDF` and
/// carries a positive integer. A line that starts the same prefix but does not
/// resolve to a single integer is recorded as ambiguous; the page number it
/// might refer to cannot be trusted.
fn parse_page_markers(markdown: &str) -> (BTreeMap<u32, String>, Vec<String>) {
    let mut markers: BTreeMap<u32, String> = BTreeMap::new();
    let mut ambiguous: Vec<String> = Vec::new();
    for line in markdown.lines() {
        let trimmed = line.trim();
        let body = match trimmed.strip_prefix("## ") {
            Some(rest) => rest.trim(),
            None => continue,
        };
        let Some(suffix) = body.strip_prefix(TRANSCRIPTION_PAGE_PREFIX) else {
            continue;
        };
        let token = suffix.trim();
        match token.parse::<u32>() {
            Ok(page) if page > 0 => {
                markers.insert(page, format!("{TRANSCRIPTION_PAGE_PREFIX}{page}"));
            }
            _ => ambiguous.push(body.to_string()),
        }
    }
    (markers, ambiguous)
}

/// Builds the transcription coverage for every page in `1..=page_count`.
pub fn transcription_coverage(
    markdown: &str,
    file: &str,
    page_count: u32,
) -> Vec<TranscriptionCoverage> {
    let (markers, ambiguous) = parse_page_markers(markdown);
    (1..=page_count)
        .map(|page| match markers.get(&page) {
            Some(reference) => TranscriptionCoverage::Covered {
                file: file.to_string(),
                reference: reference.clone(),
            },
            None => {
                // Any ambiguous marker makes coverage for a page uncertain when
                // the ambiguous text could refer to that page. To stay
                // conservative we treat a page as uncertain when an ambiguous
                // marker exists and no clean marker covers it.
                if let Some(reference) = ambiguous.first() {
                    TranscriptionCoverage::Uncertain {
                        file: file.to_string(),
                        reference: reference.clone(),
                    }
                } else {
                    TranscriptionCoverage::Unmapped
                }
            }
        })
        .collect()
}

/// Builds an assessment from a parsed extraction and a total page count.
///
/// `page_count` comes from the TRAN-001A manifest. Pages outside both the
/// section set and the incomplete set are `uncertain`, never a silent pass.
pub fn assess_extraction(extraction: &PdfExtraction, page_count: u32) -> PdfAssessment {
    let mut by_page: BTreeMap<u32, (TextLayerStatus, Option<usize>)> = BTreeMap::new();
    for section in &extraction.sections {
        if let Some(page) = section.page {
            let chars = section.content.chars().count();
            by_page.insert(page, (TextLayerStatus::Text, Some(chars)));
        }
    }
    for &page in &extraction.incomplete_pages {
        by_page
            .entry(page)
            .or_insert((TextLayerStatus::NoText, None));
    }

    let pages = (1..=page_count)
        .map(|page| {
            let (text_layer, char_count) = by_page
                .get(&page)
                .copied()
                .unwrap_or((TextLayerStatus::Uncertain, None));
            PageAssessment {
                page,
                text_layer,
                char_count,
                transcription: TranscriptionCoverage::Unmapped,
            }
        })
        .collect();
    PdfAssessment { pages }
}

/// Builds an assessment from PDF bytes and a total page count.
///
/// [`PdfError::Scanned`] marks every page as `no-text`; [`PdfError::Invalid`]
/// marks every page as `uncertain`.
pub fn assess_bytes(bytes: &[u8], page_count: u32) -> PdfAssessment {
    match crate::loader::pdf::load_pdf_bytes(bytes) {
        Ok(extraction) => assess_extraction(&extraction, page_count),
        Err(PdfError::Scanned) => PdfAssessment {
            pages: (1..=page_count)
                .map(|page| PageAssessment {
                    page,
                    text_layer: TextLayerStatus::NoText,
                    char_count: None,
                    transcription: TranscriptionCoverage::Unmapped,
                })
                .collect(),
        },
        Err(PdfError::Invalid(_)) => PdfAssessment {
            pages: (1..=page_count)
                .map(|page| PageAssessment {
                    page,
                    text_layer: TextLayerStatus::Uncertain,
                    char_count: None,
                    transcription: TranscriptionCoverage::Unmapped,
                })
                .collect(),
        },
    }
}

/// Attaches transcription coverage to every page in the assessment.
pub fn apply_transcription(
    mut assessment: PdfAssessment,
    markdown: &str,
    file: &str,
) -> PdfAssessment {
    let coverage = transcription_coverage(markdown, file, assessment.page_count());
    for (page, coverage) in assessment.pages.iter_mut().zip(coverage) {
        page.transcription = coverage;
    }
    assessment
}

/// The result of assessing one PDF source: the manifest record and the pages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PdfAssessmentReport {
    /// The TRAN-001A record for the PDF (path, size, SHA-256, page count).
    pub source: SourceRecord,
    /// The transcription file name used as the coverage source.
    pub transcription_file: String,
    /// One assessment per page.
    pub assessment: PdfAssessment,
}

/// Reads a PDF and a transcription file from a source root and builds a report.
///
/// Reads only. The SHA-256 in `source` is the manifest value recorded by
/// TRAN-001A; the transcription is opened for reading and left unchanged.
pub fn assess_source(
    root: &Path,
    pdf_rel: &str,
    transcription_rel: &str,
) -> Result<PdfAssessmentReport, String> {
    let pdf_path = Path::new(pdf_rel);
    let record = build_record(root, pdf_path);
    if record.is_missing() {
        return Err(format!("pdf source missing: {pdf_rel}"));
    }
    let page_count = record
        .pdf_page_count
        .ok_or_else(|| format!("cannot count pages: {pdf_rel}"))?;
    let bytes = std::fs::read(root.join(pdf_path))
        .map_err(|err| format!("cannot read {}: {err}", root.join(pdf_path).display()))?;
    let markdown = std::fs::read_to_string(root.join(transcription_rel))
        .map_err(|err| format!("cannot read {transcription_rel}: {err}"))?;
    let assessment = assess_bytes(&bytes, page_count);
    let assessment = apply_transcription(assessment, &markdown, transcription_rel);
    Ok(PdfAssessmentReport {
        source: record,
        transcription_file: transcription_rel.to_string(),
        assessment,
    })
}

/// Renders the report as byte-stable Markdown with a trailing newline.
pub fn render_markdown(report: &PdfAssessmentReport) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(out, "# PDF Assessment (TRAN-001B)");
    let _ = writeln!(out);
    let _ = writeln!(out, "## Source");
    let _ = writeln!(out);
    let _ = writeln!(out, "- path: `{}`", report.source.relative_path);
    let _ = writeln!(
        out,
        "- sha256: `{}`",
        report.source.sha256.as_deref().unwrap_or("(unavailable)")
    );
    let _ = writeln!(out, "- size_bytes: `{}`", report.source.size_bytes);
    let _ = writeln!(
        out,
        "- pdf_page_count: `{}`",
        report
            .source
            .pdf_page_count
            .map(|n| n.to_string())
            .unwrap_or_else(|| "(unknown)".to_string())
    );
    if let Some(notes) = &report.source.notes {
        let _ = writeln!(out, "- notes: `{notes}`");
    }
    let _ = writeln!(out, "- transcription: `{}`", report.transcription_file);
    let _ = writeln!(out);

    let _ = writeln!(out, "## OCR gaps");
    let _ = writeln!(out);
    let gaps = report.assessment.ocr_gap_pages();
    if gaps.is_empty() {
        let _ = writeln!(out, "none");
    } else {
        for page in &gaps {
            let _ = writeln!(out, "- page {page}");
        }
    }
    let _ = writeln!(out);

    let _ = writeln!(out, "## Page table");
    let _ = writeln!(out);
    let _ = writeln!(out, "| page | text-layer | characters | transcription |");
    let _ = writeln!(out, "| --- | --- | --- | --- |");
    for page in &report.assessment.pages {
        let chars = page
            .char_count
            .map(|n| n.to_string())
            .unwrap_or_else(|| "-".to_string());
        let coverage = match &page.transcription {
            TranscriptionCoverage::Covered { file, reference } => {
                format!("covered: {file} {reference}")
            }
            TranscriptionCoverage::Unmapped => "unmapped".to_string(),
            TranscriptionCoverage::Uncertain { file, reference } => {
                format!("uncertain: {file} {reference}")
            }
        };
        let _ = writeln!(
            out,
            "| {} | {} | {} | {} |",
            page.page,
            status_label(page.text_layer),
            chars,
            coverage
        );
    }
    out.push('\n');
    out
}

fn status_label(status: TextLayerStatus) -> &'static str {
    match status {
        TextLayerStatus::Text => "text",
        TextLayerStatus::NoText => "no-text",
        TextLayerStatus::Uncertain => "uncertain",
    }
}

/// Rejects a report path that collides with a source file or a frozen tree.
/// Thin wrapper over [`crate::inventory::reject_manifest_path`].
pub fn reject_report_path(root: &Path, report: &Path) -> Result<(), String> {
    crate::inventory::reject_manifest_path(root, report)
}

/// Computes a SHA-256 of a file's bytes; helper for before/after checks.
pub fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    Ok(sha256_hex(&bytes))
}

/// Loads a PDF from `path` and returns its page count from the manifest logic.
/// Kept for parity with the loader so callers can cross-check page counts.
pub fn loader_page_count(path: impl AsRef<Path>) -> Result<u32, PdfError> {
    let extraction = load_pdf(path)?;
    Ok(extraction
        .sections
        .iter()
        .filter_map(|section| section.page)
        .chain(extraction.incomplete_pages.iter().copied())
        .max()
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Section;

    fn extraction(sections: Vec<(u32, &str)>, incomplete: Vec<u32>) -> PdfExtraction {
        let sections = sections
            .into_iter()
            .map(|(page, text)| Section::with_page(format!("Page {page}"), text.to_string(), page))
            .collect();
        PdfExtraction {
            sections,
            incomplete_pages: incomplete,
        }
    }

    #[test]
    fn each_page_named_once() {
        let a = assess_extraction(&extraction(vec![(1, "hello")], vec![2]), 2);
        let pages: Vec<u32> = a.pages.iter().map(|p| p.page).collect();
        assert_eq!(pages, vec![1, 2]);
    }

    #[test]
    fn status_serializes_kebab_case_and_roundtrips() {
        assert_eq!(
            serde_json::to_string(&TextLayerStatus::NoText).unwrap(),
            "\"no-text\""
        );
        let page = PageAssessment {
            page: 3,
            text_layer: TextLayerStatus::Uncertain,
            char_count: None,
            transcription: TranscriptionCoverage::Covered {
                file: "t.md".into(),
                reference: "Trang PDF 3".into(),
            },
        };
        let text = serde_json::to_string(&page).unwrap();
        let back: PageAssessment = serde_json::from_str(&text).unwrap();
        assert_eq!(page, back);
    }

    #[test]
    fn mixed_pdf_marks_page_two_no_text() {
        let a = assess_extraction(&extraction(vec![(1, "text")], vec![2]), 2);
        assert_eq!(a.pages[0].text_layer, TextLayerStatus::Text);
        assert_eq!(a.pages[1].text_layer, TextLayerStatus::NoText);
        assert_eq!(a.ocr_gap_pages(), vec![2]);
    }

    #[test]
    fn empty_section_set_yields_uncertain() {
        let a = assess_extraction(&extraction(vec![], vec![]), 3);
        assert!(a
            .pages
            .iter()
            .all(|p| p.text_layer == TextLayerStatus::Uncertain));
        assert_eq!(a.ocr_gap_pages().len(), 3);
    }

    #[test]
    fn scanned_error_marks_all_pages_no_text() {
        let bytes = std::fs::read("fixtures/parity/inputs/no-text-layer.pdf").unwrap();
        let a = assess_bytes(&bytes, 1);
        assert_eq!(a.page_count(), 1);
        assert!(a
            .pages
            .iter()
            .all(|p| p.text_layer == TextLayerStatus::NoText));
    }

    #[test]
    fn text_layer_fixture_marks_text() {
        let bytes = std::fs::read("fixtures/parity/inputs/text-layer.pdf").unwrap();
        let a = assess_bytes(&bytes, 2);
        assert!(a
            .pages
            .iter()
            .all(|p| p.text_layer == TextLayerStatus::Text));
    }

    #[test]
    fn mixed_fixture_marks_page_two_no_text() {
        let bytes = std::fs::read("fixtures/parity/inputs/mixed-text-scan.pdf").unwrap();
        let a = assess_bytes(&bytes, 2);
        assert_eq!(a.pages[0].text_layer, TextLayerStatus::Text);
        assert_eq!(a.pages[1].text_layer, TextLayerStatus::NoText);
    }

    #[test]
    fn no_text_layer_fixture_marks_all_no_text() {
        let bytes = std::fs::read("fixtures/parity/inputs/no-text-layer.pdf").unwrap();
        let a = assess_bytes(&bytes, 1);
        assert_eq!(a.pages[0].text_layer, TextLayerStatus::NoText);
    }

    #[test]
    fn transcription_markers_cover_named_pages() {
        let md = "## Trang PDF 1\nbody\n## Trang PDF 3\nbody\n";
        let coverage = transcription_coverage(md, "t.md", 3);
        assert!(matches!(coverage[0], TranscriptionCoverage::Covered { .. }));
        assert_eq!(coverage[1], TranscriptionCoverage::Unmapped);
        assert!(matches!(coverage[2], TranscriptionCoverage::Covered { .. }));
    }

    #[test]
    fn ambiguous_marker_is_uncertain_not_covered() {
        let md = "## Trang PDF 1\n## Trang PDF one\n";
        let coverage = transcription_coverage(md, "t.md", 2);
        assert!(matches!(coverage[0], TranscriptionCoverage::Covered { .. }));
        assert!(matches!(
            coverage[1],
            TranscriptionCoverage::Uncertain { .. }
        ));
    }

    #[test]
    fn byte_stable_markdown_across_runs() {
        let bytes = std::fs::read("fixtures/parity/inputs/mixed-text-scan.pdf").unwrap();
        let record = build_record(
            Path::new("."),
            Path::new("fixtures/parity/inputs/mixed-text-scan.pdf"),
        );
        let a = assess_bytes(&bytes, 2);
        let report = PdfAssessmentReport {
            source: record,
            transcription_file: "t.md".into(),
            assessment: a,
        };
        assert_eq!(render_markdown(&report), render_markdown(&report));
    }

    #[test]
    fn real_transcription_covers_all_pages() {
        let md = std::fs::read_to_string("docs/tran/Tran-Family-Vietnamese.md").unwrap();
        let coverage = transcription_coverage(&md, "docs/tran/Tran-Family-Vietnamese.md", 56);
        assert_eq!(coverage.len(), 56);
        assert!(coverage
            .iter()
            .all(|c| matches!(c, TranscriptionCoverage::Covered { .. })));
    }
}
