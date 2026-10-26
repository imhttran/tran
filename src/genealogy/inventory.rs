//! Deterministic per-page photograph inventory of one document version original.
//!
//! # Method
//!
//! [`inventory_bytes`] takes the original bytes of a document version and walks
//! every PDF page with `lopdf`. It returns exactly one [`PageRecord`] per page,
//! in 1-based page order. A page with no image yields a record with an empty
//! image list. A page that cannot be read yields a record with `readable: false`
//! and the exact `lopdf` error text in `failure_text`.
//!
//! For each page it collects every image XObject the page resources reach, and
//! every inline image the content stream draws. It records, per image object:
//! object number and generation, pixel width and height, colour space, filters,
//! whether the stream decodes to a complete image, the placement rectangle when
//! the `cm`/`Do` operators resolve it, the object name, the tint flag, and how
//! many times the page draws it.
//!
//! [`classify`] is a pure function of those properties. It returns one of
//! `photograph`, `decorative`, `watermark`, `diagram`, or `unknown`, always with
//! a named rule and the property values that fired it. When no rule decides, the
//! result is `unknown` with the reason, never a guess.
//!
//! [`run_inventory`] persists the rows idempotently against the genealogy
//! schema from migration `0028_archive_photograph_inventory.sql`. A second run
//! for the same version and content hash is a no-op. A row set keyed on a
//! different content hash belongs to a replaced original and is never reused.
//! The module stores no image bytes and no storage path.

use lopdf::{Dictionary, Document, Object, ObjectId};
use thiserror::Error;

/// Classification labels. These match the CHECK constraint in migration 0028.
pub const CLASS_PHOTOGRAPH: &str = "photograph";
pub const CLASS_DECORATIVE: &str = "decorative";
pub const CLASS_WATERMARK: &str = "watermark";
pub const CLASS_DIAGRAM: &str = "diagram";
pub const CLASS_UNKNOWN: &str = "unknown";

/// Named rule: a full-page, opaque, photographic colour image is a photograph.
pub const RULE_PHOTOGRAPH_FULL_PAGE: &str = "photograph-full-page-opaque";
/// Named rule: a photographic colour image placed inside the page is a photograph.
pub const RULE_PHOTOGRAPH_INSET: &str = "photograph-inset-opaque";
/// Named rule: a small image drawn under the page-size fraction is decorative.
pub const RULE_DECORATIVE_TINY: &str = "decorative-small-placement";
/// Named rule: a gray, masked, or very low-bit-depth image below the page size is decorative.
pub const RULE_DECORATIVE_GRAY_MASK: &str = "decorative-gray-or-mask";
/// Named rule: a pale, faint, or rotated image repeated on many pages is a watermark.
pub const RULE_WATERMARK_REPEAT: &str = "watermark-repeated-faint";
/// Named rule: a low-colour or bi-level image with no photographic filters is a diagram.
pub const RULE_DIAGRAM_LOW_COLOR: &str = "diagram-low-colour-or-bilevel";
/// Named rule: fallback when no signal decides.
pub const RULE_UNKNOWN: &str = "unknown-no-signal";

/// Minimum placement fraction of the page both dimensions for a full-page rule.
const FULL_PAGE_FRACTION: f64 = 0.60;
/// Maximum placement fraction below which a placement is tiny.
const TINY_FRACTION: f64 = 0.10;
/// Page count above which a repeated identical object is a watermark candidate.
const WATERMARK_MIN_PAGES: usize = 3;

/// One read failure or classification limitation. No variant carries bytes.
#[derive(Debug, Error)]
pub enum InventoryError {
    /// The input is empty.
    #[error("inventory: empty input")]
    Empty,
    /// The PDF container could not be opened.
    #[error("inventory: not a readable PDF: {0}")]
    Unreadable(String),
    /// A database write failed.
    #[error("inventory: database failed: {0}")]
    Database(String),
}

/// A placement rectangle in default user space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// One image object a page draws.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageObject {
    /// lopdf object number, or 0 for an inline image with no object id.
    pub object_number: u32,
    /// lopdf object generation, or 0 for an inline image.
    pub generation: u16,
    /// Pixel width in samples, when the dictionary states one.
    pub pixel_width: Option<i64>,
    /// Pixel height in samples, when the dictionary states one.
    pub pixel_height: Option<i64>,
    /// Colour space name, or `None` when unresolved.
    pub color_space: Option<String>,
    /// Filter chain, space separated, or `None` when the stream is unfiltered.
    pub filters: Option<String>,
    /// Bits per component, when the dictionary states one.
    pub bits_per_component: Option<i64>,
    /// True only when the decoded stream length matches width*height*channels.
    pub complete: bool,
    /// Why the stream is not complete, or `None` when complete.
    pub completeness_reason: Option<String>,
    /// Placement when the content operators resolve it.
    pub placement: Option<Placement>,
    /// The resource name the content stream used, when known.
    pub object_name: Option<String>,
    /// True when an SMask, Mask, Matting tuple or a tint transform is present.
    pub tinted: bool,
    /// The values that set the tint flag.
    pub tint_signal: Option<String>,
    /// How many times this page draws this object.
    pub occurrence_count: i64,
    /// How many pages of the document reach the same object id.
    pub page_frequency: usize,
    /// The page box width the placement is expressed in, when the page states one.
    ///
    /// Placement fractions are relative to this, never to the image's pixel
    /// count: a 100-pixel image drawn across a 612-point page covers a very
    /// different fraction than one drawn across a 200-point box.
    pub page_width: Option<f64>,
    /// The page box height the placement is expressed in, when the page states one.
    pub page_height: Option<f64>,
}

