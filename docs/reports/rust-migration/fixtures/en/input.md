# Baseline fixture — English retrieval

This document is a frozen input for RUST-001 fixture `en-retrieval`. It was chosen
to exercise Markdown heading-based section parsing, chunking with overlap, and the
English FTS configuration.

## Installation

Install the CLI with the package manager. The installer verifies checksums before
writing any files. Configuration is read from the environment.

## Retrieval

Retrieval combines vector similarity with full-text search. The two ranked lists are
fused with reciprocal rank fusion, then deduplicated and expanded per section. A
context budget caps the number of bytes placed into the prompt.

## Citations

Answers cite their sources in the form [source - section]. The citation validator
parses, validates, and repairs citations before rendering the final answer.

## Page provenance

A 1-based page number flows from the loader through chunking, storage, retrieval,
and citation rendering for PDF sources.
