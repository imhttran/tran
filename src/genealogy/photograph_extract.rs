//! Extract storable image bytes for photograph objects from a document original.
//!
//! # Scope
//!
//! [`extract_photographs`] takes the original PDF `bytes` plus the [`Inventory`]
//! that `inventory::inventory_bytes` produced for the same bytes. It returns one
//! [`ExtractedImage`] per photograph object, carrying the page, occurrence, and
//! source-hash provenance the import step needs.
//!
//! The module is pure and works entirely in memory. It never repacks or rewrites
//! the source PDF: it only reads the bytes and copies stream payloads out. The
//! caller keeps ownership of `bytes`, and the source buffer is never mutated.
//!
//! # Outcomes
//!
//! Every photograph object yields a record with an [`ExtractKind`] and either
//! `bytes` or a `reason`:
//!
//! - [`ExtractKind::EmbeddedJpeg`]: a `DCTDecode` stream holding a complete JPEG.
//!   The embedded stream is returned byte for byte.
//! - [`ExtractKind::EmbeddedPng`]: the decoded stream is already a PNG. The
//!   payload is returned byte for byte.
//! - [`ExtractKind::ReencodedPng`]: the stream is decoded with the `image` crate
//!   (or from raw 8-bit samples) and re-encoded to PNG.
//! - [`ExtractKind::UnsupportedFilter`]: the filter chain is not decodable.
//!   `reason` carries the filter string.
//! - [`ExtractKind::DecodeFailed`]: the stream cannot be decoded. `reason`
//!   carries the decoder error text. No placeholder bytes are returned.
//! - [`ExtractKind::NotFound`]: the object is not present in `bytes`.
//!
//! # Page crops
//!
//! `page_crop` is not implemented. No rasterizer is present and none may be
//! added, so a [`CropRequest`] yields [`ExtractKind::CropUnavailable`] with a
//! fixed reason via [`page_crop`]. This records the gap; it does not rasterize.
//!
//! # Provenance and privacy
//!
//! Every result carries `page_number`, `occurrence_ordinal`, `occurrence_count`,
//! and the SHA-256 of the exact source bytes. The bytes are returned in memory
//! and are never stored, published, or associated with a person here.

use std::io::Cursor;

use image::{DynamicImage, GrayImage, ImageFormat, ImageReader, RgbImage};
use lopdf::{Document, Object, ObjectId, Stream};

use crate::genealogy::inventory::{self, ImageObject, Inventory, CLASS_PHOTOGRAPH};
use crate::inventory::sha256_hex;

/// Fixed reason recorded when a `page_crop` is requested.
pub const CROP_UNAVAILABLE_REASON: &str =
    "page_crop is not implemented: no rasterizer is available and none may be added";

/// How an extracted photograph object was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractKind {
    /// The embedded stream is a complete JPEG, returned unchanged.
    EmbeddedJpeg,
    /// The embedded stream is a complete PNG, returned unchanged.
    EmbeddedPng,
    /// The stream was decoded and re-encoded to PNG.
    ReencodedPng,
    /// The filter chain is not a supported JPEG/PNG/raw stream.
    UnsupportedFilter,
    /// The stream could not be decoded.
    DecodeFailed,
    /// The object could not be located in the source bytes.
    NotFound,
    /// A page crop was requested but no rasterizer is available.
    CropUnavailable,
}

/// The storable result for one photograph object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedImage {
    /// 1-based page the object was drawn on.
    pub page_number: u32,
    /// 1-based draw order of the object within its page.
    pub occurrence_ordinal: i64,
    /// How many times the page draws this object.
    pub occurrence_count: i64,
    /// The object number of the source `ImageObject`.
    pub object_number: u32,
    /// The object generation of the source `ImageObject`.
    pub generation: u16,
    /// SHA-256 of the exact source bytes the object came from.
    pub source_sha256: String,
    /// How the bytes were produced.
    pub kind: ExtractKind,
    /// The storable bytes, when extraction succeeded.
    pub bytes: Option<Vec<u8>>,
    /// The explicit reason, when extraction did not produce bytes.
    pub reason: Option<String>,
}