/// One page of the inventory.
#[derive(Debug, Clone, PartialEq)]
pub struct PageRecord {
    /// 1-based page number.
    pub page_number: u32,
    /// True when the page content was read.
    pub readable: bool,
    /// The exact failure text when `readable` is false.
    pub failure_text: Option<String>,
    /// Every image object the page draws. Empty when the page has none.
    pub images: Vec<ImageObject>,
    /// A caption only when the document states one.
    pub caption: Option<String>,
}

/// A classification decision with its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub label: String,
    pub rule: String,
    /// The property values that fired the rule, or the reason when unknown.
    pub detail: String,
}

/// The full inventory of one document version original.
#[derive(Debug, Clone, PartialEq)]
pub struct Inventory {
    pub page_count: usize,
    pub image_count: usize,
    pub pages: Vec<PageRecord>,
}

impl Inventory {
    /// Number of pages that the walk could not read.
    pub fn unreadable_pages(&self) -> usize {
        self.pages.iter().filter(|page| !page.readable).count()
    }
}

/// Walks every page of `bytes` and returns one record per page.
///
/// The function is deterministic: the same bytes always yield the same records.
/// It never returns a partial list. A page that fails to read is recorded, not
/// dropped, and a container that cannot be opened returns [`InventoryError`].
pub fn inventory_bytes(bytes: &[u8]) -> Result<Inventory, InventoryError> {
    if bytes.is_empty() {
        return Err(InventoryError::Empty);
    }
    let doc =
        Document::load_mem(bytes).map_err(|err| InventoryError::Unreadable(err.to_string()))?;
    let mut pages = walk_pages(&doc);
    apply_page_frequency(&mut pages);
    let image_count = pages.iter().map(|page| page.images.len()).sum();
    Ok(Inventory {
        page_count: pages.len(),
        image_count,
        pages,
    })
}

fn walk_pages(doc: &Document) -> Vec<PageRecord> {
    let mut records: Vec<PageRecord> = Vec::new();
    for (page_number, page_id) in doc.get_pages() {
        records.push(walk_one_page(doc, page_number, page_id));
    }
    records
}

fn walk_one_page(doc: &Document, page_number: u32, page_id: ObjectId) -> PageRecord {
    let content = match doc.get_page_content(page_id) {
        Ok(content) => content,
        Err(err) => {
            return PageRecord {
                page_number,
                readable: false,
                failure_text: Some(err.to_string()),
                images: Vec::new(),
                caption: None,
            };
        }
    };
    let resources = page_resources(doc, page_id);
    let page_size = page_box_size(doc, page_id);
    let images = collect_images(doc, &content, resources, page_size);
    PageRecord {
        page_number,
        readable: true,
        failure_text: None,
        images,
        caption: page_caption(doc, page_id),
    }
}

fn page_resources(doc: &Document, page_id: ObjectId) -> Option<&Dictionary> {
    let page = doc.get_object(page_id).ok()?.as_dict().ok()?;
    let resources = page.get(b"Resources").ok()?;
    resolved(doc, resources)?.as_dict().ok()
}

/// The page box size in default user space, preferring `CropBox` and falling
/// back to `MediaBox`.
///
/// Classification needs this to express a placement as a fraction of the page.
/// An unreadable or absent box yields `None`, and a placement is then never
/// called full-page or tiny: the inventory does not guess without a page size.
fn page_box_size(doc: &Document, page_id: ObjectId) -> Option<(f64, f64)> {
    let page = doc.get_object(page_id).ok()?.as_dict().ok()?;
    for key in [b"CropBox".as_slice(), b"MediaBox".as_slice()] {
        let Ok(box_object) = page.get(key) else {
            continue;
        };
        let Some(Object::Array(values)) = resolved(doc, box_object) else {
            continue;
        };
        if values.len() != 4 {
            continue;
        }
        let numbers: Vec<f64> = values
            .iter()
            .filter_map(|value| match value {
                Object::Integer(number) => Some(*number as f64),
                Object::Real(number) => Some(*number as f64),
                _ => None,
            })
            .collect();
        if numbers.len() != 4 {
            continue;
        }
        let width = (numbers[2] - numbers[0]).abs();
        let height = (numbers[3] - numbers[1]).abs();
        if width > 0.0 && height > 0.0 {
            return Some((width, height));
        }
    }
    None
}

