//! AP-108: archive photograph acceptance verification.
//!
//! Every test owns a disposable database through
//! `tests/support/genealogy_isolation.rs` and builds its synthetic PDF in
//! memory. No real photograph, caption, name, hash, or storage path enters the
//! repository.
//!
//! Acceptance cases:
//! 1. a single-person photograph links only the person named in the caption;
//! 2. a group caption links each explicitly named person;
//! 3. an unidentified photograph appears in the gallery but on no profile;
//! 4. a pending photograph is invisible to a family viewer;
//! 5. a reviewer-restricted photograph is invisible for thumbnail and original;
//! 6. a repeated import creates no duplicate asset or occurrence and preserves
//!    the review decision;
//! 7. every page of a two-page PDF is accounted for;
//! 8. an extraction failure is recorded without creating an asset;
//! 9. thumbnail and original access is allowed for a linked family member and
//!    refused for an unlinked caller.

#[path = "support/genealogy_isolation.rs"]
mod genealogy_isolation;

use std::io::Cursor;

use genealogy_isolation::{base_dsn, create_isolated_target, IsolatedTarget};
use image::{ImageBuffer, ImageFormat, Rgb};
use lopdf::{dictionary, Document, Object, Stream};

use rag_foundation::genealogy::inventory::{self, Inventory};
use rag_foundation::genealogy::photograph_extract::{self, ExtractKind};
use rag_foundation::genealogy::photograph_import::{import_photographs, ImportSummary};
use rag_foundation::genealogy::photograph_review::{
    confirm_db, edit_caption_db, propose_layout_links_db, propose_links_db, set_visibility_db,
    ProposalOutcome, PUBLICATION_PUBLISHED, PUBLICATION_UNPUBLISHED,
};
use rag_foundation::genealogy::photograph_store::{content_hash, PhotographStore};
use rag_foundation::onboarding::family_profile::{
    read_admin_archive_photograph, read_admin_archive_photographs, read_family_archive_photograph,
    read_family_archive_photographs, read_family_gallery, ArchivePhotographError,
};
use rag_foundation::onboarding::{FamilyProfileError, IdentityAccount};

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

fn link_member(client: &mut postgres::Client, account: &IdentityAccount, person_id: i64) {
    client
        .execute(
            "INSERT INTO onboarding_person_account_link
                 (person_id, account_id, linked_by, identity_fingerprint)
             VALUES ($1, $2, 'synthetic-admin', $3)",
            &[
                &person_id,
                &account.account_id,
                &account.identity_fingerprint,
            ],
        )
        .expect("link synthetic member");
}

fn member(account_id: i64, fingerprint_byte: char) -> IdentityAccount {
    IdentityAccount {
        account_id,
        identity_fingerprint: fingerprint_byte.to_string().repeat(64),
    }
}

fn synthetic_jpeg() -> Vec<u8> {
    let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(8, 8, Rgb([90, 140, 170]));
    let mut jpeg: Vec<u8> = Vec::new();
    image::DynamicImage::ImageRgb8(img)
        .write_to(&mut Cursor::new(&mut jpeg), ImageFormat::Jpeg)
        .expect("encode synthetic JPEG");
    jpeg
}

fn jpeg_dictionary() -> lopdf::Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => 8,
        "Height" => 8,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
        "Filter" => "DCTDecode",
    }
}

fn unsupported_image_dictionary() -> lopdf::Dictionary {
    dictionary! {
        "Type" => "XObject",
        "Subtype" => "Image",
        "Width" => 200,
        "Height" => 150,
        "ColorSpace" => "DeviceRGB",
        "BitsPerComponent" => 8,
        "Filter" => "CCITTFaxDecode",
    }
}

/// Builds a synthetic PDF with one framed image on the first page and empty
/// page content on every later page.
fn synthetic_pdf(pages: usize, image: Option<(lopdf::Dictionary, Vec<u8>)>) -> Vec<u8> {
    assert!(pages >= 1);
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let mut page_ids = Vec::new();

    let (resources_id, content_id) = match image {
        Some((dictionary, bytes)) => {
            let image_id = doc.add_object(Stream::new(dictionary, bytes));
            let resources_id = doc.add_object(dictionary! {
                "XObject" => dictionary! { "Im0" => image_id },
            });
            let content_id = doc.add_object(Stream::new(
                dictionary! {},
                b"q 612 0 0 792 0 0 cm /Im0 Do Q".to_vec(),
            ));
            (Some(resources_id), content_id)
        }
        None => {
            let content_id = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
            (None, content_id)
        }
    };

    for index in 0..pages {
        let content = if index == 0 {
            content_id
        } else {
            doc.add_object(Stream::new(dictionary! {}, Vec::new()))
        };
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => content,
        };
        if index == 0 {
            if let Some(resources_id) = resources_id {
                page.set("Resources", resources_id);
            }
        }
        page_ids.push(doc.add_object(page));
    }

    let kids: Vec<Object> = page_ids.into_iter().map(Object::Reference).collect();
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => pages as i64,
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

