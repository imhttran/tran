//! RUST-006C real end-to-end acceptance tests.
//!
//! The deterministic run drives the production ingest service over the real
//! PostgreSQL/pgvector database, the production retrieval stages, and the
//! production `AnswerService` with a mocked model. The frozen baseline gates
//! the deterministic comparison; live-model observations are reported in a
//! separate section and never affect the deterministic verdict.

use e2e_acceptance::{
    cleanup, cleanup_corpus, run_deterministic, unique_tag, Deterministic, Live, Report,
    CORPUS_PATH,
};

/// Removes the deterministic corpus rows even when the test panics.
struct CorpusCleanup(String);

impl Drop for CorpusCleanup {
    fn drop(&mut self) {
        let _ = cleanup_corpus(&self.0);
        let _ = cleanup(CORPUS_PATH);
    }
}

#[test]
fn real_ingest_query_and_cited_answer_match_the_frozen_baseline() {
    let corpus = unique_tag();
    let _cleanup = CorpusCleanup(corpus.clone());
    let (observed, report) = run_deterministic(&corpus).expect("deterministic e2e run");

    assert!(
        report.deterministic.matched,
        "deterministic baseline mismatch:\n{}",
        report.render()
    );
    assert_eq!(observed.chunks, 2, "the baseline corpus yields two chunks");
    assert_eq!(
        observed.top_candidate,
        "fixtures/parity/inputs/text-layer.pdf::Page 1::0"
    );
    assert!(
        observed
            .answer
            .contains("[fixtures/parity/inputs/text-layer.pdf - Page 1 - p.1]"),
        "the answer must cite the corpus source and page: {}",
        observed.answer
    );
    assert_eq!(
        observed.references,
        vec!["fixtures/parity/inputs/text-layer.pdf - Page 1 - p.1".to_string()]
    );
    assert_eq!(
        observed.rejections.len(),
        4,
        "invalid, incomplete, empty, and oversized inputs must all be covered"
    );
    let kinds: Vec<&str> = observed
        .rejections
        .iter()
        .map(|rejection| rejection.kind.as_str())
        .collect();
    assert_eq!(kinds, vec!["loader", "incomplete", "no-chunks", "loader"]);

    let rendered = report.render();
    assert!(rendered.contains("[deterministic]"));
    assert!(rendered.contains("[live-model]"));
    assert!(
        !report.live.available,
        "the deterministic run must not present scripted evidence as a live-model observation"
    );
}

#[test]
fn live_model_evidence_is_reported_in_a_separate_section() {
    let report = Report {
        deterministic: Deterministic {
            checks: vec!["answer: match".to_string()],
            matched: true,
        },
        live: Live {
            available: true,
            observations: vec!["live answer variation recorded".to_string()],
        },
    };
    let rendered = report.render();
    assert!(rendered.contains("[deterministic]"));
    assert!(rendered.contains("check=answer: match"));
    assert!(rendered.contains("[live-model]"));
    assert!(rendered.contains("available=true"));
    assert!(rendered.contains("observation=live answer variation recorded"));
    // The live observation must not appear in the deterministic section.
    let deterministic = rendered
        .split("[live-model]")
        .next()
        .expect("deterministic section");
    assert!(!deterministic.contains("live answer variation recorded"));
}
