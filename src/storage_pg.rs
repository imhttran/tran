//! PostgreSQL storage client and connection pool over the schema contract in
//! [`crate::storage`].
//!
//! This module ports the database client that the schema contract in
//! `storage.rs` deliberately left out. It supplies:
//!
//! * [`PgPool`] / [`create_pool`] - a pooled, DSN-based Postgres client built on
//!   `r2d2` + `r2d2_postgres`. The pool is created once and reused across
//!   writes; a connection is never opened per write.
//! * [`PgStore`] - an implementation of
//!   [`DocumentStore`](crate::storage::DocumentStore) whose `write_chunk`
//!   validates the embedding dimension before any effect and inserts every
//!   [`StoredChunk`](crate::storage::StoredChunk) column into the `documents`
//!   table, including the `vector(768)` embedding.
//! * Read operations over the `documents` table so a row written by another
//!   language can be fetched and compared: [`PgStore::find_by_content_hash`]
//!   selects by `source` and `content_hash`, and [`PgStore::find_by_id`] fetches
//!   a row by its database-assigned `id`. Both map rows back to a
//!   [`StoredChunkRow`] carrying the full field set plus `id`.
//!
//! The read functions are documented here, in the module header:
//!
//! | Function                        | Query                                                               |
//! |---------------------------------|---------------------------------------------------------------------|
//! | `PgStore::find_by_content_hash` | `SELECT ... WHERE content_hash = $1 [AND source = $2] ORDER BY id` |
//! | `PgStore::find_by_id`           | `SELECT ... WHERE id = $1`                                          |
//!
//! The DSN comes from the existing configuration surface: `DATABASE_URL`
//! (see `crate::config::Config::database_url`), whose default matches the Go
//! baseline.

use std::fmt;

use postgres::types::ToSql;
use postgres::Row;
use r2d2::{Pool, PooledConnection};
use r2d2_postgres::{postgres::NoTls, PostgresConnectionManager};
use thiserror::Error;

use crate::storage::{
    validate_embedding_dim, DimensionMismatchError, DocumentStore, StoredChunk, EMBEDDING_DIM,
};
use crate::transaction::{validate_all_dims, ReplaceError, SourceReplacer};

type PgPoolManager = PostgresConnectionManager<NoTls>;

/// A reusable pool of Postgres connections.
///
/// The pool is created once from a DSN with [`create_pool`]. Every write and
/// read borrows a pooled connection, so a connection is reused rather than
/// opened per operation.
pub type PgPool = Pool<PgPoolManager>;

/// Errors raised by the Postgres storage client (pool creation, writes, and
/// reads).
#[derive(Debug, Error)]
pub enum PgStoreError {
    /// The connection pool could not be built from the DSN.
    #[error("failed to build the Postgres connection pool: {0}")]
    Pool(String),
    /// A pooled connection could not be obtained.
    #[error("failed to acquire a pooled Postgres connection: {0}")]
    Connection(String),
    /// The database returned an error for a statement.
    #[error("Postgres query failed: {0}")]
    Query(String),
    /// The embedding dimension did not match the schema.
    #[error(transparent)]
    Dimension(#[from] DimensionMismatchError),
    /// A stored `embedding` value could not be decoded into `Vec<f64>`.
    #[error("failed to decode the stored embedding: {0}")]
    Embedding(String),
}

/// Builds a reusable Postgres connection pool from `dsn`.
///
/// `dsn` is the same connection string the Go side uses, for example
/// `postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable`. r2d2 connects
/// lazily, so building the pool does not require a live database.
pub fn create_pool(dsn: &str) -> Result<PgPool, PgStoreError> {
    let config = dsn
        .parse::<r2d2_postgres::postgres::Config>()
        .map_err(|err| PgStoreError::Pool(format!("invalid DSN: {err}")))?;
    let manager = PostgresConnectionManager::new(config, NoTls);
    // `build_unchecked` does not wait for a connection, matching the documented
    // lazy contract (r2d2's `build` eagerly establishes `min_idle` connections).
    Ok(Pool::builder().max_size(8).build_unchecked(manager))
}

/// A Postgres-backed document store.
///
/// Holds the reusable connection pool. Construct one with [`PgStore::connect`]
/// (which builds the pool from a DSN) or [`PgStore::from_pool`] (to share an
/// existing pool).
pub struct PgStore {
    pool: PgPool,
}

/// One stored section entry in a source-status response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSection {
    /// Section title stored with the chunk.
    pub section: String,
    /// Document-global chunk index.
    pub chunk_index: i32,
    /// 1-based page, or `None` for a page-less source.
    pub page: Option<i32>,
}

