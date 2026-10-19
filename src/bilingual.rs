//! Bilingual retrieval experiment harness (EVALUATION-ONLY).
//!
//! Ported from the Go `cmd/expbilingual` harness (RAG-024 RQ-006). It runs the
//! retrieval arms below over a frozen evaluation split against an
//! already-ingested database and reports per-case metrics. It does not modify
//! production retrieval: it composes the existing SQL search
//! ([`PgStore::vector_search`] / [`PgStore::keyword_search`]) and fusion
//! ([`retrieval_pg::fuse`]) and only adds the harness-owned pieces the Go
//! command defines locally (section dedup on `source|section`, the arm rules,
//! the pinned generation request, the query cache, and the metrics).
//!
//! Arms:
//!
//! * `A`  - baseline (original query)
//! * `B`  - bilingual expansion (fused)
//! * `C`  - translation (fused)
//! * `D1` - `C` + original-query rejection guard
//! * `D2` - `C` + original-query guard + baseline-first ranking
//! * `E`  - `C` + cross-language agreement (>= 2 of 3 query vector paths clear
//!   the floor)
//!
//! The frozen contract is recorded in
//! `docs/reports/rust-migration/RUST-007D-bilingual-contract.md`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::embedding::{Embedder, EmbeddingError};
use crate::ollama::{OllamaClient, OllamaError};
use crate::retrieval_pg;
use crate::storage_pg::{PgStore, PgStoreError, SectionKey, SqlHit, StoredChunkRow};

/// Default `EVAL_DATASET`.
pub const DEFAULT_DATASET: &str = "evals/retrieval-vi-en-expanded.json";
/// Default `OLLAMA_EMBED_MODEL` for the harness.
pub const DEFAULT_EMBED_MODEL: &str = "nomic-embed-text";
/// Default `OLLAMA_CHAT_MODEL` for the harness (differs from
/// [`crate::config::DEFAULT_CHAT_MODEL`]).
pub const DEFAULT_CHAT_MODEL: &str = "llama3.2:3b";
/// Default `QUERY_CACHE`.
pub const DEFAULT_QUERY_CACHE: &str = "docs/rq-006-query-cache.json";
/// Default `GEN_TIMEOUT`, in seconds.
pub const DEFAULT_GEN_TIMEOUT_SECS: i64 = 60;
/// Default `REPEATS`.
pub const DEFAULT_REPEATS: i64 = 1;
/// Arms run when `ARM` is unset or `all`.
pub const DEFAULT_ARMS: [&str; 3] = ["A", "D1", "D2"];

/// Harness-hardcoded candidate count per search path.
pub const CANDIDATE_K: usize = 4;
/// Harness-hardcoded number of fused sections expanded for evidence.
pub const FINAL_K: usize = 3;
/// Harness-hardcoded section-expansion limit.
pub const EXPAND_LIMIT: usize = 20;
/// Fusion depth used before section dedup.
pub const FUSE_ALL_K: usize = 1000;
/// Ranking length the D2 baseline-first fill stops at.
pub const BASELINE_FIRST_LEN: usize = 4;

/// The pinned decoding options, as Go prints the `decoding` map with `%v`.
pub const GEN_OPTIONS_DISPLAY: &str = "map[num_predict:64 seed:0 temperature:0]";

/// The keyword-search language. Go passes an empty `PipelineOptions.Language`,
/// so every keyword search uses the `english` configuration regardless of
/// `LANGUAGE`.
pub const KEYWORD_LANGUAGE: &str = "";

/// Errors the harness reports. Each names the failing input.
#[derive(Debug, Error)]
pub enum BilingualError {
    /// The dataset could not be read or parsed.
    #[error("dataset {path}: {message}")]
    Dataset { path: String, message: String },
    /// A database read failed.
    #[error(transparent)]
    Store(#[from] PgStoreError),
    /// Embedding a query failed.
    #[error("embed {id}: {source}")]
    Embed {
        id: String,
        #[source]
        source: EmbeddingError,
    },
    /// Generating a query variant failed.
    #[error("generate {id}/{variant}: {source}")]
    Generate {
        id: String,
        variant: String,
        #[source]
        source: OllamaError,
    },
    /// Writing harness output failed.
    #[error("write output: {0}")]
    Output(#[from] io::Error),
}

// ---------------------------------------------------------------------------
// Environment
// ---------------------------------------------------------------------------

/// Go `envOr`: the trimmed value, or `fallback` when unset or blank.
pub fn env_or<F>(lookup: &F, key: &str, fallback: &str) -> String
where
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        Some(value) if !value.trim().is_empty() => value.trim().to_string(),
        _ => fallback.to_string(),
    }
}

/// Go `envInt`: a value made only of ASCII digits and at least 1, otherwise
/// `fallback` (blank, any non-digit character including a sign, or zero).
pub fn env_int<F>(lookup: &F, key: &str, fallback: i64) -> i64
where
    F: Fn(&str) -> Option<String>,
{
    let value = match lookup(key) {
        Some(value) => value.trim().to_string(),
        None => return fallback,
    };
    if value.is_empty() {
        return fallback;
    }
    let mut n: i64 = 0;
    for ch in value.chars() {
        if !ch.is_ascii_digit() {
            return fallback;
        }
        // Go's int arithmetic wraps on overflow; so does this.
        n = n.wrapping_mul(10).wrapping_add(i64::from(ch as u8 - b'0'));
    }
    if n < 1 {
        return fallback;
    }
    n
}

/// Expands the normalized `ARM` value: `ALL` runs [`DEFAULT_ARMS`]; anything
/// else is split on commas without trimming the elements, as Go does.
pub fn expand_arms(arm: &str) -> Vec<String> {
    if arm == "ALL" {
        DEFAULT_ARMS.iter().map(|arm| arm.to_string()).collect()
    } else {
        arm.split(',').map(str::to_string).collect()
    }
}

/// Harness settings read from the environment.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// `ARM`, trimmed and uppercased (as printed in the header).
    pub arm: String,
    /// The arms to run, in order.
    pub arms: Vec<String>,
    pub dataset: String,
    pub embed_model: String,
    pub chat_model: String,
    pub cache_path: String,
    pub repeats: i64,
    pub diag: bool,
    pub gen_timeout: Duration,
}

impl Settings {
    /// Reads the harness settings through `lookup`.
    pub fn from_lookup<F>(lookup: F) -> Settings
    where
        F: Fn(&str) -> Option<String>,
    {
        let arm = env_or(&lookup, "ARM", "all").to_uppercase();
        let arms = expand_arms(&arm);
        let gen_timeout = env_int(&lookup, "GEN_TIMEOUT", DEFAULT_GEN_TIMEOUT_SECS);
        Settings {
            arm,
            arms,
            dataset: env_or(&lookup, "EVAL_DATASET", DEFAULT_DATASET),
            embed_model: env_or(&lookup, "OLLAMA_EMBED_MODEL", DEFAULT_EMBED_MODEL),
            chat_model: env_or(&lookup, "OLLAMA_CHAT_MODEL", DEFAULT_CHAT_MODEL),
            cache_path: env_or(&lookup, "QUERY_CACHE", DEFAULT_QUERY_CACHE),
            repeats: env_int(&lookup, "REPEATS", DEFAULT_REPEATS),
            diag: env_or(&lookup, "DIAG", "") == "1",
            gen_timeout: Duration::from_secs(u64::try_from(gen_timeout).unwrap_or(0)),
        }
    }

    /// Reads the harness settings from the process environment.
    pub fn from_env() -> Settings {
        Settings::from_lookup(|key| std::env::var(key).ok())
    }

