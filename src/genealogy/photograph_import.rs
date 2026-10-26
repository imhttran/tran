//! Idempotent photograph import: compose inventory, extraction, the private
//! byte store, and the archive photograph schema into one operation.
//!
//! # Operation
//!
//! [`import_photographs`] takes one document version original and imports its
//! photographs into the genealogy schema. It performs these steps in order:
//!
//! 1. Validate that the caller's `content_sha256` matches the bytes.
//! 2. Build the per-page inventory with [`inventory::inventory_bytes`].
//! 3. Extract storable bytes with [`photograph_extract::extract_photographs`].
//! 4. Store each image's bytes in the content-addressed byte store.
//! 5. In one transaction, persist the inventory summary, one asset per content
//!    hash, and one occurrence per page draw, all with `ON CONFLICT DO NOTHING`.
//!
//! Bytes are written to the private store before the transaction opens. A store
//! failure therefore leaves no database row, and a failed transaction leaves no
//! committed inventory, asset, or occurrence: the next attempt starts clean.
//!
//! # Idempotency
//!
//! A second call for the same `(version_id, content_sha256)` writes no new row.
//! The asset key is the content hash and the occurrence key is
//! `(asset, version_id, page_number, occurrence_ordinal)`. The idempotent path
//! issues only inserts with `ON CONFLICT DO NOTHING`; it never updates or
//! deletes an asset, occurrence, or review row, so every existing review decision
//! survives a repeat run.
//!
//! A new content hash belongs to a replaced original and writes its own inventory
//! and occurrence rows. Review decisions attach to an asset id, so a replacement
//! with new image bytes receives fresh asset rows and inherits no review state.
//! Because assets are content-addressed, identical image bytes reuse the same
//! asset row by design; the replacement's own inventory and occurrences are still
//! new.
//!
//! # Provenance
//!
//! Each asset keeps the inventory object id, object number and generation, pixel
//! dimensions, colour space, filters, completeness, classification rule, and
//! detail. Each occurrence keeps the document version, page, and draw order. The
//! import stores no person association and never publishes an asset.

use postgres::GenericClient;
use thiserror::Error;

use crate::genealogy::inventory::{self, ImageObject, Inventory, InventoryError, PersistOutcome};
use crate::genealogy::photograph_extract::{self, ExtractKind, ExtractedImage};
use crate::genealogy::photograph_store::{content_hash, PhotographStore, PhotographStoreError};

/// Errors raised by the photograph import. No variant carries image bytes.
#[derive(Debug, Error)]
pub enum ImportError {
    /// The caller's content hash does not match the bytes.
    #[error("import: content hash does not match the supplied bytes")]
    HashMismatch,
    /// The content hash is not a lowercase hex SHA-256.
    #[error("import: content hash is not a lowercase hex SHA-256")]
    InvalidHash,
    /// The inventory walk failed.
    #[error(transparent)]
    Inventory(#[from] InventoryError),
    /// The private byte store failed.
    #[error(transparent)]
    Store(#[from] PhotographStoreError),
    /// A database write failed.
    #[error("import: database failed: {0}")]
    Database(String),
    /// Extraction reported an object the inventory does not contain.
    #[error("import: no inventory object for page {page_number} occurrence {occurrence_ordinal}")]
    Provenance {
        /// The page the extraction reported.
        page_number: u32,
        /// The draw order the extraction reported.
        occurrence_ordinal: i64,
    },
}

impl From<postgres::Error> for ImportError {
    fn from(err: postgres::Error) -> Self {
        ImportError::Database(describe_pg_error(&err))
    }
}

/// Includes the server message and any source chain instead of the driver's
/// bare `db error` text.
fn describe_pg_error(err: &postgres::Error) -> String {
    if let Some(db) = err.as_db_error() {
        let mut text = db.message().to_string();
        if let Some(detail) = db.detail() {
            text.push_str(": ");
            text.push_str(detail);
        }
        return text;
    }
    let mut text = err.to_string();
    let mut source = std::error::Error::source(err);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = cause.source();
    }
    text
}

/// The summary [`import_photographs`] returns.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct ImportSummary {
    /// The document version the import targeted.
    pub version_id: i64,
    /// The lowercase hex content hash of the source bytes.
    pub content_sha256: String,
    /// Assets this run created.
    pub assets_created: usize,
    /// Assets that already existed and were left unchanged.
    pub assets_unchanged: usize,
    /// Occurrences this run created.
    pub occurrences_created: usize,
    /// Occurrences that already existed and were left unchanged.
    pub occurrences_unchanged: usize,
    /// Photograph objects the extraction produced bytes for.
    pub photographs_stored: usize,
    /// Photograph objects extraction could not store.
    pub photographs_skipped: usize,
    /// The inventory summary outcome.
    pub inventory: PersistOutcome,
}