/// The stored ingestion status for one `(corpus, source)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceStatus {
    /// The stored `source` identity.
    pub source: String,
    /// The corpus the rows belong to.
    pub corpus: String,
    /// The fingerprint stored with every row.
    pub content_hash: String,
    /// RFC 3339 timestamp of the stored rows.
    pub ingested_at: String,
    /// Number of stored chunks.
    pub chunks: usize,
    /// Ordered section/chunk provenance.
    pub sections: Vec<SourceSection>,
}

/// A row read back from the `documents` table: the full
/// [`StoredChunk`](crate::storage::StoredChunk) field set plus `id`.
/// database-assigned `id`.
///
/// `ingested_at` is returned as text (RFC 3339), matching the write side.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredChunkRow {
    /// `id` (`001_init.sql`): the `bigserial PRIMARY KEY`, assigned by the
    /// database.
    pub id: i64,
    /// The stored chunk (all columns except `id`).
    pub chunk: StoredChunk,
}

/// A search hit from the SQL retrieval path.
///
/// `similarity` is the pgvector cosine similarity and `keyword_score` is the
/// PostgreSQL full-text rank; the unused field is zero for each path.
#[derive(Debug, Clone, PartialEq)]
pub struct SqlHit {
    /// The stored row.
    pub row: StoredChunkRow,
    /// Cosine similarity from the vector query (`0.0` for a keyword hit).
    pub similarity: f64,
    /// Full-text rank from the keyword query (`0.0` for a vector hit).
    pub keyword_score: f64,
}

/// A `(source, section)` pair selecting the chunks [`PgStore::section_chunks`]
/// returns, matching the Go `retrieval.SectionKey`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SectionKey {
    /// The `source` column value.
    pub source: String,
    /// The `section` column value.
    pub section: String,
}

/// Returns the per-section row cap for a section read: `max(1, limit /
/// key_count)`, matching the Go `SectionChunksFiltered` split. A zero
/// `key_count` yields `max(1, limit)`.
pub fn section_cap(limit: usize, key_count: usize) -> usize {
    (limit / key_count.max(1)).max(1)
}

impl PgStore {
    /// Builds a new store by creating a connection pool from `dsn`.
    pub fn connect(dsn: &str) -> Result<PgStore, PgStoreError> {
        Ok(PgStore {
            pool: create_pool(dsn)?,
        })
    }

    /// Returns the stored status for one `(corpus, source)` pair, or `None`
    /// when the source has no rows. Sections are ordered by chunk index so the
    /// API response preserves document provenance and order.
    pub fn source_status(
        &self,
        corpus: &str,
        source: &str,
    ) -> Result<Option<SourceStatus>, PgStoreError> {
        let mut conn = self.connection()?;
        let rows = conn
            .query(
                "SELECT section, chunk_index, page, content_hash, corpus, ingested_at::text \
                 FROM documents WHERE corpus = $1 AND source = $2 ORDER BY chunk_index, id",
                &[&corpus, &source],
            )
            .map_err(|err| PgStoreError::Query(err.to_string()))?;
        if rows.is_empty() {
            return Ok(None);
        }
        let status = SourceStatus {
            source: source.to_string(),
            corpus: rows[0].get::<_, Option<String>>(4).unwrap_or_default(),
            content_hash: rows[0].get::<_, String>(3),
            ingested_at: rows[0].get::<_, Option<String>>(5).unwrap_or_default(),
            chunks: rows.len(),
            sections: rows
                .iter()
                .map(|row| SourceSection {
                    section: row.get::<_, Option<String>>(0).unwrap_or_default(),
                    chunk_index: row.get::<_, i32>(1),
                    page: row.get::<_, Option<i32>>(2),
                })
                .collect(),
        };
        Ok(Some(status))
    }

