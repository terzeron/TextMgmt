use lopdf::{Document, Object, ObjectId, dictionary};
use mupdf::pdf::{PdfDocument, PdfWriteOptions};
use mupdf::{CjkFontOrdering, Colorspace, Font, FontLoader, ImageFormat, Matrix, TextPageFlags};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
const MAX_LOPDF_BYTES: u64 = 64 * 1024 * 1024;
const MAX_JPEG_BYTES: usize = 16 * 1024 * 1024;
struct NotoCjk;
impl FontLoader for NotoCjk {
    fn load_cjk_font(&self, name: &str, ordering: CjkFontOrdering, _serif: bool) -> Option<Font> {
        static REGULAR: OnceLock<Vec<u8>> = OnceLock::new();
        static BOLD: OnceLock<Vec<u8>> = OnceLock::new();
        let bytes = if name.contains("Bold") {
            BOLD.get_or_init(|| std::fs::read("/usr/share/fonts/opentype/noto/NotoSerifCJK-Bold.ttc").unwrap_or_default())
        } else {
            REGULAR.get_or_init(|| std::fs::read("/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc").unwrap_or_default())
        };
        let index = match ordering {
            CjkFontOrdering::AdobeKorea => 1,
            CjkFontOrdering::AdobeGb => 2,
            CjkFontOrdering::AdobeCns => 3,
            _ => 0,
        };
        Font::from_bytes_with_index("Noto Serif CJK", index, bytes).ok()
    }
}
fn open(path: &Path) -> Result<PdfDocument, String> {
    std::fs::metadata(path).map_err(|e| e.to_string())?;
    let mut d = PdfDocument::open(path).map_err(|e| e.to_string())?;
    if d.needs_password().map_err(|e| e.to_string())? && !d.authenticate("").map_err(|e| e.to_string())? {
        return Err("PDF password required".into());
    }
    Ok(d)
}
fn inherited(d: &Document, mut id: ObjectId, key: &[u8]) -> Result<Option<Object>, String> {
    let mut seen = BTreeSet::new();
    loop {
        if !seen.insert(id) || seen.len() > 128 {
            return Err("cyclic PDF page tree".into());
        }
        let page = d.get_dictionary(id).map_err(|e| e.to_string())?;
        if let Ok(value) = page.get(key) {
            return Ok(Some(value.clone()));
        }
        match page.get(b"Parent").and_then(Object::as_reference) {
            Ok(parent) => id = parent,
            Err(_) => return Ok(None),
        }
    }
}
fn load_lopdf(path: &Path) -> Result<Document, String> {
    let mut d = Document::load(path).map_err(|e| e.to_string())?;
    if d.is_encrypted() {
        d.decrypt("").map_err(|e| format!("PDF password required: {e}"))?;
    }
    Ok(d)
}
fn extract_small(path: &Path, first: usize, last: usize) -> Result<Vec<u8>, String> {
    let mut d = load_lopdf(path)?;
    let pages = d.get_pages();
    if first == 0 || first > last || last > pages.len() {
        return Err("invalid PDF page range".into());
    }
    let selected: Vec<_> = pages.values().copied().skip(first - 1).take(last - first + 1).collect();
    let tree = d.new_object_id();
    for id in &selected {
        let mut page = d.get_dictionary(*id).map_err(|e| e.to_string())?.clone();
        for key in [b"Resources".as_slice(), b"MediaBox", b"CropBox", b"Rotate"] {
            if let Some(value) = inherited(&d, *id, key)? {
                page.set(key, value);
            }
        }
        page.set("Parent", tree);
        d.objects.insert(*id, Object::Dictionary(page));
    }
    d.objects.insert(tree, dictionary! {"Type"=>"Pages","Count"=>selected.len()as i64,"Kids"=>selected.iter().map(|id|Object::Reference(*id)).collect::<Vec<_>>()}.into());
    let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>tree});
    d.trailer.set("Root", catalog);
    d.trailer.remove(b"Encrypt");
    d.prune_objects();
    let mut bytes = vec![];
    d.save_to(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn extract_large(path: &Path, first: usize, last: usize) -> Result<Vec<u8>, String> {
    let mut dest = open(path)?;
    let count = dest.page_count().map_err(|e| e.to_string())? as usize;
    if first == 0 || first > last || last > count {
        return Err("invalid PDF page range".into());
    }
    dest.select_pages((first - 1..last).collect::<Vec<_>>()).map_err(|e| e.to_string())?;
    let mut catalog = dest.catalog().map_err(|e| e.to_string())?;
    for key in ["StructTreeRoot", "MarkInfo", "Outlines", "Names", "Metadata"] {
        catalog.dict_delete(key).map_err(|e| e.to_string())?;
    }
    let mut options = PdfWriteOptions::default();
    options.set_compress(true);
    options.set_garbage_level(3);
    options.set_encryption(mupdf::pdf::Encryption::None);
    let mut bytes = vec![];
    dest.write_to_with_options(&mut bytes, options).map_err(|e| e.to_string())?;
    Ok(bytes)
}
fn jpeg(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_LOPDF_BYTES {
        return Ok(None);
    }
    let d = load_lopdf(path)?;
    let Some(id) = d.get_pages().values().next().copied() else { return Ok(None) };
    let content = d.get_page_content_with_limit(id, 1024 * 1024).map_err(|e| e.to_string())?;
    let ops = lopdf::content::Content::decode(&content).map_err(|e| e.to_string())?.operations;
    if ops.iter().any(|op| !matches!(op.operator.as_str(), "q" | "Q" | "cm" | "Do")) {
        return Ok(None);
    }
    let images: Vec<_> = ops.iter().filter(|op| op.operator == "Do").collect();
    if images.len() != 1 {
        return Ok(None);
    }
    let name = images[0].operands.first().and_then(|o| o.as_name().ok()).ok_or("missing PDF image name")?;
    let Some(resources) = inherited(&d, id, b"Resources")? else { return Ok(None) };
    let resources = d.dereference(&resources).map_err(|e| e.to_string())?.1.as_dict().map_err(|e| e.to_string())?;
    let objects = resources.get_deref(b"XObject", &d).and_then(Object::as_dict).map_err(|e| e.to_string())?;
    let image = objects.get_deref(name, &d).and_then(Object::as_stream).map_err(|e| e.to_string())?;
    if image.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image")
        || image.dict.get(b"Filter").and_then(Object::as_name).ok() != Some(b"DCTDecode")
        || image.dict.has(b"SMask")
        || image.dict.has(b"Mask")
    {
        return Ok(None);
    }
    let w = image.dict.get(b"Width").and_then(Object::as_i64).unwrap_or(0);
    let h = image.dict.get(b"Height").and_then(Object::as_i64).unwrap_or(0);
    if w <= 0 || h <= 0 || w.checked_mul(h).unwrap_or(i64::MAX) > 16_000_000 || image.content.len() > MAX_JPEG_BYTES {
        return Ok(None);
    }
    // Only a single full-page image can bypass PDF composition and cropping.
    let Some(media) = inherited(&d, id, b"MediaBox")? else { return Ok(None) };
    let a = d.dereference(&media).map_err(|e| e.to_string())?.1.as_array().map_err(|e| e.to_string())?;
    let v = a.iter().map(Object::as_float).collect::<lopdf::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    if v.len() != 4 {
        return Ok(None);
    }
    let matrices: Vec<_> = ops.iter().filter(|op| op.operator == "cm").collect();
    if matrices.len() != 1 {
        return Ok(None);
    }
    let m = matrices[0].operands.iter().map(Object::as_float).collect::<lopdf::Result<Vec<_>>>().map_err(|e| e.to_string())?;
    let expected = [v[2] - v[0], 0., 0., v[3] - v[1], v[0], v[1]];
    if m.len() != 6 || m.iter().zip(expected).any(|(a, b)| (a - b).abs() > 0.01) {
        return Ok(None);
    }
    if inherited(&d, id, b"Rotate")?.is_some_and(|o| o.as_i64().unwrap_or(0) % 360 != 0) || inherited(&d, id, b"CropBox")?.is_some() {
        return Ok(None);
    }
    let decoded = mupdf::Image::from_bytes(&image.content).map_err(|e| e.to_string())?;
    if i64::from(decoded.width()) != w || i64::from(decoded.height()) != h {
        return Ok(None);
    }
    Ok(Some(image.content.clone()))
}
pub(crate) fn run(args: &[String], mut out: &mut dyn Write) -> Result<(), String> {
    let [op, path, rest @ ..] = args else { return Err("usage: tm-pdf-worker info|text|render|extract|jpeg PATH [RANGE]".into()) };
    let path = Path::new(path);
    mupdf::set_font_loader(NotoCjk);
    let num = |i: usize| -> Result<usize, String> { rest.get(i).ok_or_else(|| "missing PDF range".to_string())?.parse().map_err(|_| "invalid PDF range".to_string()) };
    match op.as_str() {
        "info" => {
            let d = open(path)?;
            writeln!(out, "Pages: {}", d.page_count().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        }
        "text" => {
            let (first, last) = (num(0)?, num(1)?);
            let d = open(path)?;
            let n = d.page_count().map_err(|e| e.to_string())? as usize;
            if first == 0 || first > last || last > n {
                return Err("invalid PDF page range".into());
            }
            for i in first - 1..last {
                let page = d.load_page(i as i32).map_err(|e| e.to_string())?;
                let text = page.to_text_page(TextPageFlags::empty()).and_then(|p| p.to_text()).map_err(|e| e.to_string())?;
                out.write_all(text.as_bytes()).and_then(|_| out.write_all(b"\x0c")).map_err(|e| e.to_string())?;
            }
        }
        "render" => {
            let (page, max) = (num(0)?, num(1)?);
            if page == 0 || max == 0 || max > 1200 {
                return Err("invalid PDF render dimensions".into());
            }
            let d = open(path)?;
            let page = d.load_page((page - 1) as i32).map_err(|e| e.to_string())?;
            let b = page.bounds().map_err(|e| e.to_string())?;
            let extent = (b.x1 - b.x0).max(b.y1 - b.y0);
            if !extent.is_finite() || extent <= 0. {
                return Err("invalid PDF page bounds".into());
            }
            let scale = (max as f32 - 0.01) / extent;
            let pix = page.to_pixmap(&Matrix::new_scale(scale, scale), &Colorspace::device_rgb(), false, true).map_err(|e| e.to_string())?;
            if pix.width() > 1200 || pix.height() > 1200 {
                return Err("PDF raster exceeds limit".into());
            }
            pix.write_to(&mut out, ImageFormat::PNM).map_err(|e| e.to_string())?;
        }
        "extract" => {
            let (first, last) = (num(0)?, num(1)?);
            let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
            let bytes = if size > MAX_LOPDF_BYTES { extract_large(path, first, last)? } else { extract_small(path, first, last)? };
            out.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        "jpeg" => {
            if let Some(bytes) = jpeg(path)? {
                out.write_all(&bytes).map_err(|e| e.to_string())?;
            }
        }
        _ => return Err("unknown PDF operation".into()),
    }
    Ok(())
}