/// A request for a rasterized page crop.
///
/// `page_crop` is recorded as unavailable. This struct only carries the page
/// number; it is never rasterized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CropRequest {
    /// The 1-based page number the crop would cover.
    pub page_number: u32,
}

/// Records that a `page_crop` is unavailable.
///
/// The module has no rasterizer, and none may be added, so this always returns
/// [`ExtractKind::CropUnavailable`] with [`CROP_UNAVAILABLE_REASON`].
pub fn page_crop(request: CropRequest) -> ExtractedImage {
    ExtractedImage {
        page_number: request.page_number,
        occurrence_ordinal: 0,
        occurrence_count: 0,
        object_number: 0,
        generation: 0,
        source_sha256: String::new(),
        kind: ExtractKind::CropUnavailable,
        bytes: None,
        reason: Some(format!(
            "page {}: {CROP_UNAVAILABLE_REASON}",
            request.page_number
        )),
    }
}

/// Extracts storable bytes for every photograph object in `inventory`.
///
/// `bytes` is the original PDF the inventory came from. Only objects classified
/// as [`inventory::CLASS_PHOTOGRAPH`] produce a record, so the returned list has
/// one entry per photograph object, in page and draw order. The source `bytes`
/// are read only and never modified.
pub fn extract_photographs(bytes: &[u8], inventory: &Inventory) -> Vec<ExtractedImage> {
    let doc = Document::load_mem(bytes).ok();
    let source_sha256 = sha256_hex(bytes);
    let mut results: Vec<ExtractedImage> = Vec::new();
    for page in &inventory.pages {
        for (index, object) in page.images.iter().enumerate() {
            if inventory::classify(object, inventory.page_count).label != CLASS_PHOTOGRAPH {
                continue;
            }
            let mut result = extract_one(doc.as_ref(), object);
            result.page_number = page.page_number;
            result.occurrence_ordinal = (index as i64) + 1;
            result.occurrence_count = object.occurrence_count;
            result.source_sha256 = source_sha256.clone();
            results.push(result);
        }
    }
    results
}

fn build(
    object: &ImageObject,
    kind: ExtractKind,
    bytes: Option<Vec<u8>>,
    reason: Option<String>,
) -> ExtractedImage {
    ExtractedImage {
        page_number: 0,
        occurrence_ordinal: 0,
        occurrence_count: object.occurrence_count,
        object_number: object.object_number,
        generation: object.generation,
        source_sha256: String::new(),
        kind,
        bytes,
        reason,
    }
}

fn extract_one(doc: Option<&Document>, object: &ImageObject) -> ExtractedImage {
    // Inline images (object number 0) carry no locatable stream bytes.
    if object.object_number == 0 {
        return build(
            object,
            ExtractKind::NotFound,
            None,
            Some("inline image has no locatable object stream".to_string()),
        );
    }

    let Some(doc) = doc else {
        return build(
            object,
            ExtractKind::NotFound,
            None,
            Some("the source bytes are not a readable PDF".to_string()),
        );
    };

    let id: ObjectId = (object.object_number, object.generation);
    let stream = match doc.get_object(id) {
        Ok(Object::Stream(stream)) => stream,
        Ok(_) => {
            return build(
                object,
                ExtractKind::NotFound,
                None,
                Some(format!(
                    "object {} {} is not a stream",
                    object.object_number, object.generation
                )),
            );
        }
        Err(_) => {
            return build(
                object,
                ExtractKind::NotFound,
                None,
                Some(format!(
                    "object {} {} is not present in the source bytes",
                    object.object_number, object.generation
                )),
            );
        }
    };

    let chain = filter_tokens(object.filters.as_deref());
    extract_from_stream(stream, &chain, object)
}