    /// Wraps an existing pool, so callers can share one pool across stores.
    pub fn from_pool(pool: PgPool) -> PgStore {
        PgStore { pool }
    }

    /// Returns a pooled connection. The connection returns to the pool when it
    /// is dropped, so the pool is reused rather than reopened per operation.
    fn connection(&self) -> Result<PooledConnection<PgPoolManager>, PgStoreError> {
        self.pool
            .get()
            .map_err(|err| PgStoreError::Connection(err.to_string()))
    }

    /// Reads every row matching `content_hash`, optionally narrowed by `source`,
    /// ordered by `id` so the caller sees a stable sequence.
    ///
    /// Returns an empty vector when no row carries the hash. This is the read
    /// path a foreign writer (for example the Go ingest path) is observed with.
    pub fn find_by_content_hash(
        &self,
        content_hash: &str,
        source: Option<&str>,
    ) -> Result<Vec<StoredChunkRow>, PgStoreError> {
        let mut conn = self.connection()?;

        // When `source` is given, only rows with that source are returned; a
        // NULL `source` never equals a bound text value.
        let rows = match source {
            Some(src) => conn.query(
                "SELECT id, content, source, section, chunk_index, language, page, corpus, \
                 content_hash, embed_model, embedding_dim, chunker_config, \
                 ingested_at::text, embedding::text \
                 FROM documents WHERE content_hash = $1 AND source = $2 ORDER BY id",
                &[&content_hash, &src],
            ),
            None => conn.query(
                "SELECT id, content, source, section, chunk_index, language, page, corpus, \
                 content_hash, embed_model, embedding_dim, chunker_config, \
                 ingested_at::text, embedding::text \
                 FROM documents WHERE content_hash = $1 ORDER BY id",
                &[&content_hash],
            ),
        }
        .map_err(|err| PgStoreError::Query(err.to_string()))?;

        rows.iter().map(row_to_chunk).collect()
    }

    /// Reads every row for `corpus`, ordered by `id`.
    ///
    /// This is the production retrieval read path: the query binary loads the
    /// corpus once and runs the shared vector/FTS/fusion modules over it.
    pub fn rows_for_corpus(&self, corpus: &str) -> Result<Vec<StoredChunkRow>, PgStoreError> {
        let mut conn = self.connection()?;
        let rows = conn
            .query(
                "SELECT id, content, source, section, chunk_index, language, page, corpus, \
                 content_hash, embed_model, embedding_dim, chunker_config, \
                 ingested_at::text, embedding::text \
                 FROM documents WHERE corpus = $1 ORDER BY id",
                &[&corpus],
            )
            .map_err(|err| PgStoreError::Query(err.to_string()))?;

        rows.iter().map(row_to_chunk).collect()
    }

