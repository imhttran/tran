//! Production question-answer service.
//!
//! Composes the single retrieval implementation ([`crate::retrieval`]) with the
//! answer modules: context budgeting, the answerability gate, prompt
//! construction, generation, citation repair, and page rendering.
//!
//! The service is thin: it does not re-implement retrieval or answering rules.

use crate::answer::answerability;
use crate::answer::citation;
use crate::answer::context_budget::{self, ContextPassage};
use crate::answer::prompt;
use crate::answer::render;
use crate::generation::{GenerationError, Generator};
use crate::retrieval::Candidate;
use thiserror::Error;

/// Supplies ranked retrieval candidates for a question.
///
/// Implementations must return candidates produced by the single retrieval
/// implementation ([`crate::retrieval`]). Tests inject fixed candidates;
/// production wires the retrieval stack here.
pub trait RetrievalSource {
    /// Returns ranked candidates for `question`.
    fn retrieve(&self, question: &str) -> Result<Vec<Candidate>, String>;
}

/// A retrieval source backed by a fixed candidate list.
///
/// Used by tests and offline callers. The candidates are produced by the shared
/// retrieval stages, so this adapter does not add a second retrieval
/// implementation.
pub struct FixedRetrieval {
    candidates: Vec<Candidate>,
}

impl FixedRetrieval {
    /// Builds a fixed retrieval source from `candidates`.
    pub fn new(candidates: Vec<Candidate>) -> Self {
        Self { candidates }
    }
}

impl RetrievalSource for FixedRetrieval {
    fn retrieve(&self, _question: &str) -> Result<Vec<Candidate>, String> {
        Ok(self.candidates.clone())
    }
}

/// Errors returned by [`AnswerService::answer`].
#[derive(Debug, Error)]
pub enum AnswerError {
    /// The retrieval dependency failed.
    #[error("retrieval failed: {0}")]
    Retrieval(String),
    /// The generation dependency failed.
    #[error(transparent)]
    Generation(#[from] GenerationError),
}

/// The rendered answer returned by [`AnswerService::answer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The final answer text with valid citations and page references.
    pub text: String,
    /// Rendered page references for the cited evidence, in order.
    pub references: Vec<String>,
    /// The budgeted passages the answer actually cites, with provenance.
    /// Empty for a refusal or an uncited answer.
    pub citations: Vec<ContextPassage>,
    /// `true` when the baseline refusal was returned.
    pub refused: bool,
}

/// The production question-answer service.
pub struct AnswerService<'a, R, G> {
    retrieval: &'a R,
    generator: &'a G,
    budget: usize,
    citation_validation: bool,
}

