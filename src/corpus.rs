//! Validated corpus identity.
//!
//! A corpus is the security and retrieval boundary for stored documents. Every
//! write and every scoped read carries a [`Corpus`], and construction is the
//! single validation point, so an empty or whitespace-only identity can never
//! reach the database. Legacy rows with a SQL `NULL` corpus are private: a
//! bound `corpus = $1` comparison never matches `NULL`, so they are not exposed
//! automatically.

use thiserror::Error;

/// Errors raised when a corpus identity is not usable.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CorpusError {
    /// The corpus was empty or contained only whitespace.
    #[error("corpus identity must not be empty")]
    Empty,
}

/// A validated corpus identity.
///
/// Construct one with [`Corpus::new`] and pass it to every corpus-scoped
/// operation. The inner value is trimmed once, so callers cannot accidentally
/// query a different corpus by adding surrounding whitespace.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Corpus(String);

impl Corpus {
    /// The default corpus for the existing private family collection.
    pub const TRUSTED: &'static str = "trusted";

    /// Validates and returns a corpus identity.
    pub fn new(value: impl AsRef<str>) -> Result<Self, CorpusError> {
        let value = value.as_ref().trim();
        if value.is_empty() {
            return Err(CorpusError::Empty);
        }
        Ok(Self(value.to_string()))
    }

    /// Returns the normalized corpus name.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Corpus {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<Corpus> for String {
    fn from(corpus: Corpus) -> Self {
        corpus.0
    }
}

impl std::fmt::Display for Corpus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Corpus {
    type Err = CorpusError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::new(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_and_accepts_a_non_empty_identity() {
        let corpus = Corpus::new("  branch-3  ").unwrap();
        assert_eq!(corpus.as_str(), "branch-3");
    }

    #[test]
    fn rejects_empty_and_whitespace_only_identities() {
        assert_eq!(Corpus::new("").unwrap_err(), CorpusError::Empty);
        assert_eq!(Corpus::new("   ").unwrap_err(), CorpusError::Empty);
    }
}
