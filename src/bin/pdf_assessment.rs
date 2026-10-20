//! TRAN-001B PDF assessment command.
//!
//! Reads a PDF and its Vietnamese transcription from a source root, builds a
//! per-page assessment of the PDF text layer, and writes a local Markdown report
//! with the page table, the OCR gap pages, transcription coverage, and the
//! source SHA-256 recorded by TRAN-001A. The command is read-only on the source
//! tree; only the report file is written, at a caller-supplied path.
//!
//! Usage:
//!
//! ```text
//! pdf_assessment [SOURCE_ROOT] REPORT_PATH
//! ```
//!
//! `SOURCE_ROOT` defaults to `docs/tran`; `REPORT_PATH` is required. The PDF is
//! `Family Genealogy.pdf` and the transcription is `Tran-Family-Vietnamese.md`,
//! both relative to the source root.
//!
//! Exit status:
//!
//! - `0` — report written.
//! - `2` — usage error, rejected report path, or assessment error.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use rag_foundation::pdf_assessment::{assess_source, reject_report_path, render_markdown};

const DEFAULT_SOURCE_ROOT: &str = "docs/tran";
const PDF_RELATIVE_PATH: &str = "Family Genealogy.pdf";
const TRANSCRIPTION_RELATIVE_PATH: &str = "Tran-Family-Vietnamese.md";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let root = args
        .first()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SOURCE_ROOT));
    let Some(report_arg) = args.get(1) else {
        eprintln!("usage: pdf_assessment [SOURCE_ROOT] REPORT_PATH");
        return ExitCode::from(2);
    };
    let report_path = PathBuf::from(report_arg);

    match run(&root, &report_path) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("pdf_assessment: {err}");
            ExitCode::from(2)
        }
    }
}

fn run(root: &Path, report_path: &Path) -> Result<ExitCode, String> {
    reject_report_path(root, report_path).map_err(|err| err.to_string())?;

    let report = assess_source(root, PDF_RELATIVE_PATH, TRANSCRIPTION_RELATIVE_PATH)
        .map_err(|err| err.to_string())?;
    let markdown = render_markdown(&report);
    write_report(report_path, &markdown)?;

    let gaps = report.assessment.ocr_gap_pages();
    eprintln!(
        "pdf_assessment: {} page(s) assessed from {}",
        report.assessment.page_count(),
        root.display()
    );
    eprintln!("report: {}", report_path.display());
    if gaps.is_empty() {
        eprintln!("ocr gaps: none");
    } else {
        let pages: Vec<String> = gaps.iter().map(u32::to_string).collect();
        eprintln!("ocr gaps: {}", pages.join(", "));
    }

    Ok(ExitCode::SUCCESS)
}

fn write_report(report_path: &Path, markdown: &str) -> Result<(), String> {
    if let Some(parent) = report_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
        }
    }
    fs::write(report_path, markdown)
        .map_err(|err| format!("cannot write {}: {err}", report_path.display()))
}