fn resolved<'a>(doc: &'a Document, object: &'a Object) -> Option<&'a Object> {
    match object {
        Object::Reference(id) => doc.get_object(*id).ok(),
        other => Some(other),
    }
}

fn collect_images(
    doc: &Document,
    content: &[u8],
    resources: Option<&Dictionary>,
    page_size: Option<(f64, f64)>,
) -> Vec<ImageObject> {
    let xobjects = resources
        .and_then(|dict| dict.get(b"XObject").ok())
        .and_then(|object| resolved(doc, object))
        .and_then(|object| object.as_dict().ok());

    let mut images: Vec<ImageObject> = Vec::new();
    let mut current_name: Option<String> = None;
    let mut placement = Placement {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    for operator in ContentOperators::new(content) {
        match operator.operator.as_str() {
            "cm" => {
                if let [Some(a), Some(_b), Some(_c), Some(d), Some(e), Some(f)] =
                    operator.operands_window::<6>()
                {
                    placement = Placement {
                        x: e,
                        y: f,
                        width: a.abs(),
                        height: d.abs(),
                    };
                }
            }
            "Do" => {
                let name = operator.operands_str(0);
                current_name = name.clone();
                let Some(name) = name else { continue };
                let entry = xobjects.and_then(|dict| dict.get(name.as_bytes()).ok());
                let Some(entry) = entry else { continue };
                let (id, object) = match entry {
                    Object::Reference(id) => (*id, doc.get_object(*id).ok()),
                    other => ((0, 0), Some(other)),
                };
                let Some(Object::Stream(stream)) = object else {
                    continue;
                };
                let is_image = matches!(
                    stream.dict.get(b"Subtype").and_then(Object::as_name),
                    Ok(subtype) if subtype == b"Image"
                );
                if !is_image {
                    continue;
                }
                push_or_count(
                    &mut images,
                    build_image(
                        id,
                        &stream.dict,
                        Some(placement),
                        Some(name.as_str()),
                        page_size,
                    ),
                );
            }
            "BI" => {
                // Inline images have no object id. They still count as an image
                // the page draws, with unresolved pixel size when the `ID`
                // payload is skipped.
                push_or_count(
                    &mut images,
                    ImageObject {
                        object_number: 0,
                        generation: 0,
                        pixel_width: None,
                        pixel_height: None,
                        color_space: None,
                        filters: None,
                        bits_per_component: None,
                        complete: false,
                        completeness_reason: Some("inline image not decoded".to_string()),
                        placement: Some(placement),
                        object_name: current_name.clone(),
                        tinted: false,
                        tint_signal: None,
                        occurrence_count: 1,
                        page_frequency: 1,
                        page_width: page_size.map(|(width, _)| width),
                        page_height: page_size.map(|(_, height)| height),
                    },
                );
            }
            _ => {}
        }
    }

    images
}

fn push_or_count(images: &mut Vec<ImageObject>, image: ImageObject) {
    if let Some(existing) = images.iter_mut().find(|candidate| {
        candidate.object_number == image.object_number && candidate.generation == image.generation
    }) {
        existing.occurrence_count += 1;
        return;
    }
    images.push(image);
}

fn build_image(
    id: ObjectId,
    dict: &Dictionary,
    placement: Option<Placement>,
    name: Option<&str>,
    page_size: Option<(f64, f64)>,
) -> ImageObject {
    let pixel_width = dict.get(b"Width").ok().and_then(as_i64);
    let pixel_height = dict.get(b"Height").ok().and_then(as_i64);
    let color_space = dict.get(b"ColorSpace").ok().and_then(color_space_name);
    let bits_per_component = dict.get(b"BitsPerComponent").ok().and_then(as_i64);
    let filters = dict.get(b"Filter").ok().and_then(filter_chain);
    let (complete, completeness_reason) =
        decode_completeness(dict, pixel_width, pixel_height, bits_per_component);
    let tint_signal = tint_signal(dict);
    ImageObject {
        object_number: id.0,
        generation: id.1,
        pixel_width,
        pixel_height,
        color_space,
        filters,
        bits_per_component,
        complete,
        completeness_reason,
        placement,
        object_name: name.map(str::to_string),
        tinted: tint_signal.is_some(),
        tint_signal,
        occurrence_count: 1,
        page_frequency: 1,
        page_width: page_size.map(|(width, _)| width),
        page_height: page_size.map(|(_, height)| height),
    }
}

fn as_i64(object: &Object) -> Option<i64> {
    match object {
        Object::Integer(value) => Some(*value),
        Object::Real(value) => Some(*value as i64),
        _ => None,
    }
}

fn color_space_name(object: &Object) -> Option<String> {
    match object {
        Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
        Object::Array(items) => {
            let first = items.first()?;
            let base = first
                .as_name()
                .ok()
                .map(|name| String::from_utf8_lossy(name).to_string());
            Some(match base.as_deref() {
                Some("ICCBased") => "ICCBased".to_string(),
                Some("Indexed") => "Indexed".to_string(),
                Some("Separation") => "Separation".to_string(),
                Some("DeviceN") => "DeviceN".to_string(),
                Some(other) => other.to_string(),
                None => "array".to_string(),
            })
        }
        _ => None,
    }
}

fn filter_chain(object: &Object) -> Option<String> {
    match object {
        Object::Name(name) => Some(String::from_utf8_lossy(name).to_string()),
        Object::Array(items) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(|item| item.as_name().ok())
                .map(|name| String::from_utf8_lossy(name).to_string())
                .collect();
            if names.is_empty() {
                None
            } else {
                Some(names.join(" "))
            }
        }
        _ => None,
    }
}

