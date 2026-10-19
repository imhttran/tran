//! RUST-006B AnswerService acceptance tests.
//!
//! These tests drive the production `AnswerService` with the single retrieval
//! implementation's `Candidate` type. They cover cited source/page metadata,
//! the insufficient-evidence refusal, and both dependency-failure paths.

use std::sync::atomic::{AtomicUsize, Ordering};

use rag_foundation::answer::answerability;
use rag_foundation::answer::service::{
    AnswerError, AnswerService, FixedRetrieval, RetrievalSource,
};
use rag_foundation::document::Chunk;
use rag_foundation::generation::{GenerationError, Generator};
use rag_foundation::ollama::OllamaError;
use rag_foundation::retrieval::Candidate;

struct StubGenerator {
    answer: String,
    calls: AtomicUsize,
}

impl StubGenerator {
    fn new(answer: &str) -> Self {
        Self {
            answer: answer.to_string(),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl Generator for StubGenerator {
    fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self.answer.clone())
    }
}

struct FailingGenerator;

impl Generator for FailingGenerator {
    fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
        Err(GenerationError::Client(OllamaError::Transport {
            path: "/api/generate".to_string(),
            message: "connection refused".to_string(),
        }))
    }
}

struct FailingRetrieval;

impl RetrievalSource for FailingRetrieval {
    fn retrieve(&self, _question: &str) -> Result<Vec<Candidate>, String> {
        Err("database unavailable".to_string())
    }
}

fn candidate(source: &str, page: u32, content: &str) -> Candidate {
    Candidate::new(
        Chunk {
            source: source.to_string(),
            section: "Intro".to_string(),
            language: "english".to_string(),
            chunk_index: 0,
            content: content.to_string(),
            page: Some(page),
        },
        0.9,
    )
}

#[test]
fn cited_source_and_page_metadata_are_returned() {
    let source = "acceptance-doc.md";
    let retrieval = FixedRetrieval::new(vec![candidate(source, 2, "the answer is 42")]);
    let generator = StubGenerator::new("It is 42 [acceptance-doc.md - Intro - p.2].");
    let service = AnswerService::new(&retrieval, &generator, 1000);

    let answer = service.answer("what is the answer?").unwrap();
    assert!(!answer.refused);
    assert!(answer.text.contains("[acceptance-doc.md - Intro - p.2]"));
    assert_eq!(
        answer.references,
        vec!["acceptance-doc.md - Intro - p.2".to_string()]
    );
    assert_eq!(generator.calls(), 1);
}

#[test]
fn invalid_citations_are_dropped() {
    let retrieval = FixedRetrieval::new(vec![candidate("acceptance-doc.md", 2, "body")]);
    let generator = StubGenerator::new(
        "Fact [acceptance-doc.md - Intro - p.2] bogus [acceptance-doc.md - Intro - p.9].",
    );
    let service = AnswerService::new(&retrieval, &generator, 1000);

    let answer = service.answer("what?").unwrap();
    assert!(answer.text.contains("[acceptance-doc.md - Intro - p.2]"));
    assert!(!answer.text.contains("p.9"));
}

#[test]
fn insufficient_evidence_refuses_without_generation() {
    let retrieval = FixedRetrieval::new(Vec::new());
    let generator = StubGenerator::new("must not be used");
    let service = AnswerService::new(&retrieval, &generator, 1000);

    let answer = service.answer("what?").unwrap();
    assert!(answer.refused);
    assert_eq!(answer.text, answerability::REFUSAL);
    assert!(answer.references.is_empty());
    assert_eq!(generator.calls(), 0);
}

#[test]
fn retrieval_dependency_failure_is_reported() {
    let retrieval = FailingRetrieval;
    let generator = StubGenerator::new("unused");
    let service = AnswerService::new(&retrieval, &generator, 1000);

    match service.answer("what?") {
        Err(AnswerError::Retrieval(message)) => {
            assert!(message.contains("database unavailable"))
        }
        other => panic!("want a retrieval error, got {other:?}"),
    }
    assert_eq!(generator.calls(), 0);
}

#[test]
fn generation_dependency_failure_is_reported() {
    let retrieval = FixedRetrieval::new(vec![candidate("acceptance-doc.md", 2, "body")]);
    let generator = FailingGenerator;
    let service = AnswerService::new(&retrieval, &generator, 1000);

    assert!(matches!(
        service.answer("what?"),
        Err(AnswerError::Generation(_))
    ));
}