    /// The `HEADER` line, without the trailing newline.
    pub fn header_line(&self, floor: f64) -> String {
        format!(
            "HEADER arm={} dataset={} embed={} chat={} floor={:.2} repeats={} gen_options={}",
            self.arm,
            self.dataset,
            self.embed_model,
            self.chat_model,
            floor,
            self.repeats,
            GEN_OPTIONS_DISPLAY
        )
    }
}

// ---------------------------------------------------------------------------
// Dataset
// ---------------------------------------------------------------------------

/// An expected source/section with its evidence strings.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct ExpectedDoc {
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub section: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub evidence: Vec<String>,
}

/// One bilingual evaluation case. An empty `expected` marks it unanswerable.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct BilingualCase {
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub question: String,
    #[serde(default, deserialize_with = "null_as_default")]
    pub expected: Vec<ExpectedDoc>,
}

impl BilingualCase {
    /// Whether the case expects a rejection.
    pub fn unanswerable(&self) -> bool {
        self.expected.is_empty()
    }
}

fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Loads the dataset at `path`, naming the path on any failure.
pub fn load_cases(path: &Path) -> Result<Vec<BilingualCase>, BilingualError> {
    let dataset_error = |message: String| BilingualError::Dataset {
        path: path.display().to_string(),
        message,
    };
    let raw = std::fs::read(path).map_err(|err| dataset_error(err.to_string()))?;
    serde_json::from_slice(&raw).map_err(|err| dataset_error(err.to_string()))
}

// ---------------------------------------------------------------------------
// Prompts and pinned generation
// ---------------------------------------------------------------------------

/// The Go `expansionPrompt`.
pub fn expansion_prompt(question: &str) -> String {
    format!(
        "Rewrite the user's question into a short bilingual search query that contains key terms \
         in BOTH English and Vietnamese, so it retrieves matching passages in either language. \
         Return only the query, no quotes or explanation.\n\nQUESTION:\n{question}\n\nQUERY:"
    )
}

/// The Go `translationPrompt`.
pub fn translation_prompt(question: &str, language: &str) -> String {
    format!(
        "Translate the following question into {language}. \
         Return only the translation, with no quotes, labels, or explanation.\n\nQUESTION:\n{question}\n\nTRANSLATION:"
    )
}

/// The Ollama generation endpoint.
pub const GENERATE_PATH: &str = "/api/generate";

/// The pinned decoding options. Fields are declared in key order so the encoded
/// object matches Go's sorted-map encoding byte for byte.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PinnedOptions {
    pub num_predict: u32,
    pub seed: u32,
    pub temperature: u32,
}

/// The pinned options: temperature 0, seed 0, num_predict 64.
pub const PINNED_OPTIONS: PinnedOptions = PinnedOptions {
    num_predict: 64,
    seed: 0,
    temperature: 0,
};

/// The `/api/generate` request body. Fields are in sorted key order, matching
/// the Go `map[string]any` encoding; `options` is always present.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PinnedGenerateRequest {
    pub model: String,
    pub options: PinnedOptions,
    pub prompt: String,
    pub stream: bool,
}

impl PinnedGenerateRequest {
    /// Builds the pinned, non-streaming request for `model` and `prompt`.
    pub fn new(model: impl Into<String>, prompt: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            options: PINNED_OPTIONS,
            prompt: prompt.into(),
            stream: false,
        }
    }
}

#[derive(Debug, Deserialize)]
struct GenerateReply {
    #[serde(default)]
    response: String,
}

/// Produces query variants. [`OllamaQueryGenerator`] is the production
/// implementation; tests supply a scripted one.
pub trait QueryGenerator {
    /// Returns the raw model response for `prompt`.
    fn generate(&self, model: &str, prompt: &str) -> Result<String, OllamaError>;
}

/// Posts [`PinnedGenerateRequest`]s through a dedicated [`OllamaClient`] whose
/// timeout is the harness `GEN_TIMEOUT`.
pub struct OllamaQueryGenerator {
    client: OllamaClient,
}

impl OllamaQueryGenerator {
    /// Wraps `client`; build it with the `GEN_TIMEOUT` duration.
    pub fn new(client: OllamaClient) -> Self {
        Self { client }
    }
}

impl QueryGenerator for OllamaQueryGenerator {
    fn generate(&self, model: &str, prompt: &str) -> Result<String, OllamaError> {
        let reply: GenerateReply = self
            .client
            .post_json(GENERATE_PATH, &PinnedGenerateRequest::new(model, prompt))?;
        Ok(reply.response)
    }
}

// ---------------------------------------------------------------------------
// Query cache
// ---------------------------------------------------------------------------

/// Cached variants for one case.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct CacheEntry {
    #[serde(default)]
    pub question: String,
    /// `None` reproduces a Go `null` variants map.
    #[serde(default)]
    pub variants: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct CacheFile {
    #[serde(default)]
    model: String,
    #[serde(default)]
    entries: Option<BTreeMap<String, CacheEntry>>,
}

/// The query cache in the Go on-disk format. Ordered maps reproduce Go's sorted
/// map-key encoding.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryCache {
    pub model: String,
    pub entries: BTreeMap<String, CacheEntry>,
}

impl QueryCache {
    /// An empty cache for `model`.
    pub fn fresh(model: &str) -> Self {
        Self {
            model: model.to_string(),
            entries: BTreeMap::new(),
        }
    }

    /// Go `loadCache`: an unreadable or malformed file, or one without
    /// `entries`, yields a fresh cache. A non-empty cached model different from
    /// `model` prints the Go mismatch line to `out` and yields a fresh cache.
    pub fn load(path: &Path, model: &str, out: &mut dyn Write) -> io::Result<Self> {
        let Ok(data) = std::fs::read(path) else {
            return Ok(Self::fresh(model));
        };
        Self::parse(&data, model, out)
    }

    /// The parsing half of [`QueryCache::load`].
    pub fn parse(data: &[u8], model: &str, out: &mut dyn Write) -> io::Result<Self> {
        let loaded = match serde_json::from_slice::<CacheFile>(data) {
            Ok(CacheFile {
                model: cached,
                entries: Some(entries),
            }) => (cached, entries),
            _ => return Ok(Self::fresh(model)),
        };
        if !loaded.0.is_empty() && loaded.0 != model {
            writeln!(
                out,
                "CACHE model mismatch (cached={} requested={}); starting fresh",
                loaded.0, model
            )?;
            return Ok(Self::fresh(model));
        }
        Ok(Self {
            model: loaded.0,
            entries: loaded.1,
        })
    }

    /// A cache hit: the entry's question must equal `question` and the variant
    /// must be non-blank after trimming. The stored text is returned untrimmed.
    pub fn lookup(&self, id: &str, question: &str, variant: &str) -> Option<&str> {
        let entry = self.entries.get(id)?;
        if entry.question != question {
            return None;
        }
        let value = entry.variants.as_ref()?.get(variant)?;
        (!value.trim().is_empty()).then_some(value.as_str())
    }

    /// Records a generated variant, replacing an entry whose question differs.
    pub fn store(&mut self, id: &str, question: &str, variant: &str, text: &str) {
        let entry = self.entries.entry(id.to_string()).or_default();
        if entry.question != question || entry.variants.is_none() {
            if entry.question != question {
                entry.variants = None;
            }
            entry.question = question.to_string();
            entry.variants.get_or_insert_with(BTreeMap::new);
        }
        entry
            .variants
            .get_or_insert_with(BTreeMap::new)
            .insert(variant.to_string(), text.to_string());
    }