fn synthetic_photograph_pdf(pages: usize) -> Vec<u8> {
    synthetic_pdf(pages, Some((jpeg_dictionary(), synthetic_jpeg())))
}

fn context_pdf(lines: &[(f64, &str)], image_y: Option<f64>) -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let image_id = doc.add_object(Stream::new(jpeg_dictionary(), synthetic_jpeg()));
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Im0" => image_id },
    });
    let mut content = Vec::new();
    if let Some(y) = image_y {
        content.extend_from_slice(format!("q 200 0 0 150 72 {y} cm /Im0 Do Q\n").as_bytes());
    }
    for (y, text) in lines {
        content.extend_from_slice(
            format!(
                "BT /F1 12 Tf 72 {y} Td ({}) Tj ET\n",
                text.replace('\\', "\\\\")
                    .replace('(', "\\(")
                    .replace(')', "\\)")
            )
            .as_bytes(),
        );
    }
    let content_id = doc.add_object(Stream::new(dictionary! {}, content));
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
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).expect("save synthetic context PDF");
    bytes
}

fn two_page_context_pdf(first: &[(f64, &str)], second: &[(f64, &str)], image_y: f64) -> Vec<u8> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let font_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type1",
        "BaseFont" => "Helvetica",
        "Encoding" => "WinAnsiEncoding",
    });
    let image_id = doc.add_object(Stream::new(jpeg_dictionary(), synthetic_jpeg()));
    let resources_id = doc.add_object(dictionary! {
        "Font" => dictionary! { "F1" => font_id },
        "XObject" => dictionary! { "Im0" => image_id },
    });
    let escape = |text: &str| {
        text.replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)")
    };
    let mut first_content = format!("q 200 0 0 150 72 {image_y} cm /Im0 Do Q\n").into_bytes();
    for (y, text) in first {
        first_content.extend_from_slice(
            format!("BT /F1 12 Tf 72 {y} Td ({}) Tj ET\n", escape(text)).as_bytes(),
        );
    }
    let mut second_content = Vec::new();
    for (y, text) in second {
        second_content.extend_from_slice(
            format!("BT /F1 12 Tf 72 {y} Td ({}) Tj ET\n", escape(text)).as_bytes(),
        );
    }
    let first_content_id = doc.add_object(Stream::new(dictionary! {}, first_content));
    let second_content_id = doc.add_object(Stream::new(dictionary! {}, second_content));
    let first_page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => resources_id,
        "Contents" => first_content_id,
    });
    let second_page = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
        "Resources" => resources_id,
        "Contents" => second_content_id,
    });
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => vec![first_page.into(), second_page.into()],
            "Count" => 2,
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    doc.trailer.set("Root", catalog_id);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes)
        .expect("save synthetic two-page context PDF");
    bytes
}

fn insert_birth_year(client: &mut postgres::Client, person_id: i64, year: &str) {
    client
        .execute(
            "INSERT INTO assertion
                 (person_id, assertion_kind, assertion_key, value_text,
                  value_date_start, value_date_precision, review_status)
             VALUES ($1, 'birth_date', 'birth_date', $2, $2, 'year', 'unverified')
             ON CONFLICT (person_id, assertion_kind, assertion_key) DO NOTHING",
            &[&person_id, &year],
        )
        .expect("insert synthetic birth year");
}