    /// Searches `corpus` by vector similarity using pgvector, returning at most
    /// `top_k` rows ordered by cosine distance. The caller applies the
    /// similarity floor, matching the Go `Search` + `KeepSimilar` contract.
    pub fn vector_search(
        &self,
        corpus: &str,
        vector: &[f64],
        top_k: usize,
    ) -> Result<Vec<SqlHit>, PgStoreError> {
        let mut conn = self.connection()?;
        let literal = format_vector(vector);
        let rows = conn
            .query(
                "SELECT id, content, source, section, chunk_index, language, page, corpus, \
                 content_hash, embed_model, embedding_dim, chunker_config, \
                 ingested_at::text, embedding::text, \
                 (1 - (embedding <=> $1::text::vector))::float8 AS similarity \
                 FROM documents WHERE corpus = $2 \
                 ORDER BY embedding <=> $1::text::vector LIMIT $3",
                &[&literal, &corpus, &(top_k as i64)],
            )
            .map_err(|err| PgStoreError::Query(err.to_string()))?;
        rows.iter()
            .map(|row| {
                let similarity: f64 = row
                    .try_get(14)
                    .map_err(|err| PgStoreError::Query(err.to_string()))?;
                Ok(SqlHit {
                    row: row_to_chunk(row)?,
                    similarity,
                    keyword_score: 0.0,
                })
            })
            .collect()
    }

    /// Searches `corpus` with PostgreSQL full-text search, returning at most
    /// `top_k` rows ordered by `ts_rank`.
    pub fn keyword_search(
        &self,
        corpus: &str,
        query: &str,
        top_k: usize,
        language: &str,
    ) -> Result<Vec<SqlHit>, PgStoreError> {
        let mut conn = self.connection()?;
        let config = normalize_fts_config(language);
        let rows = conn
            .query(
                "SELECT id, content, source, section, chunk_index, language, page, corpus, \
                 content_hash, embed_model, embedding_dim, chunker_config, \
                 ingested_at::text, embedding::text, \
                 ts_rank(\
                     to_tsvector($3::text::regconfig, COALESCE(section, '') || ' ' || content),\
                     websearch_to_tsquery($3::text::regconfig, $1)\
                 )::float8 AS rank \
                 FROM documents \
                 WHERE corpus = $4 AND to_tsvector(\
                     $3::text::regconfig, COALESCE(section, '') || ' ' || content\
                 ) @@ websearch_to_tsquery($3::text::regconfig, $1) \
                 ORDER BY rank DESC LIMIT $2",
                &[&query, &(top_k as i64), &config, &corpus],
            )
            .map_err(|err| PgStoreError::Query(err.to_string()))?;
        rows.iter()
            .map(|row| {
                let keyword_score: f64 = row
                    .try_get(14)
                    .map_err(|err| PgStoreError::Query(err.to_string()))?;
                Ok(SqlHit {
                    row: row_to_chunk(row)?,
                    similarity: 0.0,
                    keyword_score,
                })
            })
            .collect()
    }

    /// Issues `SELECT 1` so a caller can fail fast, naming the store, before any
    /// retrieval work runs. [`create_pool`] connects lazily, so this is the first
    /// point an unreachable database is observed.
    pub fn ping(&self) -> Result<(), PgStoreError> {
        let mut conn = self.connection()?;
        conn.execute("SELECT 1", &[])
            .map_err(|err| PgStoreError::Query(err.to_string()))?;
        Ok(())
    }

    /// Reads the leading chunks of each requested `(source, section)` pair in
    /// `corpus`, ordered by source, section, and chunk index.
    ///
    /// Mirrors the Go `SectionChunksFiltered` contract
    /// (`internal/retrieval/retrieval.go:415`): `limit` is split across the
    /// sections with a per-section cap of [`section_cap`]`(limit, keys.len())`
    /// applied through `ROW_NUMBER() OVER (PARTITION BY source, section ORDER BY
    /// chunk_index)`, so one long section cannot starve the others. An empty key
    /// set returns an empty result without issuing a query.
    ///
    /// Divergence: Go's harness passes a zero `Filter`, so its expansion is not
    /// corpus-scoped. Every `PgStore` read is corpus-scoped, so this read is too;
    /// the two agree on a database holding a single corpus.
    pub fn section_chunks(
        &self,
        corpus: &str,
        keys: &[SectionKey],
        limit: usize,
    ) -> Result<Vec<StoredChunkRow>, PgStoreError> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let per_section = section_cap(limit, keys.len()) as i64;