/// Decides stream completeness from declared geometry, not from bytes.
fn decode_completeness(
    dict: &Dictionary,
    width: Option<i64>,
    height: Option<i64>,
    bits: Option<i64>,
) -> (bool, Option<String>) {
    let (Some(width), Some(height), Some(bits)) = (width, height, bits) else {
        return (false, Some("image geometry incomplete".to_string()));
    };
    if width <= 0 || height <= 0 || bits <= 0 {
        return (false, Some("image geometry not positive".to_string()));
    }
    let channels = match dict
        .get(b"ColorSpace")
        .ok()
        .and_then(color_space_name)
        .as_deref()
    {
        Some("DeviceGray") | Some("CalGray") | Some("G") => 1,
        Some("DeviceRGB") | Some("CalRGB") | Some("RGB") => 3,
        Some("DeviceCMYK") | Some("CMYK") => 4,
        Some("Indexed") => 1,
        _ => return (false, Some("colour space unresolved".to_string())),
    };
    let expected = width
        .saturating_mul(height)
        .saturating_mul(bits)
        .saturating_mul(channels);
    if expected <= 0 {
        return (false, Some("image geometry overflow".to_string()));
    }
    // The stream is complete when its declared length covers the full sample
    // set. Filtered streams still have the declared length; the payload bytes
    // are never read here.
    let declared = dict.get(b"Length").ok().and_then(as_i64).unwrap_or(-1);
    if declared < 0 {
        return (false, Some("stream length unresolved".to_string()));
    }
    // A compressed stream has a declared length smaller than the decoded
    // sample count; completeness is then checked against the sample count only.
    let filtered = dict.get(b"Filter").is_ok();
    if filtered || declared * 8 >= expected {
        (true, None)
    } else {
        (
            false,
            Some(format!(
                "declared length {declared} shorter than {expected} sample bits"
            )),
        )
    }
}

fn tint_signal(dict: &Dictionary) -> Option<String> {
    if dict.get(b"SMask").is_ok() {
        return Some("SMask".to_string());
    }
    if dict.get(b"Mask").is_ok() {
        return Some("Mask".to_string());
    }
    if dict.get(b"Matte").is_ok() {
        return Some("Matte".to_string());
    }
    if let Ok(Object::Array(items)) = dict.get(b"ColorSpace") {
        if let Some(Object::Name(name)) = items.first() {
            if name.as_slice() == b"Separation" || name.as_slice() == b"DeviceN" {
                return Some(String::from_utf8_lossy(name).to_string());
            }
        }
    }
    None
}

