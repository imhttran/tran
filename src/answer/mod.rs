//! Answer layer: prompt construction, context budgeting, answerability,
//! citation parsing/validation/repair, and page rendering.
//!
//! Ported from the Go baseline under `internal/answer/`. The answering service
//! ([`crate::service::AnswerService`]) composes these modules over the single
//! retrieval implementation ([`crate::retrieval`]).

pub mod answerability;
pub mod citation;
pub mod context_budget;
pub mod prompt;
pub mod query_rewrite;
pub mod render;
pub mod service;