/// Tokenizes a space-separated filter chain into individual filter names.
fn filter_tokens(filters: Option<&str>) -> Vec<String> {
    match filters {
        Some(text) => text
            .split_whitespace()
            .filter(|token| !token.is_empty())
            .map(str::to_string)
            .collect(),
        None => Vec::new(),
    }
}

fn extract_from_stream(stream: &Stream, chain: &[String], object: &ImageObject) -> ExtractedImage {
    // A single JPEG filter means the payload is already a complete JPEG.
    if chain.len() == 1 && chain[0] == "DCTDecode" {
        if !is_jpeg(&stream.content) {
            return build(
                object,
                ExtractKind::DecodeFailed,
                None,
                Some("DCTDecode stream does not start with a JPEG marker".to_string()),
            );
        }
        return build(
            object,
            ExtractKind::EmbeddedJpeg,
            Some(stream.content.clone()),
            None,
        );
    }

    let decodable = chain.is_empty()
        || chain.iter().all(|filter| {
            matches!(
                filter.as_str(),
                "FlateDecode" | "LZWDecode" | "ASCII85Decode"
            )
        });
    if !decodable {
        return build(
            object,
            ExtractKind::UnsupportedFilter,
            None,
            Some(format!(
                "filter chain is not decodable to an image: {}",
                chain.join(" ")
            )),
        );
    }

    // Decode the filter chain (predictors included). For an unfiltered stream
    // this returns the payload unchanged.
    let payload = match stream.decompressed_content() {
        Ok(payload) => payload,
        Err(err) => {
            return build(
                object,
                ExtractKind::DecodeFailed,
                None,
                Some(err.to_string()),
            )
        }
    };

    // A decoded PNG payload is already storable.
    if is_png(&payload) {
        return build(object, ExtractKind::EmbeddedPng, Some(payload), None);
    }

    reencode_as_png(&payload, object)
}

fn reencode_as_png(payload: &[u8], object: &ImageObject) -> ExtractedImage {
    let reader = match ImageReader::new(Cursor::new(payload)).with_guessed_format() {
        Ok(reader) => reader,
        Err(err) => {
            return build(
                object,
                ExtractKind::DecodeFailed,
                None,
                Some(err.to_string()),
            )
        }
    };
    match reader.decode() {
        Ok(image) => encode_png(image, object),
        Err(err) => match raw_image(payload, object) {
            Some(image) => encode_png(image, object),
            None => build(
                object,
                ExtractKind::DecodeFailed,
                None,
                Some(err.to_string()),
            ),
        },
    }
}

fn encode_png(image: DynamicImage, object: &ImageObject) -> ExtractedImage {
    let mut out: Vec<u8> = Vec::new();
    if let Err(err) = image.write_to(&mut Cursor::new(&mut out), ImageFormat::Png) {
        return build(
            object,
            ExtractKind::DecodeFailed,
            None,
            Some(err.to_string()),
        );
    }
    build(object, ExtractKind::ReencodedPng, Some(out), None)
}

