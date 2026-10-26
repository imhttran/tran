//! AP-104: idempotent photograph import integration tests.
//!
//! Every test uses the per-suite disposable database helper
//! (`tests/support/genealogy_isolation.rs`) so it creates and drops only its own
//! database. Each test builds a synthetic one-page PDF with `lopdf` in memory.
//!
//! Covered:
//! 1. repeated import creates no duplicate asset or occurrence and preserves a
//!    review decision;
//! 2. a replaced original gets its own rows and inherits no review state;
//! 3. a partial failure rolls back every row and a retry succeeds;
//! 4. source, version, page, occurrence, and object provenance is preserved and
//!    the asset stays unpublished.
//!
//! When `GENEALOGY_DATABASE_URL` is unset the tests skip cleanly.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::io::Cursor;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use image::{ImageBuffer, ImageFormat, Rgb};
use lopdf::{dictionary, Document, Object, Stream};

use rag_foundation::genealogy::inventory::inventory_bytes;
use rag_foundation::genealogy::photograph_import::import_photographs;
use rag_foundation::genealogy::photograph_store::{content_hash, PhotographStore};

/// Owns one isolated database, its connection, and a private temporary store.
struct Harness {
    target: IsolatedTarget,
    client: postgres::Client,
    store_dir: tempfile::TempDir,
    store: PhotographStore,
}

impl Harness {
    fn new(label: &str) -> Option<Self> {
        let base = base_dsn()?;
        let target = create_isolated_target(&base, label);
        target.apply_migrations();
        let client = target.connect();
        let store_dir = tempfile::tempdir().expect("temporary photograph store");
        let store = PhotographStore::new(store_dir.path().join("photographs"));
        Some(Self {
            target,
            client,
            store_dir,
            store,
        })
    }

    fn finish(self) {
        let Harness {
            target,
            client,
            store_dir,
            ..
        } = self;
        drop(client);
        drop(store_dir);
        target.cleanup();
    }
}

fn count(
    client: &mut postgres::Client,
    sql: &str,
    params: &[&(dyn postgres::types::ToSql + Sync)],
) -> i64 {
    client.query_one(sql, params).expect("count").get(0)
}

fn insert_person(client: &mut postgres::Client, key: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $1)
             RETURNING person_id",
            &[&key],
        )
        .expect("insert synthetic person")
        .get(0)
}

/// Builds a synthetic one-page PDF that draws one embedded JPEG photograph.
fn synthetic_photograph_pdf(rgb: [u8; 3]) -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(8, 8, Rgb(rgb));
    let mut jpeg: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
        .expect("encode synthetic JPEG");

    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let content_id = doc.add_object(Stream::new(
        dictionary! {},
        b"q 612 0 0 792 0 0 cm /Im0 Do Q".to_vec(),
    ));

    let dict = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => 8,
        "Height" => 8,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
        "Filter" => "DCTDecode",
    };
    let image_id = doc.add_object(Stream::new(dict, jpeg));
    let resources_id = doc.add_object(dictionary! {
        "XObject" => dictionary! { "Im0" => image_id },
    });
    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => resources_id,
        "Contents" => content_id,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![page_id.into()],
            "Count" => 1,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes: Vec<u8> = Vec::new();
    doc.save_to(&mut bytes).expect("save synthetic PDF");
    bytes
}

fn original_pdf() -> Vec<u8> {
    synthetic_photograph_pdf([180, 120, 60])
}

fn replaced_pdf() -> Vec<u8> {
    synthetic_photograph_pdf([10, 200, 30])
}

fn import(
    client: &mut postgres::Client,
    store: &PhotographStore,
    version_id: i64,
    hash: &str,
    bytes: &[u8],
) -> rag_foundation::genealogy::photograph_import::ImportSummary {
    import_photographs(client, store, version_id, "family", hash, bytes, "tester")
        .expect("photograph import")
}