        let mut placeholders = Vec::with_capacity(keys.len());
        let mut params: Vec<&(dyn ToSql + Sync)> = Vec::with_capacity(keys.len() * 2 + 2);
        for (index, key) in keys.iter().enumerate() {
            placeholders.push(format!("(${}, ${})", 2 * index + 1, 2 * index + 2));
            params.push(&key.source);
            params.push(&key.section);
        }
        let corpus_index = keys.len() * 2 + 1;
        params.push(&corpus);
        params.push(&per_section);

        let sql = format!(
            "SELECT id, content, source, section, chunk_index, language, page, corpus, \
             content_hash, embed_model, embedding_dim, chunker_config, \
             ingested_at::text, embedding::text \
             FROM ( \
                 SELECT *, ROW_NUMBER() OVER ( \
                     PARTITION BY source, section ORDER BY chunk_index \
                 ) AS section_rank \
                 FROM documents \
                 WHERE (source, section) IN ({}) AND corpus = ${corpus_index} \
             ) AS ranked \
             WHERE section_rank <= ${} \
             ORDER BY source, section, chunk_index",
            placeholders.join(", "),
            corpus_index + 1
        );

        let mut conn = self.connection()?;
        let rows = conn
            .query(sql.as_str(), &params)
            .map_err(|err| PgStoreError::Query(err.to_string()))?;
        rows.iter().map(row_to_chunk).collect()
    }

    /// Fetches a single row by its database-assigned `id`.
    ///
    /// Returns `None` when no row carries that id. This is the read path used to
    /// fetch back a row the client (or another writer) inserted.
    pub fn find_by_id(&self, id: i64) -> Result<Option<StoredChunkRow>, PgStoreError> {
        let mut conn = self.connection()?;
        let rows = conn
            .query(
                "SELECT id, content, source, section, chunk_index, language, page, corpus, \
                 content_hash, embed_model, embedding_dim, chunker_config, \
                 ingested_at::text, embedding::text \
                 FROM documents WHERE id = $1",
                &[&id],
            )
            .map_err(|err| PgStoreError::Query(err.to_string()))?;

        match rows.first() {
            Some(row) => Ok(Some(row_to_chunk(row)?)),
            None => Ok(None),
        }
    }
}

/// Maps one `documents` row into a [`StoredChunkRow`].
///
/// The selected column order matches the `SELECT` list in [`PgStore`]'s read
/// functions and the field order of [`StoredChunk`].
fn row_to_chunk(row: &Row) -> Result<StoredChunkRow, PgStoreError> {
    let embedding_text: String = row
        .try_get(13)
        .map_err(|err| PgStoreError::Embedding(err.to_string()))?;
    let embedding = parse_vector(&embedding_text)?;

    Ok(StoredChunkRow {
        id: row.get("id"),
        chunk: StoredChunk {
            content: row.get("content"),
            source: row.get("source"),
            section: row.get("section"),
            chunk_index: row.get("chunk_index"),
            language: row.get("language"),
            page: row.get("page"),
            corpus: row.get("corpus"),
            content_hash: row.try_get("content_hash").unwrap_or_default(),
            embed_model: row.try_get("embed_model").unwrap_or_default(),
            embedding_dim: row.try_get("embedding_dim").unwrap_or(0),
            chunker_config: row.try_get("chunker_config").unwrap_or_default(),
            ingested_at: row.try_get("ingested_at").unwrap_or_default(),
            embedding,
        },
    })
}

impl DocumentStore for PgStore {
    type Error = PgStoreError;

