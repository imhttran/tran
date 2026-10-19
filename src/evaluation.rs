//! Retrieval-evaluation harness.
//!
//! Ported from the Go baseline `cmd/eval`. Loads a JSON dataset of evaluation
//! cases, runs every case through the shared retrieval modules over a corpus
//! loaded once, and aggregates the section-based recall and precision metrics.
//!
//! The module owns only the evaluation concerns: dataset loading and
//! validation, per-case ranking, and metric aggregation. Retrieval itself stays
//! in [`crate::retrieval`]; the corpus read path stays in
//! [`crate::storage_pg::PgStore`]; the query embedding stays in
//! `crate::embedding::Embedder`.
//!
//! Comparison rules (frozen, see `docs/plans/EVAL-CONTRACT.md`):
//!
//! - Relevance is section-based: a retrieved chunk matches an expected document
//!   when `source` and `section` are equal. Several chunks from one
//!   source/section count once.
//! - `recall@k = matched_expected / |expected|`, and `0` when `expected` is
//!   empty. `|expected|` counts expected entries, not distinct sections.
//! - `precision@k = relevant_retrieved_sections / |distinct_retrieved_sections|`,
//!   and `0` when nothing was retrieved. Distinct sections preserve first
//!   appearance order.
//! - Per-case source/section ordering is the fused ranking order. Fusion,
//!   vector search and FTS all break ties by candidate id ascending, so the
//!   ordering is deterministic.
//! - Aggregate metrics average over the answerable cases (cases with at least
//!   one expected document); a run with no answerable case reports `0.00`.

use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::document::Chunk;
use crate::embedding::Embedder;
use crate::retrieval::vector::{self, IndexedVector};
use crate::retrieval::{dedup, expand, fts, fusion, Candidate};
use crate::storage_pg::{PgStore, StoredChunkRow};

/// The dataset scored when `EVAL_DATASET` is unset or empty, matching the Go
/// baseline's `defaultDatasetPath`.
pub const DEFAULT_DATASET_PATH: &str = "evals/retrieval.json";

/// One expected document inside a case. `evidence` is accepted by the shared
/// dataset and is not used by the baseline metrics this stage reproduces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedDocument {
    /// Expected source path.
    pub source: String,
    /// Expected section title.
    pub section: String,
    /// Optional evidence snippets, carried through for the later stages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

/// One evaluation case: a question and its expected documents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalCase {
    /// The question asked of the retrieval pipeline.
    pub question: String,
    /// Expected documents; empty means the case is unanswerable.
    #[serde(default)]
    pub expected: Vec<ExpectedDocument>,
    /// Optional expected facts, carried through for the later stages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expected_facts: Vec<String>,
}

impl EvalCase {
    /// Reports whether the case has at least one expected document.
    pub fn is_answerable(&self) -> bool {
        !self.expected.is_empty()
    }
}

/// Retrieval settings the harness forwards into the shared modules.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalSettings {
    /// Minimum cosine similarity kept by vector search, `MIN_SIMILARITY`.
    pub min_similarity: f64,
    /// Neighbour window forwarded to [`expand::expand`], `EXPAND_LIMIT`.
    pub expand_limit: u32,
    /// Number of final candidates kept after expansion, `FINAL_K`.
    pub final_k: usize,
    /// Candidate pool size per path, `TOP_K`.
    pub candidate_k: usize,
    /// Corpus language used to select the PostgreSQL FTS configuration.
    pub language: String,
}

/// Recall and precision recorded for one ranking.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Metrics {
    /// Section-based recall.
    pub recall: f64,
    /// Section-based precision.
    pub precision: f64,
}

/// One entry in the per-case ranking.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedChunk {
    /// Source path.
    pub source: String,
    /// Section title.
    pub section: String,
    /// Chunk index inside the source.
    pub chunk_index: u32,
    /// Fused score.
    pub score: f64,
}

/// The result of one evaluation case.
#[derive(Debug, Clone, PartialEq)]
pub struct CaseResult {
    /// The case that was evaluated.
    pub case: EvalCase,
    /// Whether the case has expected documents.
    pub answerable: bool,
    /// The final fused ordering, truncated to `FINAL_K`.
    pub ranking: Vec<RankedChunk>,
    /// Recall and precision of `ranking`.
    pub metrics: Metrics,
}

/// Aggregate recall and precision over the answerable cases.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aggregate {
    /// Number of cases evaluated.
    pub cases: usize,
    /// Number of cases with at least one expected document.
    pub answerable: usize,
    /// Mean recall over the answerable cases.
    pub recall: f64,
    /// Mean precision over the answerable cases.
    pub precision: f64,
}

