//! Production ingestion service seam.
//!
//! This module is the callable entry point external crates use to ingest one
//! source (PDF, Markdown, or plain text). It composes the existing
//! [`crate::pipeline::IngestPipeline`] (which runs the pre-effect guard
//! [`crate::ingest::gate_pdf`]/[`crate::ingest::gate_sections`] before any
//! embedding call or store write) over caller-supplied
//! [`crate::embedding::Embedder`], [`crate::fingerprint::FingerprintReader`]
//! and [`crate::transaction::SourceReplacer`] ports.
//!
//! The service is deliberately thin: it constructs the pipeline and delegates
//! to it. There is exactly one pipeline implementation, so the service cannot
//! duplicate the chunk/embed/replace flow.

use std::path::Path;

use crate::embedding::Embedder;
use crate::fingerprint::FingerprintReader;
use crate::pipeline::{IngestOutcome, IngestPipeline, PipelineConfig, PipelineError};
use crate::transaction::SourceReplacer;

/// The production ingest service.
///
/// Holds references to the three ports and the pipeline settings, exactly as
/// [`IngestPipeline`] expects. Callers build it once and call
/// [`IngestService::ingest_source`] per source.
pub struct IngestService<'a, E, R, S> {
    embedder: &'a E,
    reader: &'a R,
    replacer: &'a S,
    config: PipelineConfig,
}

impl<'a, E, R, S> IngestService<'a, E, R, S>
where
    E: Embedder,
    R: FingerprintReader,
    S: SourceReplacer,
{
    /// Builds the service over the three ports and the settings.
    pub fn new(embedder: &'a E, reader: &'a R, replacer: &'a S, config: PipelineConfig) -> Self {
        Self {
            embedder,
            reader,
            replacer,
            config,
        }
    }

    /// Ingests one source from disk (PDF, Markdown, or plain text).
    ///
    /// Delegates to [`IngestPipeline::ingest_path`], which selects the loader by
    /// extension and runs the pre-effect guard before any effect. A rejection
    /// returns [`IngestOutcome::Rejected`] without any embedding call or store
    /// write, so the previously stored rows are untouched.
    pub fn ingest_source(
        &self,
        path: &Path,
        source: &str,
        content_hash: &str,
    ) -> Result<IngestOutcome, PipelineError> {
        let config = PipelineConfig::new(
            self.config.language.clone(),
            self.config.chunker,
            self.config.embed_model.clone(),
            self.config.corpus.clone(),
        );
        let pipeline = IngestPipeline::new(self.embedder, self.reader, self.replacer, config);
        pipeline.ingest_path(path, source, content_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fingerprint::ChunkerConfig;
    use crate::ingest::IngestRejection;
    use crate::pipeline::RecordingEmbedder;
    use crate::storage::EMBEDDING_DIM;
    use crate::storage_pg::StoredChunkRow;
    use crate::transaction::RecordingSourceReplacer;

    /// An empty read port: every source reads as changed.
    struct NoRows;

    impl FingerprintReader for NoRows {
        type Error = String;

        fn rows_for_hash(
            &self,
            _content_hash: &str,
            _source: &str,
        ) -> Result<Vec<StoredChunkRow>, String> {
            Ok(Vec::new())
        }
    }

    fn config() -> PipelineConfig {
        PipelineConfig::new(
            "english",
            ChunkerConfig::new(50, 20),
            "nomic-embed-text",
            "trusted",
        )
    }

    fn inputs() -> std::path::PathBuf {
        std::path::PathBuf::from("fixtures/parity/inputs")
    }

    #[test]
    fn mixed_pdf_reports_affected_pages_with_zero_effects() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let service = IngestService::new(&embedder, &reader, &replacer, config());

        let outcome = service
            .ingest_source(&inputs().join("mixed-text-scan.pdf"), "mixed.pdf", "hash")
            .unwrap();

        assert_eq!(
            outcome,
            IngestOutcome::Rejected(IngestRejection::Incomplete { pages: vec![2] })
        );
        assert_eq!(embedder.calls(), 0, "no embedding call on rejection");
        assert_eq!(replacer.counts().replacements, 0);
        assert_eq!(replacer.counts().rows_written, 0);
        assert!(replacer.is_empty());
    }

    #[test]
    fn scanned_pdf_reports_loader_rejection_with_zero_effects() {
        let embedder = RecordingEmbedder::new(EMBEDDING_DIM);
        let reader = NoRows;
        let replacer = RecordingSourceReplacer::new();
        let service = IngestService::new(&embedder, &reader, &replacer, config());

        let outcome = service
            .ingest_source(&inputs().join("no-text-layer.pdf"), "scanned.pdf", "hash")
            .unwrap();

        assert!(matches!(
            outcome,
            IngestOutcome::Rejected(IngestRejection::Loader(_))
        ));
        assert_eq!(embedder.calls(), 0);
        assert_eq!(replacer.counts().replacements, 0);
        assert_eq!(replacer.counts().rows_written, 0);
        assert!(replacer.is_empty());
    }
}