/// Imports one PDF and returns the created asset id, if any.
fn import_pdf(
    client: &mut postgres::Client,
    store: &PhotographStore,
    version_id: i64,
    pdf: &[u8],
) -> (Option<i64>, ImportSummary) {
    let hash = content_hash(pdf);
    let summary = import_photographs(client, store, version_id, "family", &hash, pdf, "tester")
        .expect("photograph import");
    let asset_id = client
        .query_opt(
            "SELECT a.asset_id \
               FROM archive_photograph_asset a \
               JOIN archive_photograph_occurrence o ON o.asset_id = a.asset_id \
               JOIN archive_photograph_inventory i ON i.inventory_id = o.inventory_id \
              WHERE o.document_version_id = $1 AND i.content_sha256 = $2",
            &[&version_id, &hash],
        )
        .expect("imported asset lookup")
        .map(|row| row.get(0));
    (asset_id, summary)
}

fn publish(client: &mut postgres::Client, asset_id: i64) {
    let decision = set_visibility_db(client, asset_id, PUBLICATION_PUBLISHED, "reviewer")
        .expect("publish photograph");
    assert!(matches!(
        decision,
        rag_foundation::genealogy::photograph_review::Decision::Changed { .. }
    ));
}

fn propose_and_confirm(
    client: &mut postgres::Client,
    asset_id: i64,
    caption: &str,
    people: &[i64],
) {
    edit_caption_db(client, asset_id, caption, "reviewer").expect("record caption");
    let outcome =
        propose_links_db(client, asset_id, caption, "reviewer").expect("propose person links");
    match &outcome {
        ProposalOutcome::Proposed { .. } | ProposalOutcome::Ambiguous { .. } => {}
        ProposalOutcome::NoLink => panic!("expected a person proposal for {caption:?}"),
    }
    for person_id in people {
        let decision =
            confirm_db(client, asset_id, *person_id, "reviewer").expect("confirm person link");
        assert!(matches!(
            decision,
            rag_foundation::genealogy::photograph_review::Decision::Changed { .. }
                | rag_foundation::genealogy::photograph_review::Decision::NoChange
        ));
    }
}

#[test]
fn a_single_person_caption_links_only_the_named_person() {
    let Some(mut harness) = Harness::new("photograph_acceptance_single") else {
        eprintln!("GENEALOGY_DATABASE_URL is unset; skipping photograph acceptance test");
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-alpha", "Synthetic Person Alpha");
    insert_person(client, "ap108-beta", "Synthetic Person Beta");
    let viewer = member(1, 'a');
    link_member(client, &viewer, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, summary) = import_pdf(client, &harness.store, 1, &pdf);
    let asset_id = asset_id.expect("one photograph asset");
    assert_eq!(summary.assets_created, 1);

    propose_and_confirm(client, asset_id, "Synthetic Person Alpha", &[alpha]);
    publish(client, asset_id);

    let photos = read_family_archive_photographs(client, &viewer, "ap108-alpha")
        .expect("read alpha archive photographs");
    assert_eq!(photos.len(), 1);
    assert_eq!(photos[0].caption.as_deref(), Some("Synthetic Person Alpha"));
    assert_eq!(photos[0].page_number, 1);

    let beta_photos = read_family_archive_photographs(client, &viewer, "ap108-beta")
        .expect("read beta archive photographs");
    assert!(
        beta_photos.is_empty(),
        "a caption naming alpha must not link beta"
    );

    let gallery = read_family_gallery(client, &viewer).expect("read family gallery");
    assert_eq!(gallery.len(), 1);
    assert_eq!(gallery[0].people.len(), 1);
    assert_eq!(gallery[0].people[0].person_key, "ap108-alpha");

    harness.finish();
}

#[test]
fn a_group_caption_links_each_explicitly_named_person() {
    let Some(mut harness) = Harness::new("photograph_acceptance_group") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-group-alpha", "Synthetic Person Alpha");
    let beta = insert_person(client, "ap108-group-beta", "Synthetic Person Beta");
    let viewer = member(2, 'b');
    link_member(client, &viewer, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, _) = import_pdf(client, &harness.store, 2, &pdf);
    let asset_id = asset_id.expect("one photograph asset");

    propose_and_confirm(
        client,
        asset_id,
        "Synthetic Person Alpha and Synthetic Person Beta",
        &[alpha, beta],
    );
    publish(client, asset_id);

    let gallery = read_family_gallery(client, &viewer).expect("read family gallery");
    assert_eq!(gallery.len(), 1);
    let mut keys: Vec<&str> = gallery[0]
        .people
        .iter()
        .map(|person| person.person_key.as_str())
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["ap108-group-alpha", "ap108-group-beta"]);

    for key in ["ap108-group-alpha", "ap108-group-beta"] {
        let photos = read_family_archive_photographs(client, &viewer, key)
            .expect("read group member archive photographs");
        assert_eq!(photos.len(), 1, "{key} sees the group photograph");
    }

    harness.finish();
}