/// Error returned by the evaluation harness. Each message names the offending
/// artifact so a mistaken setting is actionable.
#[derive(Debug)]
pub enum EvalError {
    /// The dataset could not be read; names the path.
    Dataset(String),
    /// The dataset did not match the schema; names the path and field.
    Schema(String),
    /// The document store could not be reached or the corpus is empty.
    Database(String),
    /// A question could not be embedded.
    Embedding(String),
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Dataset(message) => write!(f, "read evaluation dataset: {message}"),
            EvalError::Schema(message) => write!(f, "parse evaluation dataset: {message}"),
            EvalError::Database(message) => write!(f, "document store: {message}"),
            EvalError::Embedding(message) => write!(f, "embed question: {message}"),
        }
    }
}

impl std::error::Error for EvalError {}

/// Returns the dataset path selected by `path`, falling back to
/// [`DEFAULT_DATASET_PATH`] when it is absent or blank.
pub fn dataset_path(path: Option<&str>) -> PathBuf {
    match path.map(str::trim) {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => PathBuf::from(DEFAULT_DATASET_PATH),
    }
}

/// Loads and validates the dataset at `path`.
pub fn load_dataset(path: &Path) -> Result<Vec<EvalCase>, EvalError> {
    let data = fs::read_to_string(path)
        .map_err(|err| EvalError::Dataset(format!("{}: {err}", path.display())))?;
    parse_dataset(&data, path)
}

/// Parses and validates `data`; `path` appears only in error messages.
pub fn parse_dataset(data: &str, path: &Path) -> Result<Vec<EvalCase>, EvalError> {
    let cases: Vec<EvalCase> = serde_json::from_str(data).map_err(|err| {
        EvalError::Schema(format!(
            "{}: {err} at line {} column {}",
            path.display(),
            err.line(),
            err.column()
        ))
    })?;

    validate_cases(&cases, path)?;
    Ok(cases)
}