    /// Validates the embedding dimension and then inserts `chunk`.
    ///
    /// The dimension guard ([`validate_embedding_dim`]) runs first: a mismatch
    /// returns [`PgStoreError::Dimension`] and no statement executes, so no row
    /// is inserted. On success every [`StoredChunk`] column is written into the
    /// `documents` table, including the `vector(768)` embedding, formatted as a
    /// pgvector literal (`[1,2,...]`).
    fn write_chunk(&self, chunk: &StoredChunk) -> Result<(), PgStoreError> {
        validate_embedding_dim(EMBEDDING_DIM, chunk.embedding_dim())?;

        let embedding_literal = format_vector(&chunk.embedding);

        let mut conn = self.connection()?;
        conn.execute(
            "INSERT INTO documents \
             (content, source, section, chunk_index, language, page, corpus, \
              content_hash, embed_model, embedding_dim, chunker_config, ingested_at, embedding) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::text::timestamptz, $13::text::vector)",
            &[
                &chunk.content as &(dyn ToSql + Sync),
                &chunk.source,
                &chunk.section,
                &chunk.chunk_index,
                &chunk.language,
                &chunk.page,
                &chunk.corpus,
                &chunk.content_hash,
                &chunk.embed_model,
                &chunk.embedding_dim,
                &chunk.chunker_config,
                &chunk.ingested_at,
                &embedding_literal,
            ],
        )
        .map_err(|err| PgStoreError::Query(err.to_string()))?;

        Ok(())
    }
}

impl SourceReplacer for PgStore {
    /// Replaces every stored row of `source` with `chunks` in one transaction.
    ///
    /// The embedding dimension guard ([`validate_all_dims`]) runs first: a
    /// mismatch returns [`ReplaceError::Dimension`] and no connection or
    /// statement is touched. Otherwise the delete and every insert run inside a
    /// single `BEGIN` / `DELETE` / `INSERT` / `COMMIT` transaction; a failure at
    /// any point rolls the whole transaction back, so the previous rows remain
    /// and no new row survives.
    fn replace_source(&self, source: &str, chunks: &[StoredChunk]) -> Result<(), ReplaceError> {
        validate_all_dims(chunks)?;

        let mut conn = self
            .connection()
            .map_err(|err| ReplaceError::Store(err.to_string()))?;
        let mut tx = conn
            .transaction()
            .map_err(|err| ReplaceError::Store(err.to_string()))?;

        tx.execute("DELETE FROM documents WHERE source = $1", &[&source])
            .map_err(|err| ReplaceError::Store(err.to_string()))?;

        for chunk in chunks {
            let embedding_literal = format_vector(&chunk.embedding);
            tx.execute(
                "INSERT INTO documents \
                 (content, source, section, chunk_index, language, page, corpus, \
                  content_hash, embed_model, embedding_dim, chunker_config, ingested_at, embedding) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::text::timestamptz, $13::text::vector)",
                &[
                    &chunk.content as &(dyn ToSql + Sync),
                    &chunk.source,
                    &chunk.section,
                    &chunk.chunk_index,
                    &chunk.language,
                    &chunk.page,
                    &chunk.corpus,
                    &chunk.content_hash,
                    &chunk.embed_model,
                    &chunk.embedding_dim,
                    &chunk.chunker_config,
                    &chunk.ingested_at,
                    &embedding_literal,
                ],
            )
            .map_err(|err| ReplaceError::Store(err.to_string()))?;
        }

        tx.commit()
            .map_err(|err| ReplaceError::Store(err.to_string()))?;
        Ok(())
    }
}

/// Maps a BCP-47 language tag to a PostgreSQL text-search configuration,
/// matching the Go baseline: empty -> `english`, a supported primary subtag ->
/// its configuration, anything else -> `simple`.
pub fn normalize_fts_config(language: &str) -> &'static str {
    let tag = language.trim().to_lowercase();
    if tag.is_empty() {
        return "english";
    }
    let primary = tag.split(['-', '_']).next().unwrap_or("");
    match primary {
        "ar" => "arabic",
        "hy" => "armenian",
        "eu" => "basque",
        "ca" => "catalan",
        "da" => "danish",
        "nl" => "dutch",
        "en" => "english",
        "fi" => "finnish",
        "fr" => "french",
        "de" => "german",
        "el" => "greek",
        "hi" => "hindi",
        "hu" => "hungarian",
        "id" => "indonesian",
        "ga" => "irish",
        "it" => "italian",
        "lt" => "lithuanian",
        "ne" => "nepali",
        "no" => "norwegian",
        "pt" => "portuguese",
        "ro" => "romanian",
        "ru" => "russian",
        "sr" => "serbian",
        "es" => "spanish",
        "sv" => "swedish",
        "ta" => "tamil",
        "tr" => "turkish",
        "yi" => "yiddish",
        _ => "simple",
    }
}