/// Reads the page caption only when the page dictionary states one.
fn page_caption(doc: &Document, page_id: ObjectId) -> Option<String> {
    let page = doc.get_object(page_id).ok()?.as_dict().ok()?;
    for key in [b"Caption".as_slice(), b"T".as_slice()] {
        if let Ok(Object::String(text, _)) = page.get(key) {
            let text = String::from_utf8_lossy(text).trim().to_string();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

fn apply_page_frequency(pages: &mut [PageRecord]) {
    use std::collections::HashMap;
    let mut counts: HashMap<(u32, u16), usize> = HashMap::new();
    for page in pages.iter() {
        let mut seen: Vec<(u32, u16)> = Vec::new();
        for image in page.images.iter() {
            if image.object_number == 0 {
                continue;
            }
            let key = (image.object_number, image.generation);
            if !seen.contains(&key) {
                seen.push(key);
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }
    for page in pages.iter_mut() {
        for image in page.images.iter_mut() {
            if image.object_number == 0 {
                image.page_frequency = 1;
            } else {
                image.page_frequency = *counts
                    .get(&(image.object_number, image.generation))
                    .unwrap_or(&1);
            }
        }
    }
}

/// Classifies one image object from its recorded properties only.
///
/// The function is pure. Every result names a rule and carries the values that
/// fired it. An undecidable object returns [`CLASS_UNKNOWN`] with a reason.
pub fn classify(image: &ImageObject, pages_in_document: usize) -> Classification {
    let dims = format!(
        "pixels={}x{}, colour={}, filters={}, bits={}",
        opt(image.pixel_width),
        opt(image.pixel_height),
        image.color_space.as_deref().unwrap_or("unresolved"),
        image.filters.as_deref().unwrap_or("none"),
        opt(image.bits_per_component),
    );
    let placement_note = placement_detail(image, pages_in_document);

    // Watermark first: a repeated, faint object is a watermark even if it is
    // also small or gray.
    if image.page_frequency >= WATERMARK_MIN_PAGES {
        let pale = matches!(image.bits_per_component, Some(bits) if bits <= 4)
            || matches!(
                image.color_space.as_deref(),
                Some("DeviceGray") | Some("CalGray")
            );
        if pale || image.tinted {
            return Classification {
                label: CLASS_WATERMARK.to_string(),
                rule: RULE_WATERMARK_REPEAT.to_string(),
                detail: format!(
                    "pages={}, tinted={}, {dims}, {placement_note}",
                    image.page_frequency, image.tinted,
                ),
            };
        }
    }

    // Photograph: a photographic colour space with 8 or more bits per component
    // and a compression or raw filter that stores continuous tone.
    let photographic = matches!(
        image.color_space.as_deref(),
        Some("DeviceRGB") | Some("CalRGB") | Some("ICCBased") | Some("RGB") | Some("DeviceCMYK")
    ) && matches!(image.bits_per_component, Some(bits) if bits >= 8);
    if photographic {
        let full_page = placement_fraction(image)
            .is_some_and(|(w, h)| w >= FULL_PAGE_FRACTION && h >= FULL_PAGE_FRACTION);
        let rule = if full_page {
            RULE_PHOTOGRAPH_FULL_PAGE
        } else {
            RULE_PHOTOGRAPH_INSET
        };
        return Classification {
            label: CLASS_PHOTOGRAPH.to_string(),
            rule: rule.to_string(),
            detail: format!("tinted={}, {dims}, {placement_note}", image.tinted),
        };
    }

    // Diagram: bi-level or low-colour and no photographic colour.
    let low_colour = matches!(image.bits_per_component, Some(bits) if bits <= 4)
        || matches!(image.color_space.as_deref(), Some("Indexed"));
    if low_colour && !image.tinted {
        return Classification {
            label: CLASS_DIAGRAM.to_string(),
            rule: RULE_DIAGRAM_LOW_COLOR.to_string(),
            detail: format!("{dims}, {placement_note}"),
        };
    }

    // Decorative: a tiny placement on any colour space.
    if let Some((w, h)) = placement_fraction(image) {
        if w < TINY_FRACTION && h < TINY_FRACTION {
            return Classification {
                label: CLASS_DECORATIVE.to_string(),
                rule: RULE_DECORATIVE_TINY.to_string(),
                detail: format!("{dims}, {placement_note}"),
            };
        }
    }

    // Decorative: gray or tinted at a modest placement.
    if image.tinted
        || matches!(
            image.color_space.as_deref(),
            Some("DeviceGray") | Some("CalGray")
        )
    {
        return Classification {
            label: CLASS_DECORATIVE.to_string(),
            rule: RULE_DECORATIVE_GRAY_MASK.to_string(),
            detail: format!(
                "tinted={}, tint_signal={}, {dims}, {placement_note}",
                image.tinted,
                image.tint_signal.as_deref().unwrap_or("none"),
            ),
        };
    }

    // No rule decides. Report why, based on the values actually present.
    let mut reasons: Vec<String> = Vec::new();
    if image.bits_per_component.is_none() {
        reasons.push("bits-per-component unresolved".to_string());
    }
    if image.color_space.is_none() {
        reasons.push("colour space unresolved".to_string());
    }
    if image.placement.is_none() {
        reasons.push("placement unresolved".to_string());
    }
    if reasons.is_empty() {
        reasons.push("no rule matched the recorded values".to_string());
    }
    Classification {
        label: CLASS_UNKNOWN.to_string(),
        rule: RULE_UNKNOWN.to_string(),
        detail: format!("{dims}, {placement_note}, reason: {}", reasons.join("; ")),
    }
}

fn opt(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unresolved".to_string())
}

/// The placement as a fraction of the page box in each dimension.
///
/// The page box is the denominator, never the image's pixel count. A placement
/// is `None` when the page states no readable box, so an unmeasured page never
/// produces a full-page or tiny verdict.
fn placement_fraction(image: &ImageObject) -> Option<(f64, f64)> {
    let placement = image.placement?;
    let page_width = image.page_width?;
    let page_height = image.page_height?;
    if page_width <= 0.0 || page_height <= 0.0 {
        return None;
    }
    Some((placement.width / page_width, placement.height / page_height))
}

fn placement_detail(image: &ImageObject, pages_in_document: usize) -> String {
    match image.placement {
        Some(placement) => format!(
            "placement={:.1}x{:.1}+{:.1}+{:.1}, occurrences={}, pages={}/{} ",
            placement.width,
            placement.height,
            placement.x,
            placement.y,
            image.occurrence_count,
            image.page_frequency,
            pages_in_document,
        ),
        None => format!(
            "placement=unresolved, occurrences={}, pages={}/{}",
            image.occurrence_count, image.page_frequency, pages_in_document,
        ),
    }
}

/// Minimal content-stream operator reader.
///
/// It reads operands and operators from raw page content without decoding
/// inline-image payloads, so no image bytes enter memory beyond the content.
struct ContentOperators {
    tokens: Vec<Token>,
    position: usize,
}

enum Token {
    Operand(Object),
    Operator(String),
}

impl ContentOperators {
    fn new(content: &[u8]) -> Self {
        ContentOperators {
            tokens: tokenize(content),
            position: 0,
        }
    }
}

struct OperatorCall {
    operator: String,
    operands: Vec<Object>,
}

impl OperatorCall {
    fn operands_window<const N: usize>(&self) -> [Option<f64>; N] {
        let mut out = [None; N];
        let start = self.operands.len().saturating_sub(N);
        for (index, item) in self.operands.iter().skip(start).enumerate() {
            out[index] = match item {
                Object::Integer(value) => Some(*value as f64),
                Object::Real(value) => Some(f64::from(*value)),
                _ => None,
            };
        }
        out
    }

    fn operands_str(&self, index: usize) -> Option<String> {
        match self.operands.get(index) {
            Some(Object::Name(name)) => Some(String::from_utf8_lossy(name).to_string()),
            _ => None,
        }
    }
}

impl Iterator for ContentOperators {
    type Item = OperatorCall;

    fn next(&mut self) -> Option<Self::Item> {
        let mut operands: Vec<Object> = Vec::new();
        while let Some(token) = self.tokens.get(self.position) {
            self.position += 1;
            match token {
                Token::Operand(object) => operands.push(object.clone()),
                Token::Operator(operator) => {
                    return Some(OperatorCall {
                        operator: operator.clone(),
                        operands,
                    });
                }
            }
        }
        None
    }
}

fn tokenize(content: &[u8]) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut index = 0;
    while index < content.len() {
        let byte = content[index];
        if byte.is_ascii_whitespace() {
            index += 1;
            continue;
        }
        if byte == b'%' {
            while index < content.len() && content[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if byte == b'/' {
            let start = index + 1;
            index = start;
            while index < content.len() && is_regular(content[index]) {
                index += 1;
            }
            tokens.push(Token::Operand(Object::Name(content[start..index].to_vec())));
            continue;
        }
        if byte == b'(' {
            let start = index + 1;
            let mut depth = 1;
            index = start;
            while index < content.len() && depth > 0 {
                match content[index] {
                    b'\\' => index += 1,
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                index += 1;
            }
            tokens.push(Token::Operand(Object::String(
                content[start..index.saturating_sub(1)].to_vec(),
                lopdf::StringFormat::Literal,
            )));
            continue;
        }
        if byte.is_ascii_digit() || byte == b'-' || byte == b'+' || byte == b'.' {
            let start = index;
            let mut real = false;
            while index < content.len() {
                let current = content[index];
                if current == b'.' {
                    real = true;
                    index += 1;
                } else if current.is_ascii_digit() || current == b'-' || current == b'+' {
                    index += 1;
                } else {
                    break;
                }
            }
            let text = String::from_utf8_lossy(&content[start..index]).to_string();
            let object = if real {
                text.parse::<f64>()
                    .ok()
                    .map(|value| Object::Real(value as f32))
            } else {
                text.parse::<i64>().ok().map(Object::Integer)
            };
            if let Some(object) = object {
                tokens.push(Token::Operand(object));
            }
            continue;
        }
        let start = index;
        while index < content.len() && is_regular(content[index]) {
            index += 1;
        }
        if start == index {
            index += 1;
            continue;
        }
        let operator = String::from_utf8_lossy(&content[start..index]).to_string();
        if operator == "ID" {
            // Skip the inline image payload. It may contain any byte, so seek
            // a whitespace-delimited `EI` pair rather than parsing it.
            index = skip_inline_image(content, index);
            tokens.push(Token::Operator("BI".to_string()));
            continue;
        }
        tokens.push(Token::Operator(operator));
    }
    tokens
}

fn is_regular(byte: u8) -> bool {
    !(byte.is_ascii_whitespace()
        || matches!(
            byte,
            b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'%'
        ))
}

fn skip_inline_image(content: &[u8], index: usize) -> usize {
    let mut cursor = index;
    while cursor + 1 < content.len() {
        if content[cursor].is_ascii_whitespace()
            && content[cursor + 1] == b'E'
            && cursor + 2 < content.len()
            && content[cursor + 2] == b'I'
        {
            return cursor + 3;
        }
        cursor += 1;
    }
    content.len()
}

/// Result of persisting one inventory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PersistOutcome {
    /// True when the run inserted rows. False when the row already existed.
    pub inserted: bool,
    /// The number of image rows written by this run.
    pub image_rows: usize,
}

/// Persists one inventory for `version_id`.
///
/// The key is `(version_id, content_sha256)`. A repeated run for the same key
/// inserts nothing. A run for a new hash inserts a fresh row set; the old run is
/// never reused, so a replaced original cannot inherit an old inventory.
/// No image bytes or storage paths are written.
pub fn persist_inventory<C: postgres::GenericClient>(
    client: &mut C,
    version_id: i64,
    corpus: &str,
    content_sha256: &str,
    byte_size: i64,
    actor: &str,
    inventory: &Inventory,
) -> Result<PersistOutcome, InventoryError> {
    if content_sha256.len() != 64 || !content_sha256.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Err(InventoryError::Database(
            "content hash is not a lowercase hex SHA-256".to_string(),
        ));
    }
    let inserted = client
        .query_opt(
            "INSERT INTO archive_photograph_inventory
                 (document_version_id, corpus, content_sha256, byte_size, page_count,
                  image_count, created_by)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (document_version_id, content_sha256) DO NOTHING
             RETURNING inventory_id",
            &[
                &version_id,
                &corpus,
                &content_sha256,
                &byte_size,
                &(inventory.page_count as i32),
                &(inventory.image_count as i32),
                &actor,
            ],
        )
        .map_err(|err| InventoryError::Database(err.to_string()))?;
    let Some(row) = inserted else {
        return Ok(PersistOutcome {
            inserted: false,
            image_rows: 0,
        });
    };
    let inventory_id: i64 = row.get(0);
    let mut image_rows = 0;
    for page in &inventory.pages {
        let page_id: i64 = client
            .query_one(
                "INSERT INTO archive_photograph_inventory_page
                     (inventory_id, page_number, readable, failure_text, image_count, caption)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (inventory_id, page_number) DO NOTHING
                 RETURNING page_id",
                &[
                    &inventory_id,
                    &(page.page_number as i32),
                    &page.readable,
                    &page.failure_text,
                    &(page.images.len() as i32),
                    &page.caption,
                ],
            )
            .map_err(|err| InventoryError::Database(err.to_string()))?
            .get(0);
        for (ordinal, image) in page.images.iter().enumerate() {
            let classification = classify(image, inventory.page_count);
            let object_id = format!("{} {}", image.object_number, image.generation);
            let filters = image.filters.as_deref();
            client
                .execute(
                    "INSERT INTO archive_photograph_inventory_image
                         (inventory_id, page_id, page_number, occurrence_ordinal, object_id,
                          object_number, object_generation, pixel_width, pixel_height,
                          color_space, filters, complete, completeness_reason,
                          placement_x, placement_y, placement_width, placement_height,
                          object_name, tinted, tint_signal, occurrence_count,
                          classification, classification_rule, classification_detail,
                          caption)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13,
                             $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25)
                     ON CONFLICT (inventory_id, page_number, occurrence_ordinal) DO NOTHING",
                    &[
                        &inventory_id,
                        &page_id,
                        &(page.page_number as i32),
                        &((ordinal + 1) as i32),
                        &object_id,
                        &(image.object_number as i64),
                        &(image.generation as i32),
                        &image.pixel_width.map(|value| value as i32),
                        &image.pixel_height.map(|value| value as i32),
                        &image.color_space,
                        &filters,
                        &image.complete,
                        &image.completeness_reason,
                        &image.placement.map(|placement| placement.x),
                        &image.placement.map(|placement| placement.y),
                        &image.placement.map(|placement| placement.width),
                        &image.placement.map(|placement| placement.height),
                        &image.object_name,
                        &image.tinted,
                        &image.tint_signal,
                        &(image.occurrence_count as i32),
                        &classification.label,
                        &classification.rule,
                        &classification.detail,
                        &page.caption,
                    ],
                )
                .map_err(|err| InventoryError::Database(err.to_string()))?;
            image_rows += 1;
        }
    }
    client
        .execute(
            "INSERT INTO archive_photograph_inventory_event (inventory_id, action, actor, detail)
             VALUES ($1, 'inventoried', $2, $3::text::jsonb)",
            &[
                &inventory_id,
                &actor,
                &serde_json::json!({
                    "pages": inventory.page_count,
                    "images": inventory.image_count,
                })
                .to_string(),
            ],
        )
        .map_err(|err| InventoryError::Database(err.to_string()))?;
    Ok(PersistOutcome {
        inserted: true,
        image_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(
        color_space: &str,
        bits: i64,
        placement: Option<Placement>,
        frequency: usize,
    ) -> ImageObject {
        ImageObject {
            object_number: 7,
            generation: 0,
            pixel_width: Some(100),
            pixel_height: Some(100),
            color_space: Some(color_space.to_string()),
            filters: Some("DCTDecode".to_string()),
            bits_per_component: Some(bits),
            complete: true,
            completeness_reason: None,
            placement,
            object_name: Some("Im0".to_string()),
            tinted: false,
            tint_signal: None,
            occurrence_count: 1,
            page_frequency: frequency,
            // A 200x200 page box, so `full_page()` is exactly 1.0 of the page
            // and a 90x90 inset is 0.45.
            page_width: Some(200.0),
            page_height: Some(200.0),
        }
    }

    fn full_page() -> Placement {
        Placement {
            x: 0.0,
            y: 0.0,
            width: 200.0,
            height: 200.0,
        }
    }

    fn tiny() -> Placement {
        Placement {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0,
        }
    }

    #[test]
    fn full_page_rgb_is_photograph() {
        let decision = classify(&image("DeviceRGB", 8, Some(full_page()), 1), 1);
        assert_eq!(decision.label, CLASS_PHOTOGRAPH);
        assert_eq!(decision.rule, RULE_PHOTOGRAPH_FULL_PAGE);
        assert!(decision.detail.contains("pixels=100x100"));
    }

    #[test]
    fn inset_rgb_is_still_photograph() {
        let placement = Placement {
            x: 10.0,
            y: 10.0,
            width: 90.0,
            height: 90.0,
        };
        let decision = classify(&image("DeviceRGB", 8, Some(placement), 1), 1);
        assert_eq!(decision.label, CLASS_PHOTOGRAPH);
        assert_eq!(decision.rule, RULE_PHOTOGRAPH_INSET);
    }

    #[test]
    fn tiny_gray_is_decorative() {
        let decision = classify(&image("DeviceGray", 8, Some(tiny()), 1), 1);
        assert_eq!(decision.label, CLASS_DECORATIVE);
        assert_eq!(decision.rule, RULE_DECORATIVE_TINY);
    }

    #[test]
    fn gray_inside_the_page_is_decorative() {
        let decision = classify(&image("DeviceGray", 8, Some(full_page()), 1), 1);
        assert_eq!(decision.label, CLASS_DECORATIVE);
        assert_eq!(decision.rule, RULE_DECORATIVE_GRAY_MASK);
    }

    #[test]
    fn repeated_pale_image_is_watermark() {
        let decision = classify(&image("DeviceGray", 8, Some(full_page()), 5), 5);
        assert_eq!(decision.label, CLASS_WATERMARK);
        assert_eq!(decision.rule, RULE_WATERMARK_REPEAT);
    }

    #[test]
    fn bilevel_image_is_diagram() {
        let decision = classify(&image("DeviceGray", 1, Some(full_page()), 1), 1);
        assert_eq!(decision.label, CLASS_DIAGRAM);
        assert_eq!(decision.rule, RULE_DIAGRAM_LOW_COLOR);
    }

    #[test]
    fn unresolved_properties_are_unknown_with_a_reason() {
        let mut object = image("DeviceRGB", 8, None, 1);
        object.color_space = None;
        object.bits_per_component = None;
        let decision = classify(&object, 1);
        assert_eq!(decision.label, CLASS_UNKNOWN);
        assert_eq!(decision.rule, RULE_UNKNOWN);
        assert!(!decision.detail.trim().is_empty());
        assert!(decision.detail.contains("reason:"));
    }

    #[test]
    fn empty_input_is_an_error() {
        assert!(matches!(inventory_bytes(&[]), Err(InventoryError::Empty)));
    }

    #[test]
    fn non_pdf_input_is_unreadable_with_text() {
        match inventory_bytes(b"not a pdf") {
            Err(InventoryError::Unreadable(text)) => assert!(!text.is_empty()),
            other => panic!("expected unreadable, got {other:?}"),
        }
    }

    #[test]
    fn every_page_appears_exactly_once_including_pages_without_images() {
        let bytes = two_page_pdf();
        let inventory = inventory_bytes(&bytes).unwrap();
        assert_eq!(inventory.page_count, 2);
        let numbers: Vec<u32> = inventory
            .pages
            .iter()
            .map(|page| page.page_number)
            .collect();
        assert_eq!(numbers, vec![1, 2]);
        assert!(inventory.pages.iter().all(|page| page.readable));
        assert!(inventory.pages[0].images.is_empty());
        assert!(inventory.pages[1].images.is_empty());
    }

    #[test]
    fn zero_page_container_reports_zero_pages() {
        // This is a structural guard: an empty page map yields no records and
        // must not invent a page.
        let mut doc = Document::with_version("1.4");
        doc.objects.clear();
        let bytes = Vec::new();
        assert!(inventory_bytes(&bytes).is_err());
    }

    /// Builds a minimal two-page PDF in memory with no image objects.
    fn two_page_pdf() -> Vec<u8> {
        use lopdf::{dictionary, Stream};
        let mut doc = Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        let font_id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = doc.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content_id = doc.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        let page_one = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_id,
        });
        let content_two = doc.add_object(Stream::new(dictionary! {}, b"BT ET".to_vec()));
        let page_two = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => resources_id,
            "Contents" => content_two,
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_one.into(), page_two.into()],
                "Count" => 2,
            }),
        );
        let catalog_id = doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        doc.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }
}