fn inventory_count(client: &mut postgres::Client, version_id: i64, hash: &str) -> i64 {
    count(
        client,
        "SELECT count(*) FROM archive_photograph_inventory
         WHERE document_version_id = $1 AND content_sha256 = $2",
        &[&version_id, &hash],
    )
}

fn asset_ids(client: &mut postgres::Client, version_id: i64, pdf_hash: &str) -> Vec<i64> {
    client
        .query(
            "SELECT a.asset_id
             FROM archive_photograph_asset a
             JOIN archive_photograph_occurrence o ON o.asset_id = a.asset_id
             JOIN archive_photograph_inventory i ON i.inventory_id = o.inventory_id
             WHERE o.document_version_id = $1 AND i.content_sha256 = $2
             ORDER BY a.asset_id",
            &[&version_id, &pdf_hash],
        )
        .expect("asset ids")
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

fn occurrence_count(client: &mut postgres::Client, version_id: i64) -> i64 {
    count(
        client,
        "SELECT count(*) FROM archive_photograph_occurrence WHERE document_version_id = $1",
        &[&version_id],
    )
}

#[test]
fn repeated_import_creates_no_duplicates_and_preserves_review_state() {
    let Some(mut harness) = Harness::new("photograph_import_repeat") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping photograph import test");
        return;
    };
    let client = &mut harness.client;
    let store = &harness.store;
    let version_id: i64 = 1;
    let original = original_pdf();
    let hash = content_hash(&original);

    let person_id = insert_person(client, "ap104-repeat-person");

    let first = import(client, store, version_id, &hash, &original);
    assert_eq!(first.assets_created, 1, "first import creates one asset");
    assert_eq!(
        first.occurrences_created, 1,
        "first import creates one occurrence"
    );
    assert_eq!(first.assets_unchanged, 0);
    assert_eq!(first.occurrences_unchanged, 0);
    assert_eq!(first.photographs_stored, 1);
    assert_eq!(first.photographs_skipped, 0);
    assert_eq!(inventory_count(client, version_id, &hash), 1);
    assert_eq!(asset_ids(client, version_id, &hash).len(), 1);
    assert_eq!(occurrence_count(client, version_id), 1);

    let asset_id: i64 = asset_ids(client, version_id, &hash)[0];
    client
        .execute(
            "INSERT INTO archive_photograph_person_link
                 (asset_id, person_id, status, decided_by, decided_at)
             VALUES ($1, $2, 'confirmed', 'reviewer', now())",
            &[&asset_id, &person_id],
        )
        .expect("record review decision");

    let second = import(client, store, version_id, &hash, &original);
    assert_eq!(second.assets_created, 0, "second import creates no asset");
    assert_eq!(
        second.occurrences_created, 0,
        "second import creates no occurrence"
    );
    assert_eq!(second.assets_unchanged, 1);
    assert_eq!(second.occurrences_unchanged, 1);
    assert!(second.unchanged(), "second import reports no writes");
    assert_eq!(inventory_count(client, version_id, &hash), 1);
    assert_eq!(asset_ids(client, version_id, &hash).len(), 1);
    assert_eq!(occurrence_count(client, version_id), 1);

    let status: String = client
        .query_one(
            "SELECT status FROM archive_photograph_person_link
             WHERE asset_id = $1 AND person_id = $2",
            &[&asset_id, &person_id],
        )
        .expect("read review decision")
        .get(0);
    assert_eq!(status, "confirmed", "the review decision is preserved");

    harness.finish();
}