impl ImportSummary {
    /// True when this run wrote no new asset and no new occurrence.
    pub fn unchanged(&self) -> bool {
        self.assets_created == 0 && self.occurrences_created == 0
    }
}

/// One asset's stored result, before persistence.
struct PreparedAsset {
    /// The relative content-addressed storage key.
    key: String,
    /// The lowercase hex content hash of the stored bytes.
    content_sha256: String,
    /// The stored byte length.
    byte_size: i64,
    /// The media type of the stored bytes.
    mime_type: &'static str,
    /// The inventory object id, for example `12 0`.
    object_id: String,
    /// The lopdf object number.
    object_number: i64,
    /// The lopdf object generation.
    object_generation: i32,
    /// Declared pixel width.
    pixel_width: Option<i32>,
    /// Declared pixel height.
    pixel_height: Option<i32>,
    /// Declared colour space.
    color_space: Option<String>,
    /// Declared filter chain.
    filters: Option<String>,
    /// Whether the declared stream covers its sample set.
    complete: bool,
    /// The named classification rule that selected the photograph.
    classification_rule: String,
    /// The occurrence records this asset draws.
    occurrences: Vec<PreparedOccurrence>,
}

/// One occurrence of an asset on a page.
struct PreparedOccurrence {
    page_number: i32,
    occurrence_ordinal: i32,
}

