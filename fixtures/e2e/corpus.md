# Storage architecture

PostgreSQL with the pgvector extension stores the vectors for retrieval.
The ingest pipeline embeds each chunk and replaces the source rows in one
transaction.

## Retrieval

Retrieval fuses vector search and full-text search, filters by similarity,
deduplicates, and expands each hit to its neighbouring chunks.
