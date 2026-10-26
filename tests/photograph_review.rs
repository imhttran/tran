//! AP-105: photograph-to-person review integration tests.
//!
//! Each test owns a disposable database through
//! `tests/support/genealogy_isolation.rs` and builds its synthetic PDF in
//! memory. No real family data, image, or caption enters the repository.
//!
//! Covered:
//! 1. an unnamed caption proposes no link;
//! 2. a group caption proposes each explicitly named person;
//! 3. two people with the same recorded name stay ambiguous until a reviewer
//!    chooses one;
//! 4. repeating a decision appends no duplicate review event;
//! 5. caption edits and visibility changes are recorded and stay unpublished
//!    until a reviewer publishes.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::io::Cursor;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use image::{ImageBuffer, ImageFormat, Rgb};
use lopdf::{dictionary, Document, Object, Stream};

use rag_foundation::genealogy::photograph_import::import_photographs;
use rag_foundation::genealogy::photograph_review::{
    confirm_db, edit_caption_db, propose_links_db, reject_db, set_visibility_db, Decision,
    ProposalOutcome, PUBLICATION_PUBLISHED, PUBLICATION_UNPUBLISHED,
};
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

fn reviewed_events(client: &mut postgres::Client) -> i64 {
    count(
        client,
        "SELECT count(*) FROM archive_photograph_inventory_event
         WHERE action = 'reviewed'",
        &[],
    )
}

fn link_count(client: &mut postgres::Client, status: &str) -> i64 {
    count(
        client,
        "SELECT count(*) FROM archive_photograph_person_link WHERE status = $1",
        &[&status],
    )
}

fn insert_person(client: &mut postgres::Client, key: &str, name: &str) -> i64 {
    client
        .query_one(
            "INSERT INTO person (person_key, display_name) VALUES ($1, $2)
             RETURNING person_id",
            &[&key, &name],
        )
        .expect("insert synthetic person")
        .get(0)
}

fn insert_alias(client: &mut postgres::Client, person_id: i64, alias: &str) {
    client
        .execute(
            "INSERT INTO assertion
                 (person_id, assertion_kind, assertion_key, value_text)
             VALUES ($1, 'name_variant', $2, $2)",
            &[&person_id, &alias],
        )
        .expect("insert synthetic alias");
}

/// Builds a synthetic one-page PDF that draws one embedded JPEG photograph.
fn synthetic_photograph_pdf() -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(8, 8, Rgb([90, 140, 170]));
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

/// Imports one synthetic photograph and returns its asset id.
fn import_photo(client: &mut postgres::Client, store: &PhotographStore, version_id: i64) -> i64 {
    let pdf = synthetic_photograph_pdf();
    let hash = content_hash(&pdf);
    let summary = import_photographs(client, store, version_id, "family", &hash, &pdf, "tester")
        .expect("photograph import");
    assert_eq!(summary.assets_created, 1);
    assert_eq!(summary.occurrences_created, 1);
    client
        .query_one(
            "SELECT a.asset_id
               FROM archive_photograph_asset a
               JOIN archive_photograph_occurrence o ON o.asset_id = a.asset_id
               JOIN archive_photograph_inventory i ON i.inventory_id = o.inventory_id
              WHERE o.document_version_id = $1 AND i.content_sha256 = $2",
            &[&version_id, &hash],
        )
        .expect("imported asset")
        .get(0)
}

#[test]
fn unnamed_caption_creates_no_link() {
    let Some(mut harness) = Harness::new("photograph_review_unnamed") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping photograph review test");
        return;
    };
    let client = &mut harness.client;
    insert_person(client, "ap105-unnamed", "Nguyễn Văn An");
    let asset_id = import_photo(client, &harness.store, 1);

    let outcome = propose_links_db(client, asset_id, "Family gathering, 1962", "reviewer")
        .expect("propose links");
    assert_eq!(outcome, ProposalOutcome::NoLink);
    assert_eq!(link_count(client, "proposed"), 0);
    assert_eq!(reviewed_events(client), 0);

    harness.finish();
}

#[test]
fn a_group_caption_proposes_each_explicitly_named_person() {
    let Some(mut harness) = Harness::new("photograph_review_group") else {
        return;
    };
    let client = &mut harness.client;
    insert_person(client, "ap105-an", "Nguyễn Văn An");
    insert_person(client, "ap105-binh", "Trần Thị Bình");
    let asset_id = import_photo(client, &harness.store, 2);

    let outcome = propose_links_db(
        client,
        asset_id,
        "Nguyễn Văn An và Trần Thị Bình",
        "reviewer",
    )
    .expect("propose links");
    match outcome {
        ProposalOutcome::Proposed { people } => {
            let mut keys: Vec<&str> = people
                .iter()
                .map(|person| person.person_key.as_str())
                .collect();
            keys.sort();
            assert_eq!(keys, vec!["ap105-an", "ap105-binh"]);
        }
        other => panic!("expected a proposal, got {other:?}"),
    }
    assert_eq!(link_count(client, "proposed"), 2);
    assert_eq!(reviewed_events(client), 1);

    // A second read of the same caption writes nothing.
    let repeated = propose_links_db(
        client,
        asset_id,
        "Nguyễn Văn An và Trần Thị Bình",
        "reviewer",
    )
    .expect("repeat proposal");
    assert!(matches!(repeated, ProposalOutcome::Proposed { .. }));
    assert_eq!(link_count(client, "proposed"), 2);
    assert_eq!(reviewed_events(client), 1);

    harness.finish();
}