fn validate_cases(cases: &[EvalCase], path: &Path) -> Result<(), EvalError> {
    for (index, case) in cases.iter().enumerate() {
        if case.question.trim().is_empty() {
            return Err(EvalError::Schema(format!(
                "{}: cases[{index}].question must not be empty",
                path.display()
            )));
        }
        for (doc_index, expected) in case.expected.iter().enumerate() {
            if expected.source.trim().is_empty() {
                return Err(EvalError::Schema(format!(
                    "{}: cases[{index}].expected[{doc_index}].source must not be empty",
                    path.display()
                )));
            }
            if expected.section.trim().is_empty() {
                return Err(EvalError::Schema(format!(
                    "{}: cases[{index}].expected[{doc_index}].section must not be empty",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

/// Reads the corpus selected by `corpus` from `store` and converts it to the
/// chunk and vector inputs the shared retrieval modules consume.
pub fn load_corpus(
    store: &PgStore,
    corpus: &str,
) -> Result<(Vec<Chunk>, Vec<IndexedVector>), EvalError> {
    let rows: Vec<StoredChunkRow> = store
        .rows_for_corpus(corpus)
        .map_err(|err| EvalError::Database(format!("load corpus {corpus:?}: {err}")))?;
    if rows.is_empty() {
        return Err(EvalError::Database(format!(
            "corpus {corpus:?} is empty in the document store"
        )));
    }

    let chunks: Vec<Chunk> = rows.iter().map(stored_chunk).collect();
    let index: Vec<IndexedVector> = rows
        .iter()
        .map(|row| IndexedVector {
            chunk: stored_chunk(row),
            embedding: row.chunk.embedding.clone(),
        })
        .collect();
    Ok((chunks, index))
}

fn stored_chunk(row: &StoredChunkRow) -> Chunk {
    Chunk {
        source: row.chunk.source.clone().unwrap_or_default(),
        section: row.chunk.section.clone().unwrap_or_default(),
        language: row.chunk.language.clone().unwrap_or_default(),
        chunk_index: row.chunk.chunk_index.max(0) as u32,
        content: row.chunk.content.clone(),
        page: row.chunk.page.and_then(|page| u32::try_from(page).ok()),
    }
}

/// Ranks the corpus for one question through the shared retrieval modules:
/// vector search, full-text search, reciprocal rank fusion, deduplication,
/// section expansion, then truncation to `FINAL_K`.
pub fn rank(
    index: &[IndexedVector],
    chunks: &[Chunk],
    query: &str,
    query_embedding: &[f64],
    settings: &EvalSettings,
) -> Vec<Candidate> {
    let vector_hits = vector::search(index, query_embedding, settings.min_similarity);
    let fts_hits = fts::search(chunks, query);
    let mut candidates = fusion::fuse(&[vector_hits, fts_hits]);
    candidates = dedup::dedup(candidates);
    candidates = expand::expand(&candidates, chunks, settings.expand_limit);
    candidates.truncate(settings.final_k.max(1));
    candidates
}

/// Runs every case over `chunks`/`index`, embedding each question with
/// `embedder`, and returns the ordered per-case results.
pub fn evaluate<E: Embedder + ?Sized>(
    cases: &[EvalCase],
    chunks: &[Chunk],
    index: &[IndexedVector],
    embedder: &E,
    settings: &EvalSettings,
) -> Result<Vec<CaseResult>, EvalError> {
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let query_embedding = embedder
            .embed(&case.question)
            .map_err(|err| EvalError::Embedding(format!("{err}")))?;
        let candidates = rank(index, chunks, &case.question, &query_embedding, settings);
        let ranking: Vec<RankedChunk> = candidates
            .iter()
            .map(|candidate| RankedChunk {
                source: candidate.document.chunk.source.clone(),
                section: candidate.document.chunk.section.clone(),
                chunk_index: candidate.document.chunk.chunk_index,
                score: candidate.score,
            })
            .collect();
        let metrics = score_ranking(&case.expected, &candidates);
        results.push(CaseResult {
            case: case.clone(),
            answerable: case.is_answerable(),
            ranking,
            metrics,
        });
    }
    Ok(results)
}

/// Runs the SQL-backed production retrieval path for every case and scores the
/// raw fused ranking at `FINAL_K`, matching the Go `cmd/eval` hybrid metrics.
pub fn evaluate_sql<E: Embedder + ?Sized>(
    cases: &[EvalCase],
    store: &PgStore,
    corpus: &str,
    embedder: &E,
    settings: &EvalSettings,
) -> Result<Vec<CaseResult>, EvalError> {
    let candidate_k = settings.candidate_k.max(1);
    let mut results = Vec::with_capacity(cases.len());
    for case in cases {
        let query_embedding = embedder
            .embed(&case.question)
            .map_err(|err| EvalError::Embedding(format!("{err}")))?;
        let vector_hits: Vec<_> = store
            .vector_search(corpus, &query_embedding, candidate_k)
            .map_err(|err| EvalError::Database(format!("vector search: {err}")))?
            .into_iter()
            .filter(|hit| hit.similarity >= settings.min_similarity)
            .collect();
        let keyword_hits = store
            .keyword_search(corpus, &case.question, candidate_k, &settings.language)
            .map_err(|err| EvalError::Database(format!("keyword search: {err}")))?;
        let fused =
            crate::retrieval_pg::fuse(&[vector_hits, keyword_hits], settings.final_k.max(1));
        // The Go baseline deduplicates sections after fusing at FINAL_K, so a
        // section contributes only its highest-ranked chunk. The ranking and
        // metrics must use that same list.
        let fused = crate::retrieval::dedup::dedup_sections(fused);
        let ranking: Vec<RankedChunk> = fused
            .iter()
            .map(|candidate| RankedChunk {
                source: candidate.document.chunk.source.clone(),
                section: candidate.document.chunk.section.clone(),
                chunk_index: candidate.document.chunk.chunk_index,
                score: candidate.score,
            })
            .collect();
        let metrics = score_ranking(&case.expected, &fused);
        results.push(CaseResult {
            case: case.clone(),
            answerable: case.is_answerable(),
            ranking,
            metrics,
        });
    }
    Ok(results)
}

/// Scores one ranking against the expected documents.
pub fn score_ranking(expected: &[ExpectedDocument], candidates: &[Candidate]) -> Metrics {
    let actual: Vec<(&str, &str)> = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.document.chunk.source.as_str(),
                candidate.document.chunk.section.as_str(),
            )
        })
        .collect();
    score_sections(expected, &actual)
}

/// Scores one ranking given as `(source, section)` pairs.
pub fn score_sections(expected: &[ExpectedDocument], actual: &[(&str, &str)]) -> Metrics {
    Metrics {
        recall: recall_at_k(expected, actual),
        precision: precision_at_k(expected, actual),
    }
}

fn matches(expected: &ExpectedDocument, entry: &(&str, &str)) -> bool {
    expected.source == entry.0 && expected.section == entry.1
}

/// `recall@k = matched_expected / |expected|`, `0` when either side is empty.
fn recall_at_k(expected: &[ExpectedDocument], actual: &[(&str, &str)]) -> f64 {
    if expected.is_empty() || actual.is_empty() {
        return 0.0;
    }
    let found = expected
        .iter()
        .filter(|doc| actual.iter().any(|entry| matches(doc, entry)))
        .count();
    found as f64 / expected.len() as f64
}

/// `precision@k = relevant_distinct_sections / |distinct_sections|`.
fn precision_at_k(expected: &[ExpectedDocument], actual: &[(&str, &str)]) -> f64 {
    let mut seen: HashSet<(&str, &str)> = HashSet::new();
    let mut distinct: Vec<(&str, &str)> = Vec::new();
    for entry in actual {
        if seen.insert(*entry) {
            distinct.push(*entry);
        }
    }
    if distinct.is_empty() {
        return 0.0;
    }
    let relevant = distinct
        .iter()
        .filter(|entry| expected.iter().any(|doc| matches(doc, entry)))
        .count();
    relevant as f64 / distinct.len() as f64
}

/// Averages recall and precision over the answerable cases.
pub fn aggregate(results: &[CaseResult]) -> Aggregate {
    let answerable: Vec<&CaseResult> = results.iter().filter(|r| r.answerable).collect();
    let count = answerable.len();
    if count == 0 {
        return Aggregate {
            cases: results.len(),
            answerable: 0,
            recall: 0.0,
            precision: 0.0,
        };
    }
    let recall_sum: f64 = answerable.iter().map(|r| r.metrics.recall).sum();
    let precision_sum: f64 = answerable.iter().map(|r| r.metrics.precision).sum();
    Aggregate {
        cases: results.len(),
        answerable: count,
        recall: recall_sum / count as f64,
        precision: precision_sum / count as f64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected(pairs: &[(&str, &str)]) -> Vec<ExpectedDocument> {
        pairs
            .iter()
            .map(|(source, section)| ExpectedDocument {
                source: (*source).to_string(),
                section: (*section).to_string(),
                evidence: Vec::new(),
            })
            .collect()
    }

    fn chunk(source: &str, section: &str, index: u32, body: &str) -> Chunk {
        Chunk {
            source: source.to_string(),
            section: section.to_string(),
            language: "english".to_string(),
            chunk_index: index,
            content: body.to_string(),
            page: None,
        }
    }

    #[test]
    fn parses_the_shared_dataset_schema() {
        let data = r#"[
            {"question": "What is retrieval?", "expected": [{"source": "doc.md", "section": "Intro"}]},
            {"question": "Unanswerable", "expected": []}
        ]"#;
        let cases = parse_dataset(data, Path::new("evals/retrieval.json")).unwrap();
        assert_eq!(cases.len(), 2);
        assert_eq!(cases[0].expected[0].section, "Intro");
        assert!(cases[0].is_answerable());
        assert!(!cases[1].is_answerable());
    }

    #[test]
    fn malformed_json_names_the_path() {
        let err = parse_dataset("{", Path::new("evals/retrieval.json")).unwrap_err();
        assert!(err.to_string().contains("evals/retrieval.json"), "{err}");
    }

    #[test]
    fn missing_section_field_names_the_path() {
        let err = parse_dataset(
            r#"[{"question": "q", "expected": [{"source": "doc.md"}]}]"#,
            Path::new("d.json"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("d.json"), "{err}");
    }

    #[test]
    fn blank_question_names_the_field() {
        let err = parse_dataset(
            r#"[{"question": "  ", "expected": []}]"#,
            Path::new("d.json"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("cases[0].question"), "{err}");
    }

    #[test]
    fn blank_source_names_the_field() {
        let err = parse_dataset(
            r#"[{"question": "q", "expected": [{"source": "", "section": "S"}]}]"#,
            Path::new("d.json"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("expected[0].source"), "{err}");
    }

    #[test]
    fn dataset_path_defaults_and_overrides() {
        assert_eq!(dataset_path(None), PathBuf::from(DEFAULT_DATASET_PATH));
        assert_eq!(
            dataset_path(Some("  ")),
            PathBuf::from(DEFAULT_DATASET_PATH)
        );
        assert_eq!(dataset_path(Some("x.json")), PathBuf::from("x.json"));
    }

    #[test]
    fn recall_counts_matched_expected_documents() {
        let exp = expected(&[("doc.md", "Intro"), ("doc.md", "Body")]);
        let actual = [("doc.md", "Intro"), ("doc.md", "Other")];
        assert!((recall_at_k(&exp, &actual) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn recall_is_zero_when_nothing_expected() {
        assert_eq!(recall_at_k(&[], &[("doc.md", "Intro")]), 0.0);
    }

    #[test]
    fn precision_counts_distinct_sections_once() {
        let exp = expected(&[("doc.md", "Intro")]);
        let actual = [("doc.md", "Intro"), ("doc.md", "Intro"), ("doc.md", "Body")];
        assert!((precision_at_k(&exp, &actual) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn precision_is_zero_when_nothing_retrieved() {
        let exp = expected(&[("doc.md", "Intro")]);
        assert_eq!(precision_at_k(&exp, &[]), 0.0);
    }

    fn case_result(answerable: bool, metrics: Metrics) -> CaseResult {
        CaseResult {
            case: EvalCase {
                question: "q".to_string(),
                expected: Vec::new(),
                expected_facts: Vec::new(),
            },
            answerable,
            ranking: Vec::new(),
            metrics,
        }
    }

    #[test]
    fn aggregate_averages_over_answerable_cases() {
        let cases = vec![
            case_result(
                true,
                Metrics {
                    recall: 1.0,
                    precision: 0.5,
                },
            ),
            case_result(
                false,
                Metrics {
                    recall: 0.0,
                    precision: 0.0,
                },
            ),
        ];
        let agg = aggregate(&cases);
        assert_eq!(agg.cases, 2);
        assert_eq!(agg.answerable, 1);
        assert!((agg.recall - 1.0).abs() < 1e-12);
        assert!((agg.precision - 0.5).abs() < 1e-12);
    }

    #[test]
    fn aggregate_without_answerable_cases_is_zero() {
        let agg = aggregate(&[]);
        assert_eq!(agg.answerable, 0);
        assert_eq!(agg.recall, 0.0);
        assert_eq!(agg.precision, 0.0);
    }

    #[test]
    fn rank_uses_the_shared_retrieval_modules_in_order() {
        let chunks = vec![
            chunk("doc.md", "Intro", 0, "rust retrieval search"),
            chunk("doc.md", "Body", 1, "other content"),
        ];
        let index: Vec<IndexedVector> = vec![
            IndexedVector {
                chunk: chunks[0].clone(),
                embedding: vec![1.0, 0.0],
            },
            IndexedVector {
                chunk: chunks[1].clone(),
                embedding: vec![0.0, 1.0],
            },
        ];
        let settings = EvalSettings {
            min_similarity: 0.5,
            expand_limit: 0,
            final_k: 3,
            candidate_k: 3,
            language: String::new(),
        };
        let ranked = rank(&index, &chunks, "rust search", &[1.0, 0.0], &settings);
        assert_eq!(ranked[0].document.chunk.section, "Intro");
        assert!(ranked.len() <= 3);
    }

    #[test]
    fn rank_truncates_to_final_k() {
        let chunks = vec![
            chunk("doc.md", "A", 0, "alpha"),
            chunk("doc.md", "B", 1, "beta"),
            chunk("doc.md", "C", 2, "gamma"),
            chunk("doc.md", "D", 3, "delta"),
        ];
        let index: Vec<IndexedVector> = chunks
            .iter()
            .map(|c| IndexedVector {
                chunk: c.clone(),
                embedding: vec![1.0, 0.0],
            })
            .collect();
        let settings = EvalSettings {
            min_similarity: 0.0,
            expand_limit: 0,
            final_k: 2,
            candidate_k: 3,
            language: String::new(),
        };
        let ranked = rank(&index, &chunks, "alpha", &[1.0, 0.0], &settings);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn per_case_ordering_is_deterministic_and_section_based() {
        let chunks = vec![
            chunk("doc.md", "A", 0, "alpha"),
            chunk("doc.md", "B", 1, "alpha"),
            chunk("doc.md", "C", 2, "alpha"),
        ];
        let index: Vec<IndexedVector> = chunks
            .iter()
            .map(|c| IndexedVector {
                chunk: c.clone(),
                embedding: vec![1.0, 0.0],
            })
            .collect();
        let settings = EvalSettings {
            min_similarity: 0.0,
            expand_limit: 0,
            final_k: 3,
            candidate_k: 3,
            language: String::new(),
        };
        let first = rank(&index, &chunks, "alpha", &[1.0, 0.0], &settings);
        let second = rank(&index, &chunks, "alpha", &[1.0, 0.0], &settings);
        let order = |ranked: &[Candidate]| -> Vec<(String, String)> {
            ranked
                .iter()
                .map(|c| {
                    (
                        c.document.chunk.source.clone(),
                        c.document.chunk.section.clone(),
                    )
                })
                .collect()
        };
        assert_eq!(order(&first), order(&second));
        let scores: Vec<f64> = first.iter().map(|c| c.score).collect();
        assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));
    }
}
