use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use mupdf::pdf::{PdfDocument, PdfWriteOptions};
use mupdf::{CjkFontOrdering, Colorspace, Font, FontLoader, ImageFormat, Matrix, Pixmap, TextPageFlags};
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
const MAX_LOPDF_BYTES: u64 = 64 * 1024 * 1024;
const MAX_JPEG_BYTES: usize = 16 * 1024 * 1024;
// 뷰어로 보내는 쪽 묶음의 무손실 이미지를 JPEG로 바꾼다. 웹툰 PDF는 720px 폭 한 쪽이 Flate로 4~5MB인데
// JPEG q85면 약 1/9이 된다(전생자 184화 실측). 원본 파일은 건드리지 않는다.
const VIEW_JPEG_QUALITY: u8 = 85;
// 이보다 작은 이미지 스트림은 줄여도 이득이 작아 그대로 둔다.
const MIN_RECOMPRESS_STREAM_BYTES: usize = 256 * 1024;
// 풀어 놓은 화소 상한. worker 주소 공간 상한(512MB) 안에서 원본·화소·JPEG를 함께 들고 있을 수 있게 한다.
const MAX_RECOMPRESS_PIXELS: usize = 40_000_000;
// 뷰어가 `<img>`로 보이는 쪽 이미지의 화소 상한. 일본 만화 펼침면(6010×4479)과 긴 웹툰 쪽(690×44692)이 들어간다.
const MAX_PAGE_IMAGE_PIXELS: i64 = 40_000_000;
// 이미지 한 장이 아닌 쪽을 그림으로 그릴 때의 폭과 화소 상한. 글자 쪽도 태블릿 화면 폭(약 2400px)에서 읽힌다.
const RASTER_WIDTH: f32 = 2400.;
const MAX_RASTER_PIXELS: f32 = 24_000_000.;
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
/// Flate 무손실 8비트 RGB·Gray 이미지를 JPEG로 바꾼 바이트. 대상이 아니거나 절반 아래로 줄지 않으면 `None`.
fn lossless_image_as_jpeg(stream: &Stream) -> Option<Vec<u8>> {
    let dict = &stream.dict;
    let flate = match dict.get(b"Filter").ok()? {
        Object::Name(name) => name == b"FlateDecode",
        Object::Array(filters) => filters.len() == 1 && filters[0].as_name().ok() == Some(b"FlateDecode".as_slice()),
        _ => false,
    };
    if dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image".as_slice())
        || !flate
        || dict.get(b"BitsPerComponent").and_then(Object::as_i64).ok() != Some(8)
        || ["SMask", "Mask", "Decode", "ImageMask", "SMaskInData"].iter().any(|key| dict.has(key.as_bytes()))
        || stream.content.len() < MIN_RECOMPRESS_STREAM_BYTES
    {
        return None;
    }
    let (color, components) = match dict.get(b"ColorSpace").and_then(Object::as_name).ok()? {
        b"DeviceRGB" => (jpeg_encoder::ColorType::Rgb, 3),
        b"DeviceGray" => (jpeg_encoder::ColorType::Luma, 1),
        _ => return None,
    };
    let width = u16::try_from(dict.get(b"Width").and_then(Object::as_i64).ok()?).ok().filter(|w| *w > 0)?;
    let height = u16::try_from(dict.get(b"Height").and_then(Object::as_i64).ok()?).ok().filter(|h| *h > 0)?;
    let pixels = usize::from(width) * usize::from(height);
    if pixels > MAX_RECOMPRESS_PIXELS {
        return None;
    }
    let expected = pixels * components;
    let plain = stream.decompressed_content_with_limit(expected).ok().filter(|plain| plain.len() == expected)?;
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, VIEW_JPEG_QUALITY).encode(&plain, width, height, color).ok()?;
    (jpeg.len() * 2 <= stream.content.len()).then_some(jpeg)
}
/// 쪽 묶음 PDF의 무손실 이미지를 JPEG로 바꾼다. 바꿀 것이 없거나 다시 쓰지 못하면 원본을 그대로 돌려준다.
fn shrink_lossless_images(pdf: Vec<u8>) -> Vec<u8> {
    let Ok(mut doc) = Document::load_mem(&pdf) else { return pdf };
    let mut changed = false;
    for object in doc.objects.values_mut() {
        let Object::Stream(stream) = object else { continue };
        if let Some(jpeg) = lossless_image_as_jpeg(stream) {
            stream.dict.remove(b"DecodeParms");
            stream.dict.set("Filter", "DCTDecode");
            stream.set_content(jpeg);
            changed = true;
        }
    }
    let mut out = Vec::new();
    if !changed || doc.save_to(&mut out).is_err() {
        return pdf;
    }
    out
}
/// 쪽 전체를 덮는 이미지 한 장만 그리는 쪽이면 그 이미지. 합성·마스크·자르기·회전이 있으면 `None`.
fn full_page_image(d: &Document, id: ObjectId) -> Result<Option<Stream>, String> {
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
    let Some(resources) = inherited(d, id, b"Resources")? else { return Ok(None) };
    let resources = d.dereference(&resources).map_err(|e| e.to_string())?.1.as_dict().map_err(|e| e.to_string())?;
    let objects = resources.get_deref(b"XObject", d).and_then(Object::as_dict).map_err(|e| e.to_string())?;
    let image = objects.get_deref(name, d).and_then(Object::as_stream).map_err(|e| e.to_string())?;
    if image.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Image") || image.dict.has(b"SMask") || image.dict.has(b"Mask") {
        return Ok(None);
    }
    // Only a single full-page image can bypass PDF composition and cropping.
    let Some(media) = inherited(d, id, b"MediaBox")? else { return Ok(None) };
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
    if inherited(d, id, b"Rotate")?.is_some_and(|o| o.as_i64().unwrap_or(0) % 360 != 0) || inherited(d, id, b"CropBox")?.is_some() {
        return Ok(None);
    }
    // Resources가 쪽에서 상속한 사본이라 빌려 줄 수 없다. 이미지 스트림을 복제해 돌려준다.
    Ok(Some(image.clone()))
}
fn jpeg(path: &Path) -> Result<Option<Vec<u8>>, String> {
    if std::fs::metadata(path).map_err(|e| e.to_string())?.len() > MAX_LOPDF_BYTES {
        return Ok(None);
    }
    let d = load_lopdf(path)?;
    let Some(id) = d.get_pages().values().next().copied() else { return Ok(None) };
    let Some(image) = full_page_image(&d, id)? else { return Ok(None) };
    if image.dict.get(b"Filter").and_then(Object::as_name).ok() != Some(b"DCTDecode") {
        return Ok(None);
    }
    let w = image.dict.get(b"Width").and_then(Object::as_i64).unwrap_or(0);
    let h = image.dict.get(b"Height").and_then(Object::as_i64).unwrap_or(0);
    if w <= 0 || h <= 0 || w.checked_mul(h).unwrap_or(i64::MAX) > 16_000_000 || image.content.len() > MAX_JPEG_BYTES {
        return Ok(None);
    }
    let decoded = mupdf::Image::from_bytes(&image.content).map_err(|e| e.to_string())?;
    if i64::from(decoded.width()) != w || i64::from(decoded.height()) != h {
        return Ok(None);
    }
    Ok(Some(image.content.clone()))
}
/// 쪽 이미지 원본을 화질 손실 없이 내보낸다. JPEG는 PDF 안의 바이트 그대로, 8비트 Flate RGB·Gray는 PNG로 바꾼다.
/// 다른 형식이거나 화소 상한을 넘으면 `None`.
fn original_image_file(image: &Stream) -> Result<Option<Vec<u8>>, String> {
    let dict = &image.dict;
    let filter = match dict.get(b"Filter").ok() {
        Some(Object::Name(name)) => Some(name.as_slice()),
        Some(Object::Array(filters)) if filters.len() == 1 => filters[0].as_name().ok(),
        _ => None,
    };
    let w = dict.get(b"Width").and_then(Object::as_i64).unwrap_or(0);
    let h = dict.get(b"Height").and_then(Object::as_i64).unwrap_or(0);
    if w <= 0 || h <= 0 || w.saturating_mul(h) > MAX_PAGE_IMAGE_PIXELS || dict.has(b"Decode") || dict.has(b"ImageMask") {
        return Ok(None);
    }
    match filter {
        Some(b"DCTDecode") => {
            let decoded = mupdf::Image::from_bytes(&image.content).map_err(|e| e.to_string())?;
            Ok((i64::from(decoded.width()) == w && i64::from(decoded.height()) == h).then(|| image.content.clone()))
        }
        Some(b"FlateDecode") if dict.get(b"BitsPerComponent").and_then(Object::as_i64).ok() == Some(8) => {
            let (cs, n) = match dict.get(b"ColorSpace").and_then(Object::as_name).ok() {
                Some(b"DeviceRGB") => (Colorspace::device_rgb(), 3),
                Some(b"DeviceGray") => (Colorspace::device_gray(), 1),
                _ => return Ok(None),
            };
            let (w, h) = (w as usize, h as usize);
            let row = w * n;
            let Some(plain) = image.decompressed_content_with_limit(row * h).ok().filter(|p| p.len() == row * h) else { return Ok(None) };
            let mut pix = Pixmap::new_with_w_h(&cs, w as i32, h as i32, false).map_err(|e| e.to_string())?;
            let stride = pix.stride() as usize;
            let samples = pix.samples_mut();
            for (y, src) in plain.chunks_exact(row).enumerate() {
                samples[y * stride..y * stride + row].copy_from_slice(src);
            }
            let mut png = Vec::new();
            pix.write_to(&mut png, ImageFormat::PNG).map_err(|e| e.to_string())?;
            Ok(Some(png))
        }
        _ => Ok(None),
    }
}
/// `page`쪽(1부터)이 전면 이미지 한 장이면 그 원본 이미지 파일. 큰 파일은 쪽 하나만 떼어 낸 뒤 읽는다.
fn page_image(path: &Path, page: usize) -> Result<Option<Vec<u8>>, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let single = if size > MAX_LOPDF_BYTES { extract_large(path, page, page)? } else { extract_small(path, page, page)? };
    let d = Document::load_mem(&single).map_err(|e| e.to_string())?;
    let Some(id) = d.get_pages().values().next().copied() else { return Ok(None) };
    match full_page_image(&d, id)? {
        Some(image) => original_image_file(&image),
        None => Ok(None),
    }
}
/// 이미지 한 장이 아닌 쪽을 PNG로 그린다. 폭은 `RASTER_WIDTH`, 화소는 `MAX_RASTER_PIXELS`까지다.
fn raster_png(path: &Path, page: usize) -> Result<Vec<u8>, String> {
    let d = open(path)?;
    let page = d.load_page((page - 1) as i32).map_err(|e| e.to_string())?;
    let b = page.bounds().map_err(|e| e.to_string())?;
    let (w, h) = (b.x1 - b.x0, b.y1 - b.y0);
    if !(w > 0. && h > 0. && w.is_finite() && h.is_finite()) {
        return Err("invalid PDF page bounds".into());
    }
    let scale = (RASTER_WIDTH / w).min((MAX_RASTER_PIXELS / (w * h)).sqrt());
    let pix = page.to_pixmap(&Matrix::new_scale(scale, scale), &Colorspace::device_rgb(), false, true).map_err(|e| e.to_string())?;
    let mut png = Vec::new();
    pix.write_to(&mut png, ImageFormat::PNG).map_err(|e| e.to_string())?;
    Ok(png)
}
pub(crate) fn run(args: &[String], mut out: &mut dyn Write) -> Result<(), String> {
    let [op, path, rest @ ..] = args else { return Err("usage: tm-pdf-worker info|text|render|extract|jpeg|page-image PATH [RANGE]".into()) };
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
            let bytes = shrink_lossless_images(bytes);
            out.write_all(&bytes).map_err(|e| e.to_string())?;
        }
        "page-image" => {
            // RASTER가 1이면 이미지 한 장이 아닌 쪽도 그림으로 그려 보낸다. 0이면 빈 출력으로 "아님"을 알린다.
            let (page, raster) = (num(0)?, num(1)?);
            if page == 0 || raster > 1 {
                return Err("invalid PDF page image request".into());
            }
            let bytes = match page_image(path, page)? {
                Some(bytes) => bytes,
                None if raster == 1 => raster_png(path, page)?,
                None => vec![],
            };
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