#[test]
fn an_unidentified_photograph_appears_in_the_gallery_but_on_no_profile() {
    let Some(mut harness) = Harness::new("photograph_acceptance_unidentified") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-unidentified-alpha", "Synthetic Person Alpha");
    let viewer = member(3, 'c');
    link_member(client, &viewer, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, _) = import_pdf(client, &harness.store, 3, &pdf);
    let asset_id = asset_id.expect("one photograph asset");
    publish(client, asset_id);

    let gallery = read_family_gallery(client, &viewer).expect("read family gallery");
    assert_eq!(gallery.len(), 1);
    assert!(gallery[0].people.is_empty(), "no caption creates no link");

    let photos = read_family_archive_photographs(client, &viewer, "ap108-unidentified-alpha")
        .expect("read alpha archive photographs");
    assert!(photos.is_empty());

    let download = read_family_archive_photograph(client, &harness.store, &viewer, asset_id)
        .expect("published photograph bytes");
    assert_eq!(download.mime_type, "image/jpeg");
    assert!(!download.bytes.is_empty());

    harness.finish();
}

#[test]
fn a_pending_photograph_is_invisible_to_a_family_viewer() {
    let Some(mut harness) = Harness::new("photograph_acceptance_pending") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-pending-alpha", "Synthetic Person Alpha");
    let viewer = member(4, 'd');
    link_member(client, &viewer, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, _) = import_pdf(client, &harness.store, 4, &pdf);
    let asset_id = asset_id.expect("one photograph asset");

    let gallery = read_family_gallery(client, &viewer).expect("read family gallery");
    assert!(gallery.is_empty(), "pending photograph is not listed");
    assert_eq!(
        read_family_archive_photograph(client, &harness.store, &viewer, asset_id),
        Err(ArchivePhotographError::NotFound)
    );

    // The administrator review read still sees the pending photograph.
    let admin = read_admin_archive_photograph(client, &harness.store, asset_id)
        .expect("admin reads a pending photograph");
    assert!(!admin.bytes.is_empty());
    let review = read_admin_archive_photographs(client).expect("admin review list");
    assert_eq!(review.len(), 1);
    assert_eq!(review[0].publication_state, PUBLICATION_UNPUBLISHED);

    harness.finish();
}

#[test]
fn a_reviewer_restricted_photograph_is_invisible_for_thumbnail_and_original() {
    let Some(mut harness) = Harness::new("photograph_acceptance_restricted") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-restricted-alpha", "Synthetic Person Alpha");
    let viewer = member(5, 'e');
    link_member(client, &viewer, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, _) = import_pdf(client, &harness.store, 5, &pdf);
    let asset_id = asset_id.expect("one photograph asset");
    propose_and_confirm(client, asset_id, "Synthetic Person Alpha", &[alpha]);
    publish(client, asset_id);

    // A published photograph is readable through both family routes.
    for _ in 0..2 {
        assert!(read_family_archive_photograph(client, &harness.store, &viewer, asset_id).is_ok());
    }

    // The reviewer withdraws family visibility. Both routes must now refuse it,
    // exactly like a pending photograph.
    let withdrawn = set_visibility_db(client, asset_id, PUBLICATION_UNPUBLISHED, "reviewer")
        .expect("restrict photograph");
    assert!(matches!(
        withdrawn,
        rag_foundation::genealogy::photograph_review::Decision::Changed { .. }
    ));

    assert!(read_family_gallery(client, &viewer)
        .expect("read family gallery")
        .is_empty());
    for _ in 0..2 {
        assert_eq!(
            read_family_archive_photograph(client, &harness.store, &viewer, asset_id),
            Err(ArchivePhotographError::NotFound)
        );
    }
    // The administrator can still inspect the restricted photograph.
    assert!(read_admin_archive_photograph(client, &harness.store, asset_id).is_ok());

    harness.finish();
}