/// Formats `embedding` as a pgvector literal: `[v0,v1,...]`.
pub fn format_vector(embedding: &[f64]) -> String {
    let mut out = String::with_capacity(embedding.len() * 8 + 2);
    out.push('[');
    for (i, value) in embedding.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format_f64(*value));
    }
    out.push(']');
    out
}

/// Formats an `f64`; a whole finite number is written as an integer.
fn format_f64(value: f64) -> String {
    if value.fract() == 0.0 && value.is_finite() && value.abs() < 1e15 {
        format!("{}", value as i64)
    } else {
        format!("{value}")
    }
}

/// Parses a pgvector text literal (`[1,2,3]`) back into `Vec<f64>`.
pub fn parse_vector(text: &str) -> Result<Vec<f64>, PgStoreError> {
    let trimmed = text.trim();
    let body = trimmed
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .ok_or_else(|| PgStoreError::Embedding(format!("not a vector literal: {text}")))?;

    if body.trim().is_empty() {
        return Ok(Vec::new());
    }

    body.split(',')
        .map(|part| {
            part.trim().parse::<f64>().map_err(|err| {
                PgStoreError::Embedding(format!("bad vector element {part:?}: {err}"))
            })
        })
        .collect()
}

impl fmt::Debug for PgStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PgStore").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_literal_uses_pgvector_syntax() {
        assert_eq!(format_vector(&[1.0, 2.5, 3.0]), "[1,2.5,3]");
        assert_eq!(format_vector(&[]), "[]");
    }

    #[test]
    fn vector_literal_round_trips() {
        let original = vec![0.0, 1.0, -0.5, 768.0];
        let parsed = parse_vector(&format_vector(&original)).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_vector_rejects_garbage() {
        assert!(parse_vector("not-a-vector").is_err());
        assert!(parse_vector("[1,x]").is_err());
    }

    #[test]
    fn section_cap_splits_the_limit_across_sections() {
        assert_eq!(section_cap(20, 3), 6);
        assert_eq!(section_cap(20, 1), 20);
        assert_eq!(section_cap(20, 40), 1);
        assert_eq!(section_cap(0, 2), 1);
    }

    #[test]
    fn empty_section_keys_return_nothing_without_a_query() {
        // The DSN points at a closed port: any query would fail, so an Ok(empty)
        // proves the empty key set short-circuits before touching the database.
        let store = PgStore::connect("postgres://nobody:nothing@127.0.0.1:1/none").unwrap();
        let rows = store.section_chunks("any", &[], 20).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn invalid_dsn_is_a_pool_error() {
        assert!(matches!(
            create_pool("not a dsn"),
            Err(PgStoreError::Pool(_))
        ));
    }

    #[test]
    fn valid_dsn_builds_a_pool_without_connecting() {
        // Building the pool must not require a live database: r2d2 connects
        // lazily. This keeps the unit test DB-free, matching the storage
        // foundation it ports.
        let pool = create_pool("postgres://rag:rag@127.0.0.1:5434/rag?sslmode=disable").unwrap();
        // r2d2 connects lazily, so no connection exists yet and the pool reports
        // at most its configured `max_size` of 8.
        assert!(pool.state().connections <= 8);
        assert_eq!(pool.max_size(), 8);
    }
}