    /// Encodes the cache as Go `json.MarshalIndent(cache, "", "  ")` with
    /// `model` set to `model` (Go `saveCache`), without a trailing newline.
    pub fn to_go_json(&self, model: &str) -> String {
        let mut out = String::new();
        out.push_str("{\n  \"model\": ");
        out.push_str(&go_json_string(model));
        out.push_str(",\n  \"entries\": ");
        if self.entries.is_empty() {
            out.push_str("{}");
        } else {
            out.push_str("{\n");
            for (index, (id, entry)) in self.entries.iter().enumerate() {
                if index > 0 {
                    out.push_str(",\n");
                }
                let _ = write!(
                    out,
                    "    {}: {{\n      \"question\": {},\n      \"variants\": ",
                    go_json_string(id),
                    go_json_string(&entry.question)
                );
                match &entry.variants {
                    None => out.push_str("null"),
                    Some(variants) if variants.is_empty() => out.push_str("{}"),
                    Some(variants) => {
                        out.push_str("{\n");
                        for (vindex, (name, text)) in variants.iter().enumerate() {
                            if vindex > 0 {
                                out.push_str(",\n");
                            }
                            let _ = write!(
                                out,
                                "        {}: {}",
                                go_json_string(name),
                                go_json_string(text)
                            );
                        }
                        out.push_str("\n      }");
                    }
                }
                out.push_str("\n    }");
            }
            out.push_str("\n  }");
        }
        out.push_str("\n}");
        out
    }

    /// Go `saveCache`: writes [`QueryCache::to_go_json`] to `path` with the
    /// model set to `model`.
    pub fn save(&mut self, path: &Path, model: &str) -> io::Result<()> {
        self.model = model.to_string();
        std::fs::write(path, self.to_go_json(model))
    }
}

/// Encodes `value` as a JSON string the way Go's `encoding/json` does with HTML
/// escaping enabled: `<`, `>`, `&`, U+2028 and U+2029 are `\u` escaped, and
/// control characters use the short escapes where Go has them.
pub fn go_json_string(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '<' | '>' | '&' | '\u{00}'..='\u{1f}' => {
                let byte = ch as u8;
                out.push_str("\\u00");
                out.push(HEX[(byte >> 4) as usize] as char);
                out.push(HEX[(byte & 0xf) as usize] as char);
            }
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Returns a variant from the cache or the model, printing the Go `GEN` line.
/// The returned count is 1 only for a model call.
pub fn generate_variant(
    generator: &dyn QueryGenerator,
    model: &str,
    cache: &mut QueryCache,
    case: &BilingualCase,
    variant: &str,
    prompt: &str,
    out: &mut dyn Write,
) -> Result<(String, usize), BilingualError> {
    if let Some(text) = cache.lookup(&case.id, &case.question, variant) {
        let text = text.to_string();
        writeln!(out, "GEN id={} variant={variant} source=cache", case.id)?;
        return Ok((text, 0));
    }
    let response =
        generator
            .generate(model, prompt)
            .map_err(|source| BilingualError::Generate {
                id: case.id.clone(),
                variant: variant.to_string(),
                source,
            })?;
    let text = response.trim().to_string();
    writeln!(out, "GEN id={} variant={variant} source=model", case.id)?;
    cache.store(&case.id, &case.question, variant, &text);
    Ok((text, 1))
}

// ---------------------------------------------------------------------------
// Retrieval: search, fusion, section dedup
// ---------------------------------------------------------------------------

/// The SQL reads the harness needs. [`CorpusSearcher`] runs them against
/// [`PgStore`]; tests supply fixed hit lists.
pub trait SqlSearcher {
    fn vector_search(&self, vector: &[f64], top_k: usize) -> Result<Vec<SqlHit>, PgStoreError>;
    fn keyword_search(&self, query: &str, top_k: usize) -> Result<Vec<SqlHit>, PgStoreError>;
    fn section_chunks(
        &self,
        keys: &[SectionKey],
        limit: usize,
    ) -> Result<Vec<StoredChunkRow>, PgStoreError>;
}

/// Production [`SqlSearcher`]: corpus-scoped [`PgStore`] reads with the
/// `english` FTS configuration for every keyword search.
pub struct CorpusSearcher<'a> {
    store: &'a PgStore,
    corpus: String,
}

impl<'a> CorpusSearcher<'a> {
    pub fn new(store: &'a PgStore, corpus: impl Into<String>) -> Self {
        Self {
            store,
            corpus: corpus.into(),
        }
    }
}

impl SqlSearcher for CorpusSearcher<'_> {
    fn vector_search(&self, vector: &[f64], top_k: usize) -> Result<Vec<SqlHit>, PgStoreError> {
        self.store.vector_search(&self.corpus, vector, top_k)
    }

    fn keyword_search(&self, query: &str, top_k: usize) -> Result<Vec<SqlHit>, PgStoreError> {
        self.store
            .keyword_search(&self.corpus, query, top_k, KEYWORD_LANGUAGE)
    }

    fn section_chunks(
        &self,
        keys: &[SectionKey],
        limit: usize,
    ) -> Result<Vec<StoredChunkRow>, PgStoreError> {
        self.store.section_chunks(&self.corpus, keys, limit)
    }
}

/// The vector (floored) and keyword rankings of one query.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct QueryRankings {
    pub vector: Vec<SqlHit>,
    pub keyword: Vec<SqlHit>,
}

/// Rankings of the three generated variants.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct VariantRankings {
    pub expansion: QueryRankings,
    pub vietnamese: QueryRankings,
    pub english: QueryRankings,
}

/// Go `searchOne` at the harness candidate depth: vector search with the
/// similarity floor applied to vector hits only, and unfloored keyword search.
pub fn search_one(
    searcher: &dyn SqlSearcher,
    query: &str,
    vector: &[f64],
    min_similarity: f64,
) -> Result<QueryRankings, PgStoreError> {
    let vector_hits = searcher
        .vector_search(vector, CANDIDATE_K)?
        .into_iter()
        .filter(|hit| hit.similarity >= min_similarity)
        .collect();
    let keyword_hits = searcher.keyword_search(query, CANDIDATE_K)?;
    Ok(QueryRankings {
        vector: vector_hits,
        keyword: keyword_hits,
    })
}

/// A fused, section-deduplicated result.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedSection {
    pub source: String,
    pub section: String,
    pub chunk_index: u32,
    /// Maximum cosine similarity across the contributing hits (0 when only
    /// keyword paths contributed), as Go `FuseRankings` propagates it.
    pub similarity: f64,
    /// The RRF score.
    pub fusion_score: f64,
}

type ChunkKey = (String, String, u32);

fn hit_key(row: &StoredChunkRow) -> ChunkKey {
    (
        row.chunk.source.clone().unwrap_or_default(),
        row.chunk.section.clone().unwrap_or_default(),
        row.chunk.chunk_index.max(0) as u32,
    )
}

/// Fuses `rankings` with [`retrieval_pg::fuse`] at `top_k`, carries the
/// per-candidate maximum similarity across the contributing hits, and keeps the
/// first result per `source|section` (Go `DeduplicateSections`).
pub fn fuse_sections(rankings: &[Vec<SqlHit>], top_k: usize) -> Vec<RankedSection> {
    let mut similarity: HashMap<ChunkKey, f64> = HashMap::new();
    for hit in rankings.iter().flatten() {
        let entry = similarity
            .entry(hit_key(&hit.row))
            .or_insert(hit.similarity);
        if hit.similarity > *entry {
            *entry = hit.similarity;
        }
    }
    let fused = retrieval_pg::fuse(rankings, top_k)
        .into_iter()
        .map(|candidate| {
            let chunk = candidate.document.chunk;
            let key = (chunk.source, chunk.section, chunk.chunk_index);
            RankedSection {
                similarity: similarity.get(&key).copied().unwrap_or(0.0),
                source: key.0,
                section: key.1,
                chunk_index: key.2,
                fusion_score: candidate.score,
            }
        })
        .collect::<Vec<_>>();
    dedup_sections(fused)
}