#[test]
fn replaced_original_gets_its_own_rows() {
    let Some(mut harness) = Harness::new("photograph_import_replaced") else {
        return;
    };
    let client = &mut harness.client;
    let store = &harness.store;
    let version_id: i64 = 7;

    let original = original_pdf();
    let original_hash = content_hash(&original);
    let first = import(client, store, version_id, &original_hash, &original);
    assert_eq!(first.assets_created, 1);
    let original_inventory: i64 = client
        .query_one(
            "SELECT inventory_id FROM archive_photograph_inventory
             WHERE document_version_id = $1 AND content_sha256 = $2",
            &[&version_id, &original_hash],
        )
        .expect("original inventory")
        .get(0);
    let original_asset: i64 = asset_ids(client, version_id, &original_hash)[0];
    // Simulate an approved review decision on the original.
    client
        .execute(
            "UPDATE archive_photograph_asset
             SET publication_state = 'published' WHERE asset_id = $1",
            &[&original_asset],
        )
        .expect("publish original");

    let replaced = replaced_pdf();
    let replaced_hash = content_hash(&replaced);
    assert_ne!(replaced_hash, original_hash);
    let second = import(client, store, version_id, &replaced_hash, &replaced);
    assert_eq!(second.assets_created, 1);
    assert_eq!(second.occurrences_created, 1);

    let replaced_inventory: i64 = client
        .query_one(
            "SELECT inventory_id FROM archive_photograph_inventory
             WHERE document_version_id = $1 AND content_sha256 = $2",
            &[&version_id, &replaced_hash],
        )
        .expect("replaced inventory")
        .get(0);
    assert_ne!(
        replaced_inventory, original_inventory,
        "the replacement has its own inventory"
    );

    let replaced_asset: i64 = asset_ids(client, version_id, &replaced_hash)[0];
    let replaced_state: String = client
        .query_one(
            "SELECT publication_state FROM archive_photograph_asset WHERE asset_id = $1",
            &[&replaced_asset],
        )
        .expect("replaced asset")
        .get(0);
    assert_ne!(
        replaced_asset, original_asset,
        "the new asset has its own id"
    );
    assert_eq!(
        replaced_state, "unpublished",
        "the replacement inherits no publication state"
    );
    let replaced_links = count(
        client,
        "SELECT count(*) FROM archive_photograph_person_link WHERE asset_id = $1",
        &[&replaced_asset],
    );
    assert_eq!(
        replaced_links, 0,
        "the replacement inherits no review decision"
    );

    let original_state: String = client
        .query_one(
            "SELECT publication_state FROM archive_photograph_asset WHERE asset_id = $1",
            &[&original_asset],
        )
        .expect("original state")
        .get(0);
    assert_eq!(original_state, "published", "the original review survives");
    assert_eq!(
        occurrence_count(client, version_id),
        2,
        "each original gets its own occurrence"
    );

    harness.finish();
}

#[test]
fn partial_failure_rolls_back_and_retry_succeeds() {
    let Some(mut harness) = Harness::new("photograph_import_partial") else {
        return;
    };
    let client = &mut harness.client;
    let store = &harness.store;
    let version_id: i64 = 11;
    let original = original_pdf();
    let hash = content_hash(&original);

    // Fail the asset insert after the inventory rows have been written inside
    // the same transaction. The whole import must roll back.
    client
        .batch_execute(
            "CREATE OR REPLACE FUNCTION ap104_fail_asset() RETURNS trigger AS $$
             BEGIN RAISE EXCEPTION 'injected asset insert failure'; END;
             $$ LANGUAGE plpgsql;
             CREATE TRIGGER ap104_fail_asset BEFORE INSERT ON archive_photograph_asset
             FOR EACH ROW EXECUTE FUNCTION ap104_fail_asset();",
        )
        .expect("install failure trigger");

    let failed = import_photographs(
        client, store, version_id, "family", &hash, &original, "tester",
    );
    assert!(failed.is_err(), "the injected failure is reported");
    assert_eq!(inventory_count(client, version_id, &hash), 0);
    assert!(asset_ids(client, version_id, &hash).is_empty());
    assert_eq!(occurrence_count(client, version_id), 0);

    client
        .batch_execute(
            "DROP TRIGGER ap104_fail_asset ON archive_photograph_asset;
             DROP FUNCTION ap104_fail_asset();",
        )
        .expect("remove failure trigger");

    let recovered = import(client, store, version_id, &hash, &original);
    assert_eq!(recovered.assets_created, 1);
    assert_eq!(recovered.occurrences_created, 1);
    assert_eq!(inventory_count(client, version_id, &hash), 1);
    assert_eq!(asset_ids(client, version_id, &hash).len(), 1);
    assert_eq!(occurrence_count(client, version_id), 1);

    harness.finish();
}