impl<'a, R, G> AnswerService<'a, R, G>
where
    R: RetrievalSource,
    G: Generator,
{
    /// Builds the service over a retrieval source, a generator and a character
    /// budget for the context.
    pub fn new(retrieval: &'a R, generator: &'a G, budget: usize) -> Self {
        Self {
            retrieval,
            generator,
            budget,
            citation_validation: true,
        }
    }

    /// Enables or disables Go-compatible citation validation.
    ///
    /// Validation is enabled by [`AnswerService::new`]. When enabled, an answer
    /// with a missing or invalid citation takes the single bounded repair
    /// re-prompt and any residual invalid citation is stripped. When disabled,
    /// the raw model answer is returned unchanged, matching the Go baseline's
    /// default (`RAG_CITATION_VALIDATION=false`).
    pub fn with_citation_validation(mut self, enabled: bool) -> Self {
        self.citation_validation = enabled;
        self
    }

    /// Answers `question` with cited evidence, or returns the baseline refusal.
    ///
    /// The flow: retrieve candidates -> budget into passages -> answerability
    /// gate -> build prompt (instructions separated from untrusted text) ->
    /// generate -> repair citations -> render page references. Insufficient
    /// evidence returns the refusal without a generation call.
    pub fn answer(&self, question: &str) -> Result<Answer, AnswerError> {
        let candidates = self
            .retrieval
            .retrieve(question)
            .map_err(AnswerError::Retrieval)?;
        let passages = context_budget::budget(&candidates, self.budget);

        if !answerability::is_answerable(&passages) {
            return Ok(Answer {
                text: answerability::refusal(),
                references: Vec::new(),
                citations: Vec::new(),
                refused: true,
            });
        }

        let prompt_text = prompt::build(question, &passages);
        let raw = self.generator.generate(&prompt_text)?;
        let final_text = if self.citation_validation {
            self.validate_and_repair(question, &raw, &passages)
        } else {
            raw
        };
        let references = self.references(&final_text, &passages);
        let citations = self.cited_passages(&final_text, &passages);
        Ok(Answer {
            text: final_text,
            references,
            citations,
            refused: false,
        })
    }

    /// Applies the Go baseline's citation policy: an answer is clean only when
    /// it makes at least one citation and every citation resolves. Otherwise
    /// one repair re-prompt is issued; if it fails or returns nothing usable,
    /// the original answer is stripped instead so the caller still gets a
    /// usable answer. Residual invalid citations are always stripped, so an
    /// invalid citation is never returned as if valid.
    ///
    /// The initial generation error is returned to the caller by [`Self::answer`].
    /// A repair-call error is deliberately non-fatal, matching the Go baseline.
    fn validate_and_repair(
        &self,
        question: &str,
        raw: &str,
        passages: &[ContextPassage],
    ) -> String {
        if citation::is_clean(raw, passages) {
            return raw.to_string();
        }

        let repair_prompt = prompt::repair_build(question, passages, raw);
        match self.generator.generate(&repair_prompt) {
            Ok(repaired) if !repaired.trim().is_empty() => citation::repair(&repaired, passages),
            Ok(_) | Err(_) => citation::repair(raw, passages),
        }
    }

    fn references(&self, answer: &str, passages: &[ContextPassage]) -> Vec<String> {
        let mut out = Vec::new();
        for cited in citation::parse(answer) {
            if let Some(passage) = passages
                .iter()
                .find(|passage| citation::matches(&cited, passage))
            {
                out.push(render::render_page(passage));
            }
        }
        out
    }

    /// Returns the cited passages in answer order, deduplicated by identity so a
    /// repeated citation does not produce a repeated source entry.
    fn cited_passages(&self, answer: &str, passages: &[ContextPassage]) -> Vec<ContextPassage> {
        let mut out: Vec<ContextPassage> = Vec::new();
        for cited in citation::parse(answer) {
            if let Some(passage) = passages
                .iter()
                .find(|passage| citation::matches(&cited, passage))
            {
                let duplicate = out.iter().any(|existing| {
                    existing.source == passage.source
                        && existing.section == passage.section
                        && existing.page == passage.page
                });
                if !duplicate {
                    out.push(passage.clone());
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Chunk;

    struct StubGenerator {
        answer: String,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl StubGenerator {
        fn new(answer: &str) -> Self {
            Self {
                answer: answer.to_string(),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Generator for StubGenerator {
        fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.answer.clone())
        }
    }

    fn candidate(page: u32, content: &str) -> Candidate {
        Candidate::new(
            Chunk {
                source: "doc.md".to_string(),
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
    fn answers_with_cited_page_references() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "the answer is 42")]);
        let generator = StubGenerator::new("It is 42 [doc.md - Intro - p.2].");
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service.answer("what?").unwrap();
        assert!(!answer.refused);
        assert!(answer.text.contains("[doc.md - Intro - p.2]"));
        assert_eq!(answer.references, vec!["doc.md - Intro - p.2".to_string()]);
        assert_eq!(generator.calls(), 1);
    }

    #[test]
    fn insufficient_evidence_refuses_without_generation() {
        let retrieval = FixedRetrieval::new(Vec::new());
        let generator = StubGenerator::new("should not be used");
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service.answer("what?").unwrap();
        assert!(answer.refused);
        assert_eq!(answer.text, answerability::REFUSAL);
        assert_eq!(generator.calls(), 0);
    }

    #[test]
    fn invalid_citations_are_dropped() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "body")]);
        let generator =
            StubGenerator::new("Fact [doc.md - Intro - p.2] bogus [doc.md - Intro - p.9].");
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service.answer("what?").unwrap();
        assert!(answer.text.contains("[doc.md - Intro - p.2]"));
        assert!(!answer.text.contains("p.9"));
        assert_eq!(
            generator.calls(),
            2,
            "the invalid citation takes one repair"
        );
    }

    enum Step {
        Answer(String),
        Fail,
    }

    struct SequenceGenerator {
        steps: std::sync::Mutex<std::collections::VecDeque<Step>>,
        prompts: std::sync::Mutex<Vec<String>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl SequenceGenerator {
        fn new(steps: Vec<Step>) -> Self {
            Self {
                steps: std::sync::Mutex::new(steps.into()),
                prompts: std::sync::Mutex::new(Vec::new()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn prompts(&self) -> Vec<String> {
            self.prompts.lock().unwrap().clone()
        }
    }

    impl Generator for SequenceGenerator {
        fn generate(&self, prompt: &str) -> Result<String, GenerationError> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.prompts.lock().unwrap().push(prompt.to_string());
            match self.steps.lock().unwrap().pop_front() {
                Some(Step::Answer(answer)) => Ok(answer),
                Some(Step::Fail) | None => Err(GenerationError::Client(
                    crate::ollama::OllamaError::Transport {
                        path: "/api/generate".to_string(),
                        message: "connection refused".to_string(),
                    },
                )),
            }
        }
    }

    #[test]
    fn uncited_answer_takes_one_bounded_repair() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "the answer is 42")]);
        let generator = SequenceGenerator::new(vec![
            Step::Answer("It is 42.".to_string()),
            Step::Answer("It is 42 [doc.md - Intro - p.2].".to_string()),
        ]);
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service.answer("what?").unwrap();
        assert_eq!(answer.text, "It is 42 [doc.md - Intro - p.2].");
        assert_eq!(answer.references, vec!["doc.md - Intro - p.2".to_string()]);
        assert_eq!(generator.calls(), 2);
        let prompts = generator.prompts();
        assert!(prompts[1].contains("YOUR PREVIOUS ANSWER:\nIt is 42."));
        assert!(prompts[1].ends_with("REPAIRED ANSWER:"));
    }

    #[test]
    fn unsuccessful_repair_is_stripped_and_not_retried() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "body")]);
        let generator = SequenceGenerator::new(vec![
            Step::Answer("Fact [doc.md - Intro - p.9].".to_string()),
            Step::Answer("Still [doc.md - Intro - p.9].".to_string()),
            Step::Answer("A third call must never happen.".to_string()),
        ]);
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service.answer("what?").unwrap();
        assert_eq!(generator.calls(), 2, "repair is bounded to one retry");
        assert!(!answer.text.contains("p.9"));
        assert!(answer.references.is_empty());
    }

    #[test]
    fn failed_repair_falls_back_to_stripping_the_original() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "body")]);
        let generator = SequenceGenerator::new(vec![
            Step::Answer("Fact [doc.md - Intro - p.9].".to_string()),
            Step::Fail,
        ]);
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service
            .answer("what?")
            .expect("a repair failure must not fail the answer");
        assert_eq!(generator.calls(), 2);
        assert!(!answer.text.contains("p.9"));
        assert!(answer.text.contains("Fact"));
    }

    #[test]
    fn clean_answer_does_not_take_a_repair() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "body")]);
        let generator = SequenceGenerator::new(vec![Step::Answer(
            "Fact [doc.md - Intro - p.2].".to_string(),
        )]);
        let service = AnswerService::new(&retrieval, &generator, 1000);

        let answer = service.answer("what?").unwrap();
        assert_eq!(generator.calls(), 1);
        assert_eq!(answer.references, vec!["doc.md - Intro - p.2".to_string()]);
    }

    #[test]
    fn disabled_validation_returns_the_raw_answer() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "body")]);
        let generator = SequenceGenerator::new(vec![Step::Answer("uncited answer".to_string())]);
        let service =
            AnswerService::new(&retrieval, &generator, 1000).with_citation_validation(false);

        let answer = service.answer("what?").unwrap();
        assert_eq!(generator.calls(), 1);
        assert_eq!(answer.text, "uncited answer");
        assert!(answer.references.is_empty());
    }

    struct FailingRetrieval;

    impl RetrievalSource for FailingRetrieval {
        fn retrieve(&self, _question: &str) -> Result<Vec<Candidate>, String> {
            Err("database unavailable".to_string())
        }
    }

    struct FailingGenerator;

    impl Generator for FailingGenerator {
        fn generate(&self, _prompt: &str) -> Result<String, GenerationError> {
            Err(GenerationError::Client(
                crate::ollama::OllamaError::Transport {
                    path: "/api/generate".to_string(),
                    message: "connection refused".to_string(),
                },
            ))
        }
    }

    #[test]
    fn retrieval_dependency_failure_is_reported() {
        let retrieval = FailingRetrieval;
        let generator = StubGenerator::new("unused");
        let service = AnswerService::new(&retrieval, &generator, 1000);
        match service.answer("what?") {
            Err(AnswerError::Retrieval(message)) => {
                assert!(message.contains("database unavailable"));
            }
            other => panic!("want a retrieval error, got {other:?}"),
        }
        assert_eq!(generator.calls(), 0);
    }

    #[test]
    fn generation_dependency_failure_is_reported() {
        let retrieval = FixedRetrieval::new(vec![candidate(2, "body")]);
        let generator = FailingGenerator;
        let service = AnswerService::new(&retrieval, &generator, 1000);
        assert!(matches!(
            service.answer("what?"),
            Err(AnswerError::Generation(_))
        ));
    }
}