#[test]
fn a_repeated_import_creates_no_duplicates_and_preserves_review() {
    let Some(mut harness) = Harness::new("photograph_acceptance_repeat") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-repeat-alpha", "Synthetic Person Alpha");
    let viewer = member(6, 'f');
    link_member(client, &viewer, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, first) = import_pdf(client, &harness.store, 6, &pdf);
    let asset_id = asset_id.expect("one photograph asset");
    assert_eq!(first.assets_created, 1);
    assert_eq!(first.occurrences_created, 1);

    propose_and_confirm(client, asset_id, "Synthetic Person Alpha", &[alpha]);
    publish(client, asset_id);

    let (_, second) = import_pdf(client, &harness.store, 6, &pdf);
    assert!(second.unchanged(), "second import writes no new row");
    assert_eq!(second.assets_created, 0);
    assert_eq!(second.occurrences_created, 0);

    assert_eq!(
        count(
            client,
            "SELECT count(*) FROM archive_photograph_asset WHERE asset_id = $1",
            &[&asset_id],
        ),
        1
    );
    assert_eq!(
        count(
            client,
            "SELECT count(*) FROM archive_photograph_occurrence WHERE asset_id = $1",
            &[&asset_id],
        ),
        1
    );
    let state: String = client
        .query_one(
            "SELECT publication_state FROM archive_photograph_asset WHERE asset_id = $1",
            &[&asset_id],
        )
        .expect("publication state")
        .get(0);
    assert_eq!(state, PUBLICATION_PUBLISHED);
    assert_eq!(
        count(
            client,
            "SELECT count(*) FROM archive_photograph_person_link
              WHERE asset_id = $1 AND person_id = $2 AND status = 'confirmed'",
            &[&asset_id, &alpha],
        ),
        1
    );

    // The profile still reads after the repeat import.
    let photos = read_family_archive_photographs(client, &viewer, "ap108-repeat-alpha")
        .expect("read archive photographs");
    assert_eq!(photos.len(), 1);

    harness.finish();
}

#[test]
fn every_page_of_a_two_page_pdf_is_accounted_for() {
    let Some(mut harness) = Harness::new("photograph_acceptance_pages") else {
        return;
    };
    let client = &mut harness.client;
    let pdf = synthetic_photograph_pdf(2);
    let hash = content_hash(&pdf);
    let (_, summary) = import_pdf(client, &harness.store, 7, &pdf);
    assert_eq!(summary.assets_created, 1);

    let page_count: i32 = client
        .query_one(
            "SELECT page_count FROM archive_photograph_inventory
              WHERE document_version_id = $1 AND content_sha256 = $2",
            &[&7_i64, &hash],
        )
        .expect("inventory page count")
        .get(0);
    assert_eq!(page_count, 2);
    assert_eq!(
        count(
            client,
            "SELECT count(*) FROM archive_photograph_inventory_page
              WHERE inventory_id = (
                  SELECT inventory_id FROM archive_photograph_inventory
                   WHERE document_version_id = $1 AND content_sha256 = $2
              )",
            &[&7_i64, &hash],
        ),
        2
    );
    let pages: Vec<i32> = client
        .query(
            "SELECT page_number FROM archive_photograph_inventory_page
              WHERE inventory_id = (
                  SELECT inventory_id FROM archive_photograph_inventory
                   WHERE document_version_id = $1 AND content_sha256 = $2
              )
              ORDER BY page_number",
            &[&7_i64, &hash],
        )
        .expect("inventory pages")
        .into_iter()
        .map(|row| row.get(0))
        .collect();
    assert_eq!(pages, vec![1, 2]);

    harness.finish();
}

#[test]
fn an_extraction_failure_is_recorded_without_an_asset() {
    let Some(mut harness) = Harness::new("photograph_acceptance_extract_failure") else {
        return;
    };
    let client = &mut harness.client;
    let pdf = synthetic_pdf(1, Some((unsupported_image_dictionary(), vec![0_u8; 128])));

    let inventory: Inventory = inventory::inventory_bytes(&pdf).expect("inventory");
    assert_eq!(inventory.page_count, 1);
    let extracted = photograph_extract::extract_photographs(&pdf, &inventory);
    let failure = extracted
        .iter()
        .find(|image| image.kind == ExtractKind::UnsupportedFilter)
        .expect("unsupported filter is recorded");
    assert!(
        failure
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("CCITTFaxDecode")),
        "the recorded reason names the unsupported filter"
    );
    assert!(failure.bytes.is_none());

    let (asset_id, summary) = import_pdf(client, &harness.store, 8, &pdf);
    assert!(asset_id.is_none(), "no asset for an unstorable object");
    assert_eq!(summary.assets_created, 0);
    assert!(summary.photographs_skipped >= 1);
    assert_eq!(
        count(
            client,
            "SELECT count(*) FROM archive_photograph_asset a
               JOIN archive_photograph_occurrence o ON o.asset_id = a.asset_id
               JOIN archive_photograph_inventory i ON i.inventory_id = o.inventory_id
              WHERE i.content_sha256 = $1",
            &[&content_hash(&pdf)],
        ),
        0
    );

    harness.finish();
}