#[test]
fn provenance_is_preserved() {
    let Some(mut harness) = Harness::new("photograph_import_provenance") else {
        return;
    };
    let client = &mut harness.client;
    let store = &harness.store;
    let version_id: i64 = 21;
    let original = original_pdf();
    let hash = content_hash(&original);
    let inventory = inventory_bytes(&original).expect("inventory");
    let expected = &inventory.pages[0].images[0];

    let summary = import(client, store, version_id, &hash, &original);
    assert_eq!(summary.assets_created, 1);
    assert_eq!(summary.photographs_stored, 1);

    let row = client
        .query_one(
            "SELECT a.object_id, a.object_number, a.object_generation,
                    a.pixel_width, a.pixel_height, a.color_space, a.filters,
                    a.mime_type, a.byte_size, a.storage_key, a.publication_state,
                    a.content_sha256, a.inventory_id, a.asset_id
             FROM archive_photograph_asset a
             JOIN archive_photograph_occurrence o ON o.asset_id = a.asset_id
             JOIN archive_photograph_inventory i ON i.inventory_id = o.inventory_id
             WHERE o.document_version_id = $1 AND i.content_sha256 = $2",
            &[&version_id, &hash],
        )
        .expect("provenance row");

    assert_eq!(
        row.get::<_, String>(0),
        format!("{} {}", expected.object_number, expected.generation)
    );
    assert_eq!(row.get::<_, i64>(1), expected.object_number as i64);
    assert_eq!(row.get::<_, i32>(2), expected.generation as i32);
    assert_eq!(
        row.get::<_, Option<i32>>(3),
        expected.pixel_width.map(|value| value as i32)
    );
    assert_eq!(
        row.get::<_, Option<i32>>(4),
        expected.pixel_height.map(|value| value as i32)
    );
    assert_eq!(row.get::<_, Option<String>>(5), expected.color_space);
    assert_eq!(row.get::<_, Option<String>>(6), expected.filters);
    assert_eq!(row.get::<_, String>(7), "image/jpeg");
    assert!(row.get::<_, i64>(8) > 0);
    assert_eq!(row.get::<_, String>(10), "unpublished");

    let storage_key: String = row.get(9);
    let stored = std::fs::read(store.root().join(&storage_key)).expect("stored bytes");
    assert_eq!(content_hash(&stored), row.get::<_, String>(11));
    assert_eq!(
        &stored[..3],
        &[0xFF, 0xD8, 0xFF],
        "stored bytes are the JPEG"
    );

    let asset_id: i64 = row.get(13);
    let occurrence = client
        .query_one(
            "SELECT document_version_id, page_number, occurrence_ordinal
             FROM archive_photograph_occurrence WHERE asset_id = $1",
            &[&asset_id],
        )
        .expect("occurrence");
    assert_eq!(occurrence.get::<_, i64>(0), version_id);
    assert_eq!(occurrence.get::<_, i32>(1), 1);
    assert_eq!(occurrence.get::<_, i32>(2), 1);

    let inventory_row = client
        .query_one(
            "SELECT document_version_id, corpus, content_sha256
             FROM archive_photograph_inventory WHERE inventory_id = $1",
            &[&row.get::<_, i64>(12)],
        )
        .expect("inventory provenance");
    assert_eq!(inventory_row.get::<_, i64>(0), version_id);
    assert_eq!(inventory_row.get::<_, String>(1), "family");
    assert_eq!(inventory_row.get::<_, String>(2), hash);

    harness.finish();
}