#[test]
fn an_alias_caption_proposes_the_recorded_person() {
    let Some(mut harness) = Harness::new("photograph_review_alias") else {
        return;
    };
    let client = &mut harness.client;
    let an = insert_person(client, "ap105-alias-an", "Nguyễn Văn An");
    insert_alias(client, an, "Ông An");
    let asset_id = import_photo(client, &harness.store, 3);

    let outcome = propose_links_db(client, asset_id, "Ông An", "reviewer").expect("propose alias");
    match outcome {
        ProposalOutcome::Proposed { people } => {
            assert_eq!(people.len(), 1);
            assert_eq!(people[0].person_key, "ap105-alias-an");
        }
        other => panic!("expected a proposal, got {other:?}"),
    }
    assert_eq!(link_count(client, "proposed"), 1);

    harness.finish();
}

#[test]
fn namesakes_require_an_explicit_choice_and_repeat_is_a_no_op() {
    let Some(mut harness) = Harness::new("photograph_review_namesake") else {
        return;
    };
    let client = &mut harness.client;
    let first = insert_person(client, "ap105-namesake-1", "Nguyễn Văn An");
    let second = insert_person(client, "ap105-namesake-2", "Nguyễn Văn An");
    let asset_id = import_photo(client, &harness.store, 4);

    let outcome =
        propose_links_db(client, asset_id, "Nguyễn Văn An", "reviewer").expect("propose namesakes");
    match outcome {
        ProposalOutcome::Ambiguous {
            proposed,
            candidates,
        } => {
            assert!(proposed.is_empty(), "a namesake is not proposed");
            assert_eq!(candidates.len(), 2);
        }
        other => panic!("expected ambiguity, got {other:?}"),
    }
    assert_eq!(link_count(client, "proposed"), 0);
    assert_eq!(reviewed_events(client), 0);

    let decision = confirm_db(client, asset_id, second, "reviewer").expect("choose namesake");
    match &decision {
        Decision::Changed { prior, next, .. } => {
            assert_eq!(prior, "none");
            assert_eq!(next, "confirmed");
        }
        Decision::NoChange => panic!("the first choice must change state"),
    }
    assert_eq!(reviewed_events(client), 1);

    let repeated = confirm_db(client, asset_id, second, "reviewer").expect("repeat choice");
    assert_eq!(repeated, Decision::NoChange);
    assert_eq!(reviewed_events(client), 1, "no duplicate review event");

    // The other namesake is still separate and can be rejected explicitly.
    let rejected = reject_db(client, asset_id, first, "reviewer").expect("reject other namesake");
    assert!(matches!(rejected, Decision::Changed { .. }));
    assert_eq!(link_count(client, "confirmed"), 1);
    assert_eq!(link_count(client, "rejected"), 1);
    assert_eq!(reviewed_events(client), 2);

    harness.finish();
}

#[test]
fn caption_and_visibility_changes_are_recorded_once() {
    let Some(mut harness) = Harness::new("photograph_review_visibility") else {
        return;
    };
    let client = &mut harness.client;
    insert_person(client, "ap105-visibility", "Nguyễn Văn An");
    let asset_id = import_photo(client, &harness.store, 5);

    let initial: String = client
        .query_one(
            "SELECT publication_state FROM archive_photograph_asset WHERE asset_id = $1",
            &[&asset_id],
        )
        .expect("initial publication state")
        .get(0);
    assert_eq!(initial, PUBLICATION_UNPUBLISHED);

    let edited =
        edit_caption_db(client, asset_id, "Nguyễn Văn An", "reviewer").expect("edit caption");
    assert!(matches!(edited, Decision::Changed { .. }));
    assert_eq!(
        edit_caption_db(client, asset_id, "Nguyễn Văn An", "reviewer").expect("repeat caption"),
        Decision::NoChange
    );
    let caption: Option<String> = client
        .query_one(
            "SELECT i.caption
               FROM archive_photograph_inventory_image i
               JOIN archive_photograph_asset a
                 ON a.inventory_id = i.inventory_id
                AND a.object_number = i.object_number
                AND a.object_generation = i.object_generation
              WHERE a.asset_id = $1
              ORDER BY i.page_number, i.occurrence_ordinal
              LIMIT 1",
            &[&asset_id],
        )
        .expect("edited caption")
        .get(0);
    assert_eq!(caption.as_deref(), Some("Nguyễn Văn An"));

    let published =
        set_visibility_db(client, asset_id, PUBLICATION_PUBLISHED, "reviewer").expect("publish");
    assert!(matches!(published, Decision::Changed { .. }));
    assert_eq!(
        set_visibility_db(client, asset_id, PUBLICATION_PUBLISHED, "reviewer")
            .expect("repeat publication"),
        Decision::NoChange
    );
    let state: String = client
        .query_one(
            "SELECT publication_state FROM archive_photograph_asset WHERE asset_id = $1",
            &[&asset_id],
        )
        .expect("publication state")
        .get(0);
    assert_eq!(state, PUBLICATION_PUBLISHED);
    assert_eq!(reviewed_events(client), 2, "one event per real change");

    harness.finish();
}