/// Builds an image from raw 8-bit samples when the payload is not a container.
///
/// Supports DeviceGray/CalGray, DeviceRGB/CalRGB/ICCBased/RGB, and
/// DeviceCMYK. Every other colour space or bit depth is left undecoded, so the
/// caller records a decode failure rather than guessing.
fn raw_image(payload: &[u8], object: &ImageObject) -> Option<DynamicImage> {
    if object.bits_per_component != Some(8) {
        return None;
    }
    let width = object.pixel_width?;
    let height = object.pixel_height?;
    if width <= 0 || height <= 0 {
        return None;
    }
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    let pixels = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?;
    let color_space = object.color_space.as_deref()?;

    match color_space {
        "DeviceGray" | "CalGray" | "G" => {
            if payload.len() != pixels {
                return None;
            }
            GrayImage::from_raw(width, height, payload.to_vec()).map(DynamicImage::ImageLuma8)
        }
        "DeviceRGB" | "CalRGB" | "ICCBased" | "RGB" => {
            if payload.len() != pixels.checked_mul(3)? {
                return None;
            }
            RgbImage::from_raw(width, height, payload.to_vec()).map(DynamicImage::ImageRgb8)
        }
        "DeviceCMYK" | "CMYK" => {
            let mut rgb = Vec::with_capacity(pixels.checked_mul(3)?);
            if payload.len() != pixels.checked_mul(4)? {
                return None;
            }
            for pixel in payload.as_chunks::<4>().0 {
                let c = u16::from(pixel[0]);
                let m = u16::from(pixel[1]);
                let y = u16::from(pixel[2]);
                let k = u16::from(pixel[3]);
                rgb.push((255 - c) * (255 - k) / 255);
                rgb.push((255 - m) * (255 - k) / 255);
                rgb.push((255 - y) * (255 - k) / 255);
            }
            let rgb: Vec<u8> = rgb.into_iter().map(|value| value as u8).collect();
            RgbImage::from_raw(width, height, rgb).map(DynamicImage::ImageRgb8)
        }
        _ => None,
    }
}

fn is_png(payload: &[u8]) -> bool {
    payload.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A])
}