#[test]
fn thumbnail_and_original_access_is_enforced_for_linked_and_unlinked_callers() {
    let Some(mut harness) = Harness::new("photograph_acceptance_bytes") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-bytes-alpha", "Synthetic Person Alpha");
    let linked = member(9, '9');
    let unlinked = member(10, '0');
    link_member(client, &linked, alpha);

    let pdf = synthetic_photograph_pdf(1);
    let (asset_id, _) = import_pdf(client, &harness.store, 9, &pdf);
    let asset_id = asset_id.expect("one photograph asset");

    // A pending asset is not found for the linked family member.
    assert_eq!(
        read_family_archive_photograph(client, &harness.store, &linked, asset_id),
        Err(ArchivePhotographError::NotFound)
    );

    publish(client, asset_id);

    // Both the thumbnail and the original route call the same authorized read.
    for _ in 0..2 {
        let download = read_family_archive_photograph(client, &harness.store, &linked, asset_id)
            .expect("linked caller reads the photograph");
        assert_eq!(download.mime_type, "image/jpeg");
        assert!(!download.bytes.is_empty());
    }

    // An account with no family link is refused, even for a published photo.
    assert_eq!(
        read_family_archive_photograph(client, &harness.store, &unlinked, asset_id),
        Err(ArchivePhotographError::NotFamilyMember)
    );
    assert_eq!(
        read_family_gallery(client, &unlinked),
        Err(FamilyProfileError::NotFamilyMember)
    );

    harness.finish();
}

#[test]
fn layout_proposals_persist_provenance_and_preserve_approved_links() {
    let Some(mut harness) = Harness::new("photograph_acceptance_layout") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-layout-alpha", "Synthetic Person Alpha");
    insert_birth_year(client, alpha, "1940");
    let pdf = context_pdf(
        &[
            (300.0, "Photo: Synthetic Person Alpha"),
            (280.0, "Synthetic Person Alpha was born in 1940."),
        ],
        Some(400.0),
    );
    let (asset_id, _) = import_pdf(client, &harness.store, 10, &pdf);
    let asset_id = asset_id.expect("one photograph asset");

    let proposal =
        propose_layout_links_db(client, asset_id, &pdf, "reviewer").expect("layout proposal");
    assert_eq!(proposal.candidates.len(), 1);
    let candidate = &proposal.candidates[0];
    assert_eq!(candidate.person_id, alpha);
    assert_eq!(candidate.confidence, "high");
    assert_eq!(candidate.source_kind, "caption");
    assert_eq!(candidate.source_page, 1);
    assert!(candidate
        .supporting_text
        .contains("Photo: Synthetic Person Alpha"));
    assert!(!candidate.confidence_basis.is_empty());

    let row = client
        .query_one(
            "SELECT status, source_page, supporting_text, confidence,
                    confidence_basis, match_kind, candidate_rank, proposal_origin
               FROM archive_photograph_person_link
              WHERE asset_id = $1 AND person_id = $2",
            &[&asset_id, &alpha],
        )
        .expect("stored proposal");
    assert_eq!(row.get::<_, String>(0), "proposed");
    assert_eq!(row.get::<_, Option<i32>>(1), Some(1));
    assert!(row
        .get::<_, Option<String>>(2)
        .is_some_and(|text| text.contains("Photo: Synthetic Person Alpha")));
    assert_eq!(row.get::<_, Option<String>>(3).as_deref(), Some("high"));
    assert!(row
        .get::<_, Option<String>>(4)
        .is_some_and(|text| !text.is_empty()));
    assert_eq!(row.get::<_, Option<String>>(5).as_deref(), Some("exact"));
    assert_eq!(row.get::<_, Option<i32>>(6), Some(1));
    assert_eq!(row.get::<_, Option<String>>(7).as_deref(), Some("layout"));

    // The reviewer approves the proposal through the established workflow.
    confirm_db(client, asset_id, alpha, "reviewer").expect("confirm link");

    // A second proposal run must not disturb the approved link or its
    // provenance. It also has no undecided candidate left to propose.
    let repeated = propose_layout_links_db(client, asset_id, &pdf, "reviewer")
        .expect("repeat layout proposal");
    assert_eq!(repeated.preserved_decisions, 1);
    assert!(repeated.candidates.is_empty());

    let confirmed = client
        .query_one(
            "SELECT status, proposal_origin FROM archive_photograph_person_link
              WHERE asset_id = $1 AND person_id = $2",
            &[&asset_id, &alpha],
        )
        .expect("confirmed link");
    assert_eq!(confirmed.get::<_, String>(0), "confirmed");
    assert_eq!(
        confirmed.get::<_, Option<String>>(1).as_deref(),
        Some("layout")
    );
    assert_eq!(
        count(
            client,
            "SELECT count(*) FROM archive_photograph_person_link
              WHERE asset_id = $1 AND status = 'proposed'",
            &[&asset_id],
        ),
        0
    );

    harness.finish();
}