/// Keeps the first entry for each `source|section` pair.
pub fn dedup_sections(sections: Vec<RankedSection>) -> Vec<RankedSection> {
    let mut seen = HashSet::new();
    sections
        .into_iter()
        .filter(|section| seen.insert(format!("{}|{}", section.source, section.section)))
        .collect()
}

/// The distinct section keys of `sections`, in order of first appearance.
pub fn section_keys(sections: &[RankedSection]) -> Vec<SectionKey> {
    let mut seen = HashSet::new();
    sections
        .iter()
        .filter(|section| seen.insert(format!("{}|{}", section.source, section.section)))
        .map(|section| SectionKey {
            source: section.source.clone(),
            section: section.section.clone(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Arms
// ---------------------------------------------------------------------------

/// Whether `arm` generates query variants.
pub fn arm_generates(arm: &str) -> bool {
    matches!(arm, "B" | "C" | "D1" | "D2" | "E")
}

/// The ranking lists fused for `arm`. Without variants (arm `A` or an
/// unrecognised arm) only the original rankings are used.
pub fn compose_rankings(
    arm: &str,
    original: &QueryRankings,
    variants: Option<&VariantRankings>,
) -> Vec<Vec<SqlHit>> {
    let mut rankings = vec![original.vector.clone(), original.keyword.clone()];
    let Some(variants) = variants else {
        return rankings;
    };
    match arm {
        "B" => {
            rankings.push(variants.expansion.vector.clone());
            rankings.push(variants.expansion.keyword.clone());
        }
        "C" | "D1" | "D2" | "E" => {
            rankings.push(variants.vietnamese.vector.clone());
            rankings.push(variants.vietnamese.keyword.clone());
            rankings.push(variants.english.vector.clone());
            rankings.push(variants.english.keyword.clone());
            rankings.push(variants.expansion.vector.clone());
            rankings.push(variants.expansion.keyword.clone());
        }
        _ => {}
    }
    rankings
}

/// The number of query vector paths (original, Vietnamese, English) that
/// returned a hit above the floor.
pub fn vector_paths_clear(original: &QueryRankings, variants: Option<&VariantRankings>) -> usize {
    let mut clear = usize::from(!original.vector.is_empty());
    if let Some(variants) = variants {
        clear += usize::from(!variants.vietnamese.vector.is_empty());
        clear += usize::from(!variants.english.vector.is_empty());
    }
    clear
}

/// The arm guard: D1/D2 need a non-empty original vector ranking, E needs at
/// least 2 clear vector paths, every other arm passes.
pub fn guard_ok(arm: &str, original: &QueryRankings, variants: Option<&VariantRankings>) -> bool {
    match arm {
        "D1" | "D2" => !original.vector.is_empty(),
        "E" => vector_paths_clear(original, variants) >= 2,
        _ => true,
    }
}

/// D2 baseline-first ranking: the original rankings' fused top-1 section at
/// rank 1, filled to [`BASELINE_FIRST_LEN`] from `fused_all` without repeating
/// a section. An empty baseline falls back to `fused_all`.
pub fn baseline_first(original: &QueryRankings, fused_all: &[RankedSection]) -> Vec<RankedSection> {
    let mut out = fuse_sections(&[original.vector.clone(), original.keyword.clone()], 1);
    if out.is_empty() {
        return fused_all.to_vec();
    }
    for candidate in fused_all {
        if out.len() >= BASELINE_FIRST_LEN {
            break;
        }
        let duplicate = out
            .iter()
            .any(|kept| kept.source == candidate.source && kept.section == candidate.section);
        if !duplicate {
            out.push(candidate.clone());
        }
    }
    out
}

/// The outcome of one arm for one case.
#[derive(Debug, Clone, PartialEq)]
pub struct ArmOutcome {
    pub guard_ok: bool,
    pub rejected: bool,
    pub ranking: Vec<RankedSection>,
}

/// Applies the Go arm rules: fuse at [`FUSE_ALL_K`] and dedup; D2 replaces the
/// ranking with [`baseline_first`] when the guard passes and with nothing when
/// it fails; any arm whose guard fails on an unanswerable case returns nothing.
pub fn rank_arm(
    arm: &str,
    unanswerable: bool,
    original: &QueryRankings,
    variants: Option<&VariantRankings>,
) -> ArmOutcome {
    let guard = guard_ok(arm, original, variants);
    let fused_all = fuse_sections(&compose_rankings(arm, original, variants), FUSE_ALL_K);
    let mut ranking = if arm == "D2" {
        if guard {
            baseline_first(original, &fused_all)
        } else {
            Vec::new()
        }
    } else {
        fused_all
    };
    if unanswerable && !guard {
        ranking.clear();
    }
    ArmOutcome {
        guard_ok: guard,
        rejected: unanswerable,
        ranking,
    }
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Go `recallAtK`: the share of expected sections found in the top `k`, with
/// `k` clamped to the ranking length; 0 when nothing is expected.
pub fn recall_at_k(expected: &[ExpectedDoc], ranking: &[RankedSection], k: usize) -> f64 {
    if expected.is_empty() {
        return 0.0;
    }
    let top = &ranking[..k.min(ranking.len())];
    let found = expected
        .iter()
        .filter(|doc| {
            top.iter()
                .any(|hit| hit.source == doc.source && hit.section == doc.section)
        })
        .count();
    found as f64 / expected.len() as f64
}

/// Go `evidenceRecall`: the share of expected evidence strings found
/// (case-insensitively) in an expanded chunk of the same source/section; 0
/// when there is no evidence.
pub fn evidence_recall(expected: &[ExpectedDoc], chunks: &[StoredChunkRow]) -> f64 {
    let mut total = 0usize;
    let mut found = 0usize;
    for doc in expected {
        for evidence in &doc.evidence {
            total += 1;
            let needle = evidence.to_lowercase();
            let hit = chunks.iter().any(|row| {
                row.chunk.source.as_deref().unwrap_or_default() == doc.source
                    && row.chunk.section.as_deref().unwrap_or_default() == doc.section
                    && row.chunk.content.to_lowercase().contains(&needle)
            });
            if hit {
                found += 1;
            }
        }
    }
    if total == 0 {
        return 0.0;
    }
    found as f64 / total as f64
}

/// The rejection label: `n/a` for an answerable case, `reject` when an
/// unanswerable case returns nothing, `FALSE_POSITIVE` otherwise.
pub fn rejection_label(unanswerable: bool, ranking: &[RankedSection]) -> &'static str {
    match (unanswerable, ranking.is_empty()) {
        (false, _) => "n/a",
        (true, true) => "reject",
        (true, false) => "FALSE_POSITIVE",
    }
}

// ---------------------------------------------------------------------------
// Case runner
// ---------------------------------------------------------------------------

/// The collaborators one harness run needs.
pub struct Harness<'a> {
    pub searcher: &'a dyn SqlSearcher,
    pub embedder: &'a dyn Embedder,
    pub generator: &'a dyn QueryGenerator,
    pub chat_model: &'a str,
    pub min_similarity: f64,
    pub diag: bool,
}

impl Harness<'_> {
    /// Runs every arm, repeat, and case in the Go loop order, returning the
    /// number of model-generated queries.
    pub fn run_all(
        &self,
        arms: &[String],
        repeats: i64,
        cases: &[BilingualCase],
        cache: &mut QueryCache,
        out: &mut dyn Write,
    ) -> Result<usize, BilingualError> {
        let mut generated = 0;
        for arm in arms {
            for rep in 1..=repeats {
                for case in cases {
                    generated += self.run_case(arm, rep, case, cache, out)?;
                }
            }
        }
        Ok(generated)
    }

    fn embed(&self, id: &str, text: &str) -> Result<Vec<f64>, BilingualError> {
        self.embedder
            .embed(text)
            .map_err(|source| BilingualError::Embed {
                id: id.to_string(),
                source,
            })
    }

    /// Runs one case for one arm and repeat, printing its `GEN`, `DIAG` and
    /// `CASE` lines. Returns the number of model-generated queries.
    pub fn run_case(
        &self,
        arm: &str,
        rep: i64,
        case: &BilingualCase,
        cache: &mut QueryCache,
        out: &mut dyn Write,
    ) -> Result<usize, BilingualError> {
        let floor = self.min_similarity;
        let query_vector = self.embed(&case.id, &case.question)?;
        let original = search_one(self.searcher, &case.question, &query_vector, floor)?;

        let mut generated = 0;
        let variants = if arm_generates(arm) {
            let (expansion, n) = generate_variant(
                self.generator,
                self.chat_model,
                cache,
                case,
                "exp",
                &expansion_prompt(&case.question),
                out,
            )?;
            generated += n;
            let expansion_vector = self.embed(&case.id, &expansion)?;
            let expansion = search_one(self.searcher, expansion.trim(), &expansion_vector, floor)?;

            let (vietnamese, n) = generate_variant(
                self.generator,
                self.chat_model,
                cache,
                case,
                "vi",
                &translation_prompt(&case.question, "Vietnamese"),
                out,
            )?;
            generated += n;
            let (english, n) = generate_variant(
                self.generator,
                self.chat_model,
                cache,
                case,
                "en",
                &translation_prompt(&case.question, "English"),
                out,
            )?;
            generated += n;
            let vietnamese_vector = self.embed(&case.id, &vietnamese)?;
            let english_vector = self.embed(&case.id, &english)?;
            Some(VariantRankings {
                expansion,
                vietnamese: search_one(
                    self.searcher,
                    vietnamese.trim(),
                    &vietnamese_vector,
                    floor,
                )?,
                english: search_one(self.searcher, english.trim(), &english_vector, floor)?,
            })
        } else {
            None
        };

        let outcome = rank_arm(arm, case.unanswerable(), &original, variants.as_ref());

        if self.diag {
            writeln!(
                out,
                "DIAG arm={arm} rep={rep} id={} kind={} guardOK={} candidates={} rejected={}",
                case.id,
                case.kind,
                outcome.guard_ok,
                outcome.ranking.len(),
                outcome.rejected
            )?;
            for (index, section) in outcome.ranking.iter().enumerate() {
                writeln!(
                    out,
                    "DIAG   #{} sim={:.4} {} / {}",
                    index + 1,
                    section.similarity,
                    section.source,
                    section.section
                )?;
            }
        }

        let r1 = recall_at_k(&case.expected, &outcome.ranking, 1);
        let r4 = recall_at_k(&case.expected, &outcome.ranking, 4);
        let label = rejection_label(outcome.rejected, &outcome.ranking);

        // Evidence follows cmd/eval FinalK semantics: expand only the FINAL_K
        // fused sections.
        let evidence = if outcome.rejected {
            0.0
        } else {
            let final_sections = &outcome.ranking[..FINAL_K.min(outcome.ranking.len())];
            let chunks = self
                .searcher
                .section_chunks(&section_keys(final_sections), EXPAND_LIMIT)?;
            evidence_recall(&case.expected, &chunks)
        };

        writeln!(
            out,
            "CASE arm={arm} rep={rep} id={} kind={} r1={r1:.2} r4={r4:.2} evidence={evidence:.2} {label}",
            case.id, case.kind
        )?;
        Ok(generated)
    }
}

/// Renders a duration rounded to the millisecond the way Go prints
/// `time.Since(start).Round(time.Millisecond)`.
pub fn go_duration_ms(elapsed: Duration) -> String {
    let nanos = elapsed.as_nanos();
    let ms = 1_000_000u128;
    let remainder = nanos % ms;
    let rounded = if remainder * 2 < ms {
        nanos - remainder
    } else {
        nanos + ms - remainder
    };
    format_go_duration(u64::try_from(rounded).unwrap_or(u64::MAX))
}

/// Go `time.Duration.String` for a non-negative nanosecond count.
pub fn format_go_duration(nanos: u64) -> String {
    if nanos == 0 {
        return "0s".to_string();
    }
    if nanos < 1_000_000_000 {
        let (unit, scale, digits) = if nanos < 1_000 {
            return format!("{nanos}ns");
        } else if nanos < 1_000_000 {
            ("µs", 1_000u64, 3)
        } else {
            ("ms", 1_000_000u64, 6)
        };
        return format!("{}{unit}", fixed_fraction(nanos, scale, digits));
    }
    let total_seconds = nanos / 1_000_000_000;
    let hours = total_seconds / 3600;
    let minutes = (total_seconds / 60) % 60;
    let seconds = fixed_fraction(nanos % 60_000_000_000, 1_000_000_000, 9);
    let mut out = String::new();
    if hours > 0 {
        let _ = write!(out, "{hours}h");
    }
    if hours > 0 || minutes > 0 {
        let _ = write!(out, "{minutes}m");
    }
    let _ = write!(out, "{seconds}s");
    out
}

/// `value / scale` with up to `digits` fractional digits, trailing zeros and a
/// bare decimal point dropped.
fn fixed_fraction(value: u64, scale: u64, digits: usize) -> String {
    let whole = value / scale;
    let fraction = value % scale;
    if fraction == 0 {
        return whole.to_string();
    }
    let fraction = format!("{fraction:0digits$}");
    format!("{whole}.{}", fraction.trim_end_matches('0'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StoredChunk;
    use std::cell::RefCell;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |key| map.get(key).cloned()
    }

    // --- environment -------------------------------------------------------

    #[test]
    fn env_int_follows_go_fallback_rules() {
        for (value, want) in [
            ("", 60),
            ("   ", 60),
            ("abc", 60),
            ("12s", 60),
            ("0", 60),
            ("000", 60),
            ("-5", 60),
            ("+5", 60),
            ("1.5", 60),
            ("30", 30),
            (" 45 ", 45),
        ] {
            assert_eq!(
                env_int(&env(&[("GEN_TIMEOUT", value)]), "GEN_TIMEOUT", 60),
                want,
                "GEN_TIMEOUT={value:?}"
            );
        }
        assert_eq!(env_int(&env(&[]), "REPEATS", 1), 1);
        assert_eq!(env_int(&env(&[("REPEATS", "x")]), "REPEATS", 1), 1);
        assert_eq!(env_int(&env(&[("REPEATS", "-1")]), "REPEATS", 1), 1);
        assert_eq!(env_int(&env(&[("REPEATS", "3")]), "REPEATS", 1), 3);
    }

    #[test]
    fn settings_defaults_match_the_go_harness() {
        let settings = Settings::from_lookup(env(&[]));
        assert_eq!(settings.arm, "ALL");
        assert_eq!(settings.arms, vec!["A", "D1", "D2"]);
        assert_eq!(settings.dataset, DEFAULT_DATASET);
        assert_eq!(settings.embed_model, "nomic-embed-text");
        assert_eq!(settings.chat_model, "llama3.2:3b");
        assert_eq!(settings.cache_path, "docs/rq-006-query-cache.json");
        assert_eq!(settings.repeats, 1);
        assert!(!settings.diag);
        assert_eq!(settings.gen_timeout, Duration::from_secs(60));
    }

    #[test]
    fn arm_parsing_trims_uppercases_and_splits_without_element_trim() {
        let settings = Settings::from_lookup(env(&[("ARM", "  all ")]));
        assert_eq!(settings.arms, vec!["A", "D1", "D2"]);
        let settings = Settings::from_lookup(env(&[("ARM", " b,c ")]));
        assert_eq!(settings.arm, "B,C");
        assert_eq!(settings.arms, vec!["B", "C"]);
        let settings = Settings::from_lookup(env(&[("ARM", "a, d1")]));
        assert_eq!(settings.arms, vec!["A", " D1"]);
        let settings = Settings::from_lookup(env(&[("ARM", "e")]));
        assert_eq!(settings.arms, vec!["E"]);
        assert!(Settings::from_lookup(env(&[("DIAG", " 1 ")])).diag);
        assert!(!Settings::from_lookup(env(&[("DIAG", "true")])).diag);
    }

    #[test]
    fn header_line_matches_the_go_format() {
        let settings = Settings::from_lookup(env(&[]));
        assert_eq!(
            settings.header_line(0.6),
            "HEADER arm=ALL dataset=evals/retrieval-vi-en-expanded.json embed=nomic-embed-text \
             chat=llama3.2:3b floor=0.60 repeats=1 gen_options=map[num_predict:64 seed:0 temperature:0]"
        );
    }

    // --- prompts and generation -------------------------------------------

    #[test]
    fn prompts_match_the_go_strings() {
        assert_eq!(
            expansion_prompt("Q?"),
            "Rewrite the user's question into a short bilingual search query that contains key terms in BOTH English and Vietnamese, so it retrieves matching passages in either language. Return only the query, no quotes or explanation.\n\nQUESTION:\nQ?\n\nQUERY:"
        );
        assert_eq!(
            translation_prompt("Q?", "Vietnamese"),
            "Translate the following question into Vietnamese. Return only the translation, with no quotes, labels, or explanation.\n\nQUESTION:\nQ?\n\nTRANSLATION:"
        );
    }

    #[test]
    fn pinned_request_serializes_with_options() {
        let encoded = serde_json::to_string(&PinnedGenerateRequest::new("m", "p")).unwrap();
        assert_eq!(
            encoded,
            r#"{"model":"m","options":{"num_predict":64,"seed":0,"temperature":0},"prompt":"p","stream":false}"#
        );
    }

    struct Scripted {
        calls: RefCell<Vec<String>>,
    }

    impl QueryGenerator for Scripted {
        fn generate(&self, _model: &str, prompt: &str) -> Result<String, OllamaError> {
            self.calls.borrow_mut().push(prompt.to_string());
            Ok(format!("  generated-{}  ", self.calls.borrow().len()))
        }
    }

    fn case(id: &str, question: &str, expected: Vec<ExpectedDoc>) -> BilingualCase {
        BilingualCase {
            kind: "en-vi".to_string(),
            id: id.to_string(),
            question: question.to_string(),
            expected,
        }
    }

    #[test]
    fn generation_uses_the_cache_and_counts_only_model_calls() {
        let generator = Scripted {
            calls: RefCell::new(Vec::new()),
        };
        let mut cache = QueryCache::fresh("m");
        let c = case("c1", "Q?", vec![]);
        let mut out = Vec::new();
        let (text, n) =
            generate_variant(&generator, "m", &mut cache, &c, "vi", "p", &mut out).unwrap();
        assert_eq!((text.as_str(), n), ("generated-1", 1));
        let (text, n) =
            generate_variant(&generator, "m", &mut cache, &c, "vi", "p", &mut out).unwrap();
        assert_eq!((text.as_str(), n), ("generated-1", 0));
        // A changed question misses and replaces the entry.
        let changed = case("c1", "Other?", vec![]);
        let (_, n) =
            generate_variant(&generator, "m", &mut cache, &changed, "vi", "p", &mut out).unwrap();
        assert_eq!(n, 1);
        assert_eq!(cache.entries["c1"].question, "Other?");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "GEN id=c1 variant=vi source=model\nGEN id=c1 variant=vi source=cache\nGEN id=c1 variant=vi source=model\n"
        );
    }

    #[test]
    fn blank_cached_variant_is_a_miss() {
        let mut cache = QueryCache::fresh("m");
        cache.store("c1", "Q?", "vi", "   ");
        assert_eq!(cache.lookup("c1", "Q?", "vi"), None);
        cache.store("c1", "Q?", "vi", " x ");
        assert_eq!(cache.lookup("c1", "Q?", "vi"), Some(" x "));
        assert_eq!(cache.lookup("c1", "Q?", "en"), None);
        assert_eq!(cache.lookup("c1", "different", "vi"), None);
    }

    // --- cache file format ---------------------------------------------------

    #[test]
    fn repository_cache_round_trips_byte_for_byte() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/rq-006-query-cache.json");
        let original = std::fs::read_to_string(&path).unwrap();
        let mut out = Vec::new();
        let cache = QueryCache::parse(original.as_bytes(), "llama3.2:3b", &mut out).unwrap();
        assert!(out.is_empty());
        assert!(!cache.entries.is_empty());
        assert_eq!(cache.to_go_json("llama3.2:3b"), original);
    }

    #[test]
    fn cache_model_mismatch_starts_fresh_with_the_go_message() {
        let data = br#"{"model":"old","entries":{"c1":{"question":"Q","variants":{"vi":"x"}}}}"#;
        let mut out = Vec::new();
        let cache = QueryCache::parse(data, "new", &mut out).unwrap();
        assert_eq!(cache, QueryCache::fresh("new"));
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "CACHE model mismatch (cached=old requested=new); starting fresh\n"
        );
    }

    #[test]
    fn malformed_or_entryless_cache_starts_fresh() {
        let mut out = Vec::new();
        for data in [
            &b"not json"[..],
            br#"{"model":"m"}"#,
            br#"{"model":"m","entries":null}"#,
        ] {
            assert_eq!(
                QueryCache::parse(data, "m", &mut out).unwrap(),
                QueryCache::fresh("m")
            );
        }
        let cache = QueryCache::parse(br#"{"entries":{}}"#, "m", &mut out).unwrap();
        assert_eq!(cache.model, "");
        assert!(out.is_empty());
        assert_eq!(
            QueryCache::load(Path::new("/nonexistent/cache.json"), "m", &mut out).unwrap(),
            QueryCache::fresh("m")
        );
    }

    #[test]
    fn cache_encoding_matches_go_marshal_indent() {
        let mut cache = QueryCache::fresh("m");
        assert_eq!(
            cache.to_go_json("m"),
            "{\n  \"model\": \"m\",\n  \"entries\": {}\n}"
        );
        cache.store("b", "Q <&> \"x\"\n", "vi", "tiếng\u{2028}\u{1}");
        cache.store("a", "A", "en", "e");
        cache.store("a", "A", "exp", "x");
        assert_eq!(
            cache.to_go_json("m2"),
            "{\n  \"model\": \"m2\",\n  \"entries\": {\n    \"a\": {\n      \"question\": \"A\",\n      \"variants\": {\n        \"en\": \"e\",\n        \"exp\": \"x\"\n      }\n    },\n    \"b\": {\n      \"question\": \"Q \\u003c\\u0026\\u003e \\\"x\\\"\\n\",\n      \"variants\": {\n        \"vi\": \"tiếng\\u2028\\u0001\"\n      }\n    }\n  }\n}"
        );
    }

    #[test]
    fn cache_save_writes_the_go_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache.json");
        let mut cache = QueryCache::fresh("");
        cache.store("c", "Q", "vi", "v");
        cache.save(&path, "m").unwrap();
        let mut out = Vec::new();
        let loaded = QueryCache::load(&path, "m", &mut out).unwrap();
        assert_eq!(loaded.model, "m");
        assert_eq!(loaded.lookup("c", "Q", "vi"), Some("v"));
    }

    // --- dataset ---------------------------------------------------------------

    #[test]
    fn dataset_loads_kind_and_id_and_reports_the_path() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("evals/retrieval-vi-en-expanded.json");
        let cases = load_cases(&path).unwrap();
        assert_eq!(cases.len(), 39);
        assert_eq!(cases[0].id, "eval-en-en-01");
        assert_eq!(cases[0].kind, "en-en");
        assert!(cases.iter().any(BilingualCase::unanswerable));

        let err = load_cases(Path::new("/nonexistent/dataset.json")).unwrap_err();
        assert!(err.to_string().contains("/nonexistent/dataset.json"));
        let dir = tempfile::tempdir().unwrap();
        let bad = dir.path().join("bad.json");
        std::fs::write(&bad, "{").unwrap();
        assert!(load_cases(&bad)
            .unwrap_err()
            .to_string()
            .contains(&bad.display().to_string()));
    }

    // --- ranking -------------------------------------------------------------

    fn hit(id: i64, source: &str, section: &str, index: i32, similarity: f64) -> SqlHit {
        SqlHit {
            row: StoredChunkRow {
                id,
                chunk: StoredChunk {
                    content: format!("{section} content {index}"),
                    source: Some(source.to_string()),
                    section: Some(section.to_string()),
                    chunk_index: index,
                    language: None,
                    page: None,
                    corpus: Some("c".to_string()),
                    content_hash: String::new(),
                    embed_model: String::new(),
                    embedding_dim: 0,
                    chunker_config: String::new(),
                    ingested_at: String::new(),
                    embedding: Vec::new(),
                },
            },
            similarity,
            keyword_score: if similarity == 0.0 { 1.0 } else { 0.0 },
        }
    }

    fn sections(ranking: &[RankedSection]) -> Vec<String> {
        ranking
            .iter()
            .map(|s| format!("{}/{}", s.source, s.section))
            .collect()
    }

    fn rankings(vector: Vec<SqlHit>, keyword: Vec<SqlHit>) -> QueryRankings {
        QueryRankings { vector, keyword }
    }

    fn fixture() -> (QueryRankings, VariantRankings) {
        let original = rankings(
            vec![hit(1, "a", "One", 0, 0.9), hit(2, "a", "Two", 0, 0.8)],
            vec![hit(3, "b", "Three", 0, 0.0)],
        );
        let variants = VariantRankings {
            expansion: rankings(vec![hit(4, "c", "Four", 0, 0.7)], vec![]),
            vietnamese: rankings(
                vec![hit(5, "v", "Five", 0, 0.85), hit(2, "a", "Two", 0, 0.95)],
                vec![hit(5, "v", "Five", 0, 0.0)],
            ),
            english: rankings(vec![hit(2, "a", "Two", 0, 0.75)], vec![]),
        };
        (original, variants)
    }

    #[test]
    fn arm_composition_follows_the_go_order() {
        let (original, variants) = fixture();
        let ids = |lists: Vec<Vec<SqlHit>>| -> Vec<Vec<i64>> {
            lists
                .iter()
                .map(|l| l.iter().map(|h| h.row.id).collect())
                .collect()
        };
        assert_eq!(
            ids(compose_rankings("A", &original, None)),
            vec![vec![1, 2], vec![3]]
        );
        assert_eq!(
            ids(compose_rankings("B", &original, Some(&variants))),
            vec![vec![1, 2], vec![3], vec![4], vec![]]
        );
        for arm in ["C", "D1", "D2", "E"] {
            assert_eq!(
                ids(compose_rankings(arm, &original, Some(&variants))),
                vec![
                    vec![1, 2],
                    vec![3],
                    vec![5, 2],
                    vec![5],
                    vec![2],
                    vec![],
                    vec![4],
                    vec![]
                ],
                "arm {arm}"
            );
        }
        // An unrecognised arm falls through to the baseline rankings.
        assert_eq!(
            ids(compose_rankings(" D1", &original, Some(&variants))),
            vec![vec![1, 2], vec![3]]
        );
    }

    #[test]
    fn fused_rankings_per_arm() {
        let (original, variants) = fixture();
        let a = rank_arm("A", false, &original, None);
        assert_eq!(sections(&a.ranking), vec!["a/One", "b/Three", "a/Two"]);
        let b = rank_arm("B", false, &original, Some(&variants));
        assert_eq!(
            sections(&b.ranking),
            vec!["a/One", "b/Three", "c/Four", "a/Two"]
        );
        let c = rank_arm("C", false, &original, Some(&variants));
        assert_eq!(
            sections(&c.ranking),
            vec!["a/Two", "v/Five", "a/One", "b/Three", "c/Four"]
        );
        for arm in ["D1", "E"] {
            assert_eq!(
                rank_arm(arm, false, &original, Some(&variants)).ranking,
                c.ranking
            );
        }
        // Similarity is the max across contributing paths; keyword-only is 0.
        assert_eq!(c.ranking[0].similarity, 0.95);
        assert_eq!(c.ranking[3].similarity, 0.0);
    }

    #[test]
    fn d2_pins_the_baseline_top1_and_fills_to_four() {
        let (original, variants) = fixture();
        let d2 = rank_arm("D2", false, &original, Some(&variants));
        assert_eq!(
            sections(&d2.ranking),
            vec!["a/One", "a/Two", "v/Five", "b/Three"]
        );

        // An empty baseline falls back to the fused list.
        let fused = rank_arm("C", false, &original, Some(&variants)).ranking;
        assert_eq!(baseline_first(&QueryRankings::default(), &fused), fused);
    }

    #[test]
    fn guards_follow_the_go_rules() {
        let (mut original, variants) = fixture();
        original.vector.clear();
        // D2 returns nothing whenever its guard fails, even when answerable.
        for unanswerable in [false, true] {
            let d2 = rank_arm("D2", unanswerable, &original, Some(&variants));
            assert!(!d2.guard_ok);
            assert!(d2.ranking.is_empty());
        }
        // D1 rejects unanswerable cases; an answerable case keeps the fused list.
        let d1 = rank_arm("D1", true, &original, Some(&variants));
        assert!(d1.ranking.is_empty());
        assert_eq!(rejection_label(true, &d1.ranking), "reject");
        assert!(!rank_arm("D1", false, &original, Some(&variants))
            .ranking
            .is_empty());

        // E: two of three vector paths (vi, en) still clear.
        let e = rank_arm("E", true, &original, Some(&variants));
        assert!(e.guard_ok);
        assert_eq!(rejection_label(true, &e.ranking), "FALSE_POSITIVE");
        let mut weak = variants.clone();
        weak.english.vector.clear();
        let e = rank_arm("E", true, &original, Some(&weak));
        assert!(!e.guard_ok);
        assert!(e.ranking.is_empty());
        assert_eq!(vector_paths_clear(&original, Some(&weak)), 1);

        // Arm A never guards.
        assert!(rank_arm("A", true, &original, None).guard_ok);
    }

    #[test]
    fn similarity_floor_applies_to_vector_hits_only() {
        struct Fixed;
        impl SqlSearcher for Fixed {
            fn vector_search(&self, _: &[f64], _: usize) -> Result<Vec<SqlHit>, PgStoreError> {
                Ok(vec![hit(1, "a", "One", 0, 0.7), hit(2, "a", "Two", 0, 0.5)])
            }
            fn keyword_search(&self, _: &str, _: usize) -> Result<Vec<SqlHit>, PgStoreError> {
                Ok(vec![hit(3, "b", "Three", 0, 0.0)])
            }
            fn section_chunks(
                &self,
                _: &[SectionKey],
                _: usize,
            ) -> Result<Vec<StoredChunkRow>, PgStoreError> {
                Ok(Vec::new())
            }
        }
        let rankings = search_one(&Fixed, "q", &[1.0], 0.6).unwrap();
        assert_eq!(rankings.vector.len(), 1);
        assert_eq!(rankings.keyword.len(), 1);
    }

    #[test]
    fn section_dedup_keeps_the_first_chunk_of_a_section() {
        let fused = fuse_sections(
            &[vec![
                hit(1, "a", "One", 2, 0.9),
                hit(2, "a", "One", 0, 0.8),
                hit(3, "a", "Two", 1, 0.7),
            ]],
            FUSE_ALL_K,
        );
        assert_eq!(sections(&fused), vec!["a/One", "a/Two"]);
        assert_eq!(fused[0].chunk_index, 2);
        // The production chunk-level dedup still distinguishes chunk indexes.
        let candidates = retrieval_pg::fuse(
            &[vec![hit(1, "a", "One", 2, 0.9), hit(2, "a", "One", 0, 0.8)]],
            10,
        );
        assert_eq!(crate::retrieval::dedup::dedup(candidates).len(), 2);
    }

    // --- metrics -------------------------------------------------------------

    fn expected(source: &str, section: &str, evidence: &[&str]) -> ExpectedDoc {
        ExpectedDoc {
            source: source.to_string(),
            section: section.to_string(),
            evidence: evidence.iter().map(|e| e.to_string()).collect(),
        }
    }

    #[test]
    fn recall_matches_go_recall_at_k() {
        let (original, variants) = fixture();
        let ranking = rank_arm("C", false, &original, Some(&variants)).ranking;
        let want = vec![expected("a", "Two", &[]), expected("a", "One", &[])];
        assert_eq!(recall_at_k(&want, &ranking, 1), 0.5);
        assert_eq!(recall_at_k(&want, &ranking, 4), 1.0);
        assert_eq!(recall_at_k(&[], &ranking, 4), 0.0);
        // k beyond the ranking length is clamped.
        assert_eq!(recall_at_k(&want, &ranking[..1], 4), 0.5);
        assert_eq!(recall_at_k(&want, &[], 4), 0.0);
    }

    #[test]
    fn evidence_recall_matches_go() {
        let rows: Vec<StoredChunkRow> = vec![
            hit(1, "a", "One", 0, 0.0).row,
            hit(2, "a", "Two", 1, 0.0).row,
        ];
        // Contents: "One content 0", "Two content 1".
        let want = vec![
            expected("a", "One", &["ONE CONTENT", "missing"]),
            expected("a", "Two", &["content 1"]),
            expected("a", "Three", &["content"]),
        ];
        assert_eq!(evidence_recall(&want, &rows), 0.5);
        assert_eq!(evidence_recall(&[expected("a", "One", &[])], &rows), 0.0);
        assert_eq!(evidence_recall(&[], &rows), 0.0);
    }

    // --- case runner -----------------------------------------------------------

    struct FixedEmbedder;
    impl Embedder for FixedEmbedder {
        fn embed(&self, _: &str) -> Result<Vec<f64>, EmbeddingError> {
            Ok(vec![1.0])
        }
    }

    struct FixedSearcher;
    impl SqlSearcher for FixedSearcher {
        fn vector_search(&self, _: &[f64], _: usize) -> Result<Vec<SqlHit>, PgStoreError> {
            Ok(vec![hit(1, "a", "One", 0, 0.9)])
        }
        fn keyword_search(&self, _: &str, _: usize) -> Result<Vec<SqlHit>, PgStoreError> {
            Ok(vec![hit(2, "a", "Two", 0, 0.0)])
        }
        fn section_chunks(
            &self,
            keys: &[SectionKey],
            limit: usize,
        ) -> Result<Vec<StoredChunkRow>, PgStoreError> {
            assert_eq!(limit, EXPAND_LIMIT);
            Ok(keys
                .iter()
                .enumerate()
                .map(|(i, k)| hit(10 + i as i64, &k.source, &k.section, 0, 0.0).row)
                .collect())
        }
    }

    #[test]
    fn run_case_prints_the_go_lines() {
        let generator = Scripted {
            calls: RefCell::new(Vec::new()),
        };
        let harness = Harness {
            searcher: &FixedSearcher,
            embedder: &FixedEmbedder,
            generator: &generator,
            chat_model: "m",
            min_similarity: 0.6,
            diag: true,
        };
        let mut cache = QueryCache::fresh("m");
        let cases = vec![
            case("c1", "Q?", vec![expected("a", "Two", &["two content"])]),
            case("c2", "None?", vec![]),
        ];
        let mut out = Vec::new();
        let generated = harness
            .run_all(
                &["A".to_string(), "D2".to_string()],
                1,
                &cases,
                &mut cache,
                &mut out,
            )
            .unwrap();
        assert_eq!(generated, 6);
        let text = String::from_utf8(out).unwrap();
        assert_eq!(
            text,
            "DIAG arm=A rep=1 id=c1 kind=en-vi guardOK=true candidates=2 rejected=false\n\
             DIAG   #1 sim=0.9000 a / One\n\
             DIAG   #2 sim=0.0000 a / Two\n\
             CASE arm=A rep=1 id=c1 kind=en-vi r1=0.00 r4=1.00 evidence=1.00 n/a\n\
             DIAG arm=A rep=1 id=c2 kind=en-vi guardOK=true candidates=2 rejected=true\n\
             DIAG   #1 sim=0.9000 a / One\n\
             DIAG   #2 sim=0.0000 a / Two\n\
             CASE arm=A rep=1 id=c2 kind=en-vi r1=0.00 r4=0.00 evidence=0.00 FALSE_POSITIVE\n\
             GEN id=c1 variant=exp source=model\n\
             GEN id=c1 variant=vi source=model\n\
             GEN id=c1 variant=en source=model\n\
             DIAG arm=D2 rep=1 id=c1 kind=en-vi guardOK=true candidates=2 rejected=false\n\
             DIAG   #1 sim=0.9000 a / One\n\
             DIAG   #2 sim=0.0000 a / Two\n\
             CASE arm=D2 rep=1 id=c1 kind=en-vi r1=0.00 r4=1.00 evidence=1.00 n/a\n\
             GEN id=c2 variant=exp source=model\n\
             GEN id=c2 variant=vi source=model\n\
             GEN id=c2 variant=en source=model\n\
             DIAG arm=D2 rep=1 id=c2 kind=en-vi guardOK=true candidates=2 rejected=true\n\
             DIAG   #1 sim=0.9000 a / One\n\
             DIAG   #2 sim=0.0000 a / Two\n\
             CASE arm=D2 rep=1 id=c2 kind=en-vi r1=0.00 r4=0.00 evidence=0.00 FALSE_POSITIVE\n"
        );
        assert_eq!(generator.calls.borrow().len(), 6);
        assert_eq!(
            cache.entries["c1"].variants.as_ref().unwrap()["exp"],
            "generated-1"
        );
    }

    // --- footer ----------------------------------------------------------------

    #[test]
    fn durations_render_like_go() {
        assert_eq!(format_go_duration(0), "0s");
        assert_eq!(go_duration_ms(Duration::from_micros(400)), "0s");
        assert_eq!(go_duration_ms(Duration::from_micros(1_500)), "2ms");
        assert_eq!(go_duration_ms(Duration::from_millis(512)), "512ms");
        assert_eq!(go_duration_ms(Duration::from_millis(1_500)), "1.5s");
        assert_eq!(go_duration_ms(Duration::from_millis(2_000)), "2s");
        assert_eq!(go_duration_ms(Duration::from_millis(62_345)), "1m2.345s");
        assert_eq!(go_duration_ms(Duration::from_secs(3_600)), "1h0m0s");
        assert_eq!(format_go_duration(1_500), "1.5µs");
        assert_eq!(format_go_duration(999), "999ns");
    }
}