fn is_jpeg(payload: &[u8]) -> bool {
    payload.starts_with(&[0xFF, 0xD8, 0xFF])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genealogy::inventory::PageRecord;
    use image::{ImageBuffer, Rgb};
    use lopdf::{dictionary, Object, Stream};

    fn photograph(object_number: u32, filters: Option<&str>) -> ImageObject {
        ImageObject {
            object_number,
            generation: 0,
            pixel_width: Some(100),
            pixel_height: Some(100),
            color_space: Some("DeviceRGB".to_string()),
            filters: filters.map(str::to_string),
            bits_per_component: Some(8),
            complete: true,
            completeness_reason: None,
            placement: None,
            object_name: Some("Im0".to_string()),
            tinted: false,
            tint_signal: None,
            occurrence_count: 1,
            page_frequency: 1,
            page_width: Some(612.0),
            page_height: Some(792.0),
        }
    }

    /// Builds a one-page PDF that draws `payload` as `/Im0 Do`.
    fn pdf_with_drawn_image(payload: Vec<u8>, filter: Option<&str>, compress: bool) -> Vec<u8> {
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            b"q 612 0 0 792 0 0 cm /Im0 Do Q".to_vec(),
        ));

        let mut dict = dictionary! {
            "Type" => "XObject",
            "Subtype" => "Image",
            "Width" => 100,
            "Height" => 100,
            "ColorSpace" => "DeviceRGB",
            "BitsPerComponent" => 8,
        };
        if let Some(filter) = filter {
            dict.set("Filter", filter);
        }
        let mut stream = Stream::new(dict, payload);
        if compress {
            stream.compress().expect("compress image stream");
        }
        let image_id = doc.add_object(stream);
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

    /// Builds an inventory for one page that names `object`.
    fn one_page_inventory(object: ImageObject) -> Inventory {
        Inventory {
            page_count: 1,
            image_count: 1,
            pages: vec![PageRecord {
                page_number: 1,
                readable: true,
                failure_text: None,
                images: vec![object],
                caption: None,
            }],
        }
    }

    fn jpeg_bytes() -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(8, 8, Rgb([200, 100, 50]));
        let mut out: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Jpeg)
            .unwrap();
        out
    }

    fn png_bytes() -> Vec<u8> {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> = ImageBuffer::from_pixel(8, 8, Rgb([10, 20, 30]));
        let mut out: Vec<u8> = Vec::new();
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut Cursor::new(&mut out), ImageFormat::Png)
            .unwrap();
        out
    }

    #[test]
    fn complete_jpeg_is_returned_unchanged_with_provenance() {
        let payload = jpeg_bytes();
        let bytes = pdf_with_drawn_image(payload.clone(), Some("DCTDecode"), false);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::EmbeddedJpeg);
        assert_eq!(results[0].bytes.as_deref(), Some(payload.as_slice()));
        assert_eq!(results[0].page_number, 1);
        assert_eq!(results[0].occurrence_ordinal, 1);
        assert_eq!(results[0].occurrence_count, 1);
        assert_eq!(results[0].source_sha256, sha256_hex(&bytes));
    }

    #[test]
    fn complete_png_is_returned_unchanged() {
        let payload = png_bytes();
        // A real FlateDecode stream: the payload is zlib-compressed and the
        // extractor must decompress it before seeing the PNG signature.
        let bytes = pdf_with_drawn_image(payload.clone(), None, true);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::EmbeddedPng);
        assert_eq!(results[0].bytes.as_deref(), Some(payload.as_slice()));
    }

    #[test]
    fn other_stream_is_reencoded_to_png() {
        let payload = jpeg_bytes();
        let bytes = pdf_with_drawn_image(payload, None, false);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::ReencodedPng);
        let out = results[0].bytes.as_ref().expect("reencoded bytes");
        assert_eq!(&out[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn raw_rgb_samples_are_reencoded_to_png() {
        // 100x100 DeviceRGB 8-bit raw samples, Flate-compressed by the writer.
        let payload: Vec<u8> = (0..100 * 100 * 3)
            .map(|value| (value % 251) as u8)
            .collect();
        let bytes = pdf_with_drawn_image(payload, None, true);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::ReencodedPng);
        let out = results[0].bytes.as_ref().expect("reencoded bytes");
        assert_eq!(&out[..8], &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
    }

    #[test]
    fn undecodable_stream_yields_decode_failed() {
        let bytes = pdf_with_drawn_image(b"not an image at all".to_vec(), None, false);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::DecodeFailed);
        assert!(results[0].bytes.is_none());
        assert!(results[0]
            .reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty()));
    }

    #[test]
    fn unsupported_filter_is_recorded() {
        let bytes = pdf_with_drawn_image(b"payload".to_vec(), Some("CCITTFaxDecode"), false);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::UnsupportedFilter);
        assert!(results[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("CCITTFaxDecode")));
    }

    #[test]
    fn missing_object_yields_not_found() {
        let bytes = pdf_with_drawn_image(png_bytes(), None, true);
        let mut object = photograph(999, Some("FlateDecode"));
        object.occurrence_count = 1;
        let inventory = one_page_inventory(object);
        let results = extract_photographs(&bytes, &inventory);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, ExtractKind::NotFound);
        assert!(results[0]
            .reason
            .as_deref()
            .is_some_and(|reason| !reason.is_empty()));
    }

    #[test]
    fn page_crop_is_unavailable_with_a_reason() {
        let result = page_crop(CropRequest { page_number: 3 });
        assert_eq!(result.page_number, 3);
        assert_eq!(result.kind, ExtractKind::CropUnavailable);
        assert!(result.bytes.is_none());
        assert!(result
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains(CROP_UNAVAILABLE_REASON)));
    }

    #[test]
    fn non_photograph_objects_are_skipped() {
        let bytes = pdf_with_drawn_image(jpeg_bytes(), Some("DCTDecode"), false);
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let mut inventory = inventory;
        inventory.pages[0].images[0].color_space = Some("DeviceGray".to_string());
        let results = extract_photographs(&bytes, &inventory);
        assert!(results.is_empty());
    }

    #[test]
    fn source_bytes_are_not_modified() {
        let bytes = pdf_with_drawn_image(jpeg_bytes(), Some("DCTDecode"), false);
        let before = bytes.clone();
        let inventory = inventory::inventory_bytes(&bytes).expect("inventory");
        let _ = extract_photographs(&bytes, &inventory);
        assert_eq!(bytes, before);
    }
}