#[test]
fn layout_proposals_order_namesakes_and_record_ambiguity() {
    let Some(mut harness) = Harness::new("photograph_acceptance_layout_namesake") else {
        return;
    };
    let client = &mut harness.client;
    let older = insert_person(client, "ap108-layout-alpha-1940", "Synthetic Alpha");
    let younger = insert_person(client, "ap108-layout-alpha-1950", "Synthetic Alpha");
    insert_birth_year(client, older, "1940");
    insert_birth_year(client, younger, "1950");
    let pdf = context_pdf(
        &[
            (300.0, "Photo: Synthetic Alpha"),
            (280.0, "Synthetic Alpha was born in 1940."),
        ],
        Some(400.0),
    );
    let (asset_id, _) = import_pdf(client, &harness.store, 11, &pdf);
    let asset_id = asset_id.expect("one photograph asset");

    let proposal = propose_layout_links_db(client, asset_id, &pdf, "reviewer")
        .expect("layout namesake proposal");
    assert_eq!(proposal.candidates.len(), 2);
    assert_eq!(proposal.candidates[0].person_id, older);
    assert!(proposal.candidates[0].confidence_basis.contains("1940"));
    assert_eq!(proposal.candidates[1].person_id, younger);
    assert!(proposal.candidates[1].ambiguity.is_some());

    let ranks: Vec<(i64, Option<i32>, Option<String>)> = client
        .query(
            "SELECT person_id, candidate_rank, ambiguity_note
               FROM archive_photograph_person_link
              WHERE asset_id = $1
              ORDER BY candidate_rank",
            &[&asset_id],
        )
        .expect("namesake ranks")
        .into_iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect();
    assert_eq!(ranks[0].0, older);
    assert_eq!(ranks[0].1, Some(1));
    assert!(ranks[0].2.is_none());
    assert_eq!(ranks[1].0, younger);
    assert_eq!(ranks[1].1, Some(2));
    assert!(ranks[1].2.is_some());

    harness.finish();
}

#[test]
fn layout_proposals_use_a_continuing_biography_on_the_next_page() {
    let Some(mut harness) = Harness::new("photograph_acceptance_layout_adjacent") else {
        return;
    };
    let client = &mut harness.client;
    let alpha = insert_person(client, "ap108-layout-adjacent", "Synthetic Person Alpha");
    insert_birth_year(client, alpha, "1940");
    let pdf = two_page_context_pdf(
        &[],
        &[
            (700.0, "Biography of Synthetic Person Alpha"),
            (680.0, "Synthetic Person Alpha was born in 1940."),
        ],
        400.0,
    );
    let (asset_id, _) = import_pdf(client, &harness.store, 12, &pdf);
    let asset_id = asset_id.expect("one photograph asset");

    let proposal = propose_layout_links_db(client, asset_id, &pdf, "reviewer")
        .expect("adjacent-page layout proposal");
    assert_eq!(proposal.candidates.len(), 1);
    let candidate = &proposal.candidates[0];
    assert_eq!(candidate.person_id, alpha);
    assert_eq!(candidate.source_kind, "adjacent_page");
    assert_eq!(candidate.source_page, 2);
    assert_eq!(candidate.confidence, "low");

    harness.finish();
}