/// Imports the photographs of one document version original.
///
/// The caller supplies the document `version_id`, the `corpus`, the lowercase
/// hex `content_sha256` of `bytes`, the original `bytes`, and the `actor` that
/// performs the import. The client and the byte store are supplied separately so
/// the function stays callable from a caller-held connection and a test-held
/// store.
///
/// A second call for the same `(version_id, content_sha256)` writes no new row
/// and reports the existing rows as unchanged. A new content hash writes its own
/// inventory and occurrence rows and inherits no review state.
pub fn import_photographs<C: GenericClient>(
    client: &mut C,
    store: &PhotographStore,
    version_id: i64,
    corpus: &str,
    content_sha256: &str,
    bytes: &[u8],
    actor: &str,
) -> Result<ImportSummary, ImportError> {
    if content_sha256.len() != 64 || contains_uppercase(content_sha256) {
        return Err(ImportError::InvalidHash);
    }
    if content_sha256 != content_hash(bytes) {
        return Err(ImportError::HashMismatch);
    }

    let source: Inventory = inventory::inventory_bytes(bytes)?;
    let extracted: Vec<ExtractedImage> = photograph_extract::extract_photographs(bytes, &source);

    // Store every extractable image first. A store failure aborts before any row
    // is written, so a partial import never leaves a half-written version.
    let mut prepared: Vec<PreparedAsset> = Vec::new();
    let mut photographs_stored = 0usize;
    let mut photographs_skipped = 0usize;
    for image in &extracted {
        let Some(image_bytes) = image.bytes.as_deref() else {
            // A non-storable result (unsupported filter, decode failure, not
            // found) has no asset to store; the inventory image row already
            // records it.
            photographs_skipped += 1;
            continue;
        };
        let object = object_for(&source, image.page_number, image.occurrence_ordinal).ok_or(
            ImportError::Provenance {
                page_number: image.page_number,
                occurrence_ordinal: image.occurrence_ordinal,
            },
        )?;
        let analysis = inventory::classify(object, source.page_count);
        let put = store.put(image_bytes)?;
        photographs_stored += 1;
        let occurrence = PreparedOccurrence {
            page_number: image.page_number as i32,
            occurrence_ordinal: image.occurrence_ordinal as i32,
        };
        match prepared
            .iter_mut()
            .find(|asset| asset.content_sha256 == put.content_sha256)
        {
            Some(existing) => existing.occurrences.push(occurrence),
            None => prepared.push(PreparedAsset {
                key: put.key,
                content_sha256: put.content_sha256,
                byte_size: image_bytes.len() as i64,
                mime_type: mime_type_of(image.kind),
                object_id: format!("{} {}", object.object_number, object.generation),
                object_number: object.object_number as i64,
                object_generation: object.generation as i32,
                pixel_width: object.pixel_width.map(|value| value as i32),
                pixel_height: object.pixel_height.map(|value| value as i32),
                color_space: object.color_space.clone(),
                filters: object.filters.clone(),
                complete: object.complete,
                classification_rule: analysis.rule,
                occurrences: vec![occurrence],
            }),
        }
    }

    // One transaction covers the inventory summary, every asset, and every
    // occurrence. A failure at any point rolls all of them back.
    let mut tx = client.transaction()?;
    let inventory_outcome = inventory::persist_inventory(
        &mut tx,
        version_id,
        corpus,
        content_sha256,
        bytes.len() as i64,
        actor,
        &source,
    )?;
    let inventory_id: i64 = tx
        .query_one(
            "SELECT inventory_id FROM archive_photograph_inventory
             WHERE document_version_id = $1 AND content_sha256 = $2",
            &[&version_id, &content_sha256],
        )?
        .get(0);

    let mut summary = ImportSummary {
        version_id,
        content_sha256: content_sha256.to_string(),
        assets_created: 0,
        assets_unchanged: 0,
        occurrences_created: 0,
        occurrences_unchanged: 0,
        photographs_stored,
        photographs_skipped,
        inventory: inventory_outcome,
    };

    for asset in &prepared {
        // One asset per content hash. The unique index on content_sha256 makes
        // the insert a no-op when the asset already exists.
        let inserted: Option<postgres::Row> = tx.query_opt(
            "INSERT INTO archive_photograph_asset
                 (inventory_id, object_id, object_number, object_generation,
                  pixel_width, pixel_height, color_space, filters, complete,
                  classification, classification_rule, occurrence_count,
                  content_sha256, mime_type, byte_size, storage_key,
                  publication_state)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'photograph', $10, $11,
                     $12, $13, $14, $15, 'unpublished')
             ON CONFLICT (content_sha256) DO NOTHING
             RETURNING asset_id",
            &[
                &inventory_id,
                &asset.object_id,
                &asset.object_number,
                &asset.object_generation,
                &asset.pixel_width,
                &asset.pixel_height,
                &asset.color_space,
                &asset.filters,
                &asset.complete,
                &asset.classification_rule,
                &(asset.occurrences.len() as i32),
                &asset.content_sha256,
                &asset.mime_type,
                &asset.byte_size,
                &asset.key,
            ],
        )?;

        let asset_id: i64 = match inserted {
            Some(row) => {
                summary.assets_created += 1;
                row.get(0)
            }
            None => {
                summary.assets_unchanged += 1;
                tx.query_one(
                    "SELECT asset_id FROM archive_photograph_asset WHERE content_sha256 = $1",
                    &[&asset.content_sha256],
                )?
                .get(0)
            }
        };

        for occurrence in &asset.occurrences {
            // No conflict target: both the versioned key and the legacy
            // asset/page/ordinal key must stay a no-op on a repeat import.
            let created: Option<postgres::Row> = tx.query_opt(
                "INSERT INTO archive_photograph_occurrence
                     (asset_id, inventory_id, document_version_id, page_number,
                      occurrence_ordinal)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT DO NOTHING
                 RETURNING occurrence_id",
                &[
                    &asset_id,
                    &inventory_id,
                    &version_id,
                    &occurrence.page_number,
                    &occurrence.occurrence_ordinal,
                ],
            )?;
            if created.is_some() {
                summary.occurrences_created += 1;
            } else {
                summary.occurrences_unchanged += 1;
            }
        }
    }

    tx.commit()?;
    Ok(summary)
}

/// Returns the inventory object at `page_number` and 1-based `ordinal`.
fn object_for(
    inventory: &Inventory,
    page_number: u32,
    occurrence_ordinal: i64,
) -> Option<&ImageObject> {
    let page = inventory
        .pages
        .iter()
        .find(|page| page.page_number == page_number)?;
    let index = usize::try_from(occurrence_ordinal).ok()?.checked_sub(1)?;
    page.images.get(index)
}

fn contains_uppercase(value: &str) -> bool {
    value.chars().any(|ch| ch.is_ascii_uppercase())
}

fn mime_type_of(kind: ExtractKind) -> &'static str {
    match kind {
        ExtractKind::EmbeddedJpeg => "image/jpeg",
        ExtractKind::EmbeddedPng | ExtractKind::ReencodedPng => "image/png",
        _ => "application/octet-stream",
    }
}
