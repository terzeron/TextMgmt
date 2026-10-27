//! PDF 만들기·읽기 도구. book_fixtures의 `PdfBuilder`/`make_pdf`/`make_blank_pdf`/`make_image_pdf`/`make_pixel_pdf`와 읽기 도우미,
//! cover_fixtures의 `write_image_pdf`/`write_blank_pdf`(lopdf `Document::with_version` 직접 조립)를 `PdfBuilder` 하나로 합쳤다.
//! cover_status의 `make_pdf(path, size)`는 `write_blank_pdf`/`write_image_pdf` 호출 조합이므로 따로 두지 않는다. 설계: docs/rust-test-seams.md 5장.
//!
//! 사용: `use common::pdf::{make_pdf, page_count, widths};` 픽셀 비교는 `common::raster::Raster`를 쓴다.
//! PDF를 가짜로 여는 `PdfOpener`/`PdfDoc`/`PdfPageOps` 대역은 이 파일에 없다(다음 단계에서 `fake_pdf.rs`).

#![allow(dead_code)]

use std::io::{Read, Write};
use std::path::Path;

use image::{ImageFormat, Rgb, RgbImage};
use lopdf::{dictionary, Dictionary, Document, Object, ObjectId, Stream, StringFormat};

use super::image::jpeg_bytes;
use super::raster::Raster;

fn name(s: &str) -> Object {
    Object::Name(s.as_bytes().to_vec())
}

fn mediabox_obj(w: i64, h: i64) -> Object {
    Object::Array(vec![Object::Integer(0), Object::Integer(0), Object::Integer(w), Object::Integer(h)])
}

pub struct PdfBuilder {
    pub doc: Document,
    pages_id: ObjectId,
    kids: Vec<Object>,
}

impl PdfBuilder {
    pub fn new() -> Self {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        Self { doc, pages_id, kids: Vec::new() }
    }

    /// 빈 `Resources`를 가진 쪽을 더한다. pikepdf `add_blank_page`와 같다.
    pub fn add_page(&mut self, w: i64, h: i64) -> ObjectId {
        let page = dictionary! {"Type" => "Page", "Parent" => self.pages_id, "MediaBox" => mediabox_obj(w, h), "Resources" => Dictionary::new()};
        let id = self.doc.add_object(page);
        self.kids.push(Object::Reference(id));
        id
    }

    pub fn page_mut(&mut self, id: ObjectId) -> &mut Dictionary {
        self.doc.get_object_mut(id).unwrap().as_dict_mut().unwrap()
    }

    pub fn content(&mut self, id: ObjectId, data: &[u8]) {
        let stream = self.doc.add_object(Stream::new(Dictionary::new(), data.to_vec()));
        self.page_mut(id).set("Contents", Object::Reference(stream));
    }

    pub fn save(self, path: &Path) {
        self.save_with(path, false);
    }

    /// `compress`가 참이면 압축을 허용한 스트림을 Flate로 줄여 저장한다. lopdf `Document::compress`.
    pub fn save_with(mut self, path: &Path, compress: bool) {
        let count = self.kids.len() as i64;
        let pages = dictionary! {"Type" => "Pages", "Kids" => self.kids, "Count" => count};
        self.doc.objects.insert(self.pages_id, Object::Dictionary(pages));
        let catalog = self.doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => self.pages_id});
        self.doc.trailer.set("Root", Object::Reference(catalog));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        if compress {
            self.doc.compress();
        }
        self.doc.save(path).unwrap();
    }
}

/// `_make_pdf`: 쪽마다 크기를 달리해 어느 쪽이 남았는지 구분한다. 너비 100+i, 높이 200.
pub fn make_pdf(path: &Path, num_pages: usize) {
    let mut b = PdfBuilder::new();
    for i in 0..num_pages {
        b.add_page(100 + i as i64, 200);
    }
    b.save(path);
}

/// `create_test_pdf`: 612x792 빈 쪽 `num_pages`장.
pub fn make_blank_pdf(path: &Path, num_pages: usize) {
    let mut b = PdfBuilder::new();
    for _ in 0..num_pages {
        b.add_page(612, 792);
    }
    b.save(path);
}

/// 이미지 없는 빈 한 쪽짜리 PDF. Python `PdfWriter().add_blank_page(width, height)`. cover_fixtures의 `write_blank_pdf`.
pub fn write_blank_pdf(path: &Path, width: i64, height: i64) {
    let mut b = PdfBuilder::new();
    b.add_page(width, height);
    b.save(path);
}

/// 이미지 한 장당 한 쪽인 PDF. 쪽 크기는 픽셀 크기와 같다. Python `Image.save(path, "PDF", resolution=72)`. cover_fixtures의 `write_image_pdf`.
pub fn write_image_pdf(path: &Path, pages: &[RgbImage]) {
    let mut b = PdfBuilder::new();
    for image in pages {
        let (width, height) = image.dimensions();
        let page = b.add_page(width as i64, height as i64);
        let dict = dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => width as i64, "Height" => height as i64, "ColorSpace" => name("DeviceRGB"), "BitsPerComponent" => 8};
        attach_image(&mut b, page, Stream::new(dict, image.as_raw().clone()), width as i64, height as i64);
    }
    b.save_with(path, true);
}

fn image_stream(w: i64, h: i64, color_space: Object, bpc: i64, filter: Option<&str>, data: Vec<u8>) -> Stream {
    let mut dict = dictionary! {"Type" => "XObject", "Subtype" => "Image", "Width" => w, "Height" => h, "ColorSpace" => color_space, "BitsPerComponent" => bpc};
    if let Some(f) = filter {
        dict.set("Filter", name(f));
    }
    let mut stream = Stream::new(dict, data);
    stream.allows_compression = false;
    stream
}

fn attach_image(b: &mut PdfBuilder, page: ObjectId, image: Stream, w: i64, h: i64) {
    let image_id = b.doc.add_object(image);
    let resources = dictionary! {"XObject" => dictionary! {"Im0" => Object::Reference(image_id)}};
    b.page_mut(page).set("Resources", Object::Dictionary(resources));
    b.content(page, format!("q {w} 0 0 {h} 0 0 cm /Im0 Do Q").as_bytes());
}

/// `_make_image_pdf`: 쪽마다 왼쪽 절반은 빨강, 오른쪽 절반은 파랑인 JPEG 한 장짜리 PDF.
/// kinds 원소: "image" | "text"(이미지 없는 쪽) | "rotated"(회전된 이미지 쪽)
pub fn make_image_pdf(path: &Path, kinds: &[&str], size: (u32, u32)) {
    let (width, height) = size;
    let source = RgbImage::from_fn(width, height, |x, _| if x < width / 2 { Rgb([255, 0, 0]) } else { Rgb([0, 0, 255]) });
    let jpeg = jpeg_bytes(&source, 95);
    let mut b = PdfBuilder::new();
    for kind in kinds {
        let page = b.add_page(width as i64, height as i64);
        if *kind == "text" {
            b.content(page, b"BT /F1 12 Tf 10 10 Td (hello) Tj ET");
            continue;
        }
        let image = image_stream(width as i64, height as i64, name("DeviceRGB"), 8, Some("DCTDecode"), jpeg.clone());
        attach_image(&mut b, page, image, width as i64, height as i64);
        if *kind == "rotated" {
            b.page_mut(page).set("Rotate", Object::Integer(90));
        }
    }
    b.save(path);
}

pub const FLATE_W: usize = 100;
pub const FLATE_H: usize = 60;

/// `_flate_source`: 왼쪽 절반과 오른쪽 절반이 구분되는 원본.
pub fn flate_source(kind: &str) -> Raster {
    let pick = |left: [u8; 3], right: [u8; 3]| -> Vec<[u8; 3]> { (0..FLATE_W * FLATE_H).map(|i| if i % FLATE_W < FLATE_W / 2 { left } else { right }).collect() };
    match kind {
        "rgb" | "rgb_png" => Raster { mode: "RGB", w: FLATE_W, h: FLATE_H, rgb: pick([255, 0, 0], [0, 0, 255]) },
        "gray" => Raster { mode: "L", w: FLATE_W, h: FLATE_H, rgb: pick([60; 3], [200; 3]) },
        "bw" => Raster { mode: "1", w: FLATE_W, h: FLATE_H, rgb: pick([0; 3], [255; 3]) },
        "palette" => Raster { mode: "P", w: FLATE_W, h: FLATE_H, rgb: pick([255, 0, 0], [0, 0, 255]) },
        other => panic!("모르는 kind: {other}"),
    }
}

/// `Image.tobytes()`에 대응하는 원본 바이트.
fn flate_raw(kind: &str) -> Vec<u8> {
    let src = flate_source(kind);
    match src.mode {
        "RGB" => src.rgb.iter().flatten().copied().collect(),
        "L" => src.rgb.iter().map(|p| p[0]).collect(),
        // 1비트: 행마다 바이트 경계로 패딩한다.
        "1" => {
            let row_bytes = FLATE_W.div_ceil(8);
            let mut out = vec![0u8; row_bytes * FLATE_H];
            for y in 0..FLATE_H {
                for x in 0..FLATE_W {
                    if src.pixel(x, y)[0] != 0 {
                        out[y * row_bytes + x / 8] |= 0x80 >> (x % 8);
                    }
                }
            }
            out
        }
        "P" => (0..FLATE_W * FLATE_H).map(|i| u8::from(i % FLATE_W >= FLATE_W / 2)).collect(),
        _ => unreachable!(),
    }
}

pub fn zlib(data: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

/// `_png_up_filter`: PDF /Predictor 15(PNG) 용으로 각 행을 Up 필터로 바꾼다.
pub fn png_up_filter(raw: &[u8], row_bytes: usize) -> Vec<u8> {
    let mut out = Vec::new();
    let mut previous = vec![0u8; row_bytes];
    for row in raw.chunks(row_bytes) {
        out.push(2);
        out.extend(row.iter().zip(&previous).map(|(v, p)| v.wrapping_sub(*p)));
        previous = row.to_vec();
    }
    out
}

/// `_make_pixel_pdf`: 이미지 한 장이 쪽 전체를 덮는 PDF.
/// kind: rgb, rgb_png(PNG 예측기), gray, bw(1비트), palette(Indexed), jpeg, cmyk, smask, jbig2, corrupt
pub fn make_pixel_pdf(path: &Path, kinds: &[&str]) {
    let (w, h) = (FLATE_W as i64, FLATE_H as i64);
    let mut b = PdfBuilder::new();
    for kind in kinds {
        let page = b.add_page(w, h);
        let image = match *kind {
            "jpeg" => {
                let src = flate_source("rgb");
                let img = RgbImage::from_fn(FLATE_W as u32, FLATE_H as u32, |x, y| Rgb(src.pixel(x as usize, y as usize)));
                image_stream(w, h, name("DeviceRGB"), 8, Some("DCTDecode"), jpeg_bytes(&img, 95))
            }
            "cmyk" => image_stream(w, h, name("DeviceCMYK"), 8, Some("FlateDecode"), zlib(&vec![0u8; FLATE_W * FLATE_H * 4])),
            "jbig2" => image_stream(w, h, name("DeviceGray"), 1, Some("JBIG2Decode"), vec![0u8; 16]),
            "corrupt" => image_stream(w, h, name("DeviceRGB"), 8, Some("FlateDecode"), b"this is not zlib data at all".to_vec()),
            other => {
                let base = if other == "smask" { "rgb" } else { other };
                let source = flate_source(base);
                let mut image = if other == "rgb_png" {
                    let mut s = image_stream(w, h, name("DeviceRGB"), 8, Some("FlateDecode"), zlib(&png_up_filter(&flate_raw(base), FLATE_W * 3)));
                    s.dict.set("DecodeParms", dictionary! {"Predictor" => 15, "Colors" => 3, "BitsPerComponent" => 8, "Columns" => FLATE_W as i64});
                    s
                } else {
                    let data = zlib(&flate_raw(base));
                    match source.mode {
                        "P" => {
                            let palette = Object::String(vec![255, 0, 0, 0, 0, 255], StringFormat::Hexadecimal);
                            let cs = Object::Array(vec![name("Indexed"), name("DeviceRGB"), Object::Integer(1), palette]);
                            image_stream(w, h, cs, 8, Some("FlateDecode"), data)
                        }
                        "RGB" => image_stream(w, h, name("DeviceRGB"), 8, Some("FlateDecode"), data),
                        "L" => image_stream(w, h, name("DeviceGray"), 8, Some("FlateDecode"), data),
                        _ => image_stream(w, h, name("DeviceGray"), 1, Some("FlateDecode"), data),
                    }
                };
                if other == "smask" {
                    let mask = image_stream(w, h, name("DeviceGray"), 8, Some("FlateDecode"), zlib(&vec![255u8; FLATE_W * FLATE_H]));
                    let mask_id = b.doc.add_object(mask);
                    image.dict.set("SMask", Object::Reference(mask_id));
                }
                image
            }
        };
        attach_image(&mut b, page, image, w, h);
    }
    b.save(path);
}

// ── PDF 읽기 (pikepdf/pypdf 대응) ──

pub fn load(path: &Path) -> Document {
    Document::load(path).unwrap()
}

fn deref<'a>(doc: &'a Document, obj: &'a Object) -> &'a Object {
    doc.dereference(obj).map(|(_, o)| o).unwrap_or(obj)
}

pub fn page_dict(doc: &Document, page_no: usize) -> &Dictionary {
    let id = *doc.get_pages().get(&(page_no as u32)).expect("쪽 번호");
    doc.get_dictionary(id).unwrap()
}

pub fn page_count(path: &Path) -> usize {
    load(path).get_pages().len()
}

pub fn page_count_bytes(data: &[u8]) -> usize {
    Document::load_mem(data).unwrap().get_pages().len()
}

pub fn mediabox_of(doc: &Document, page_no: usize) -> [f64; 4] {
    let arr = deref(doc, page_dict(doc, page_no).get(b"MediaBox").unwrap()).as_array().unwrap();
    let v: Vec<f64> = arr.iter().map(|o| deref(doc, o).as_float().unwrap() as f64).collect();
    [v[0], v[1], v[2], v[3]]
}

/// `_widths`: 각 쪽 MediaBox의 너비.
pub fn widths(path: &Path) -> Vec<i64> {
    let doc = load(path);
    (1..=doc.get_pages().len()).map(|n| mediabox_of(&doc, n)).map(|b| (b[2] - b[0]) as i64).collect()
}

/// `_mediabox_size`
pub fn mediabox_size(path: &Path, page_no: usize) -> (f64, f64) {
    let b = mediabox_of(&load(path), page_no);
    (b[2] - b[0], b[3] - b[1])
}

pub fn resources_of(doc: &Document, page_no: usize) -> &Dictionary {
    deref(doc, page_dict(doc, page_no).get(b"Resources").unwrap()).as_dict().unwrap()
}

/// 쪽 Resources에 XObject가 있는가. (`"/XObject" in page.Resources`)
pub fn has_xobject(doc: &Document, page_no: usize) -> bool {
    resources_of(doc, page_no).has(b"XObject")
}

pub fn first_xobject(doc: &Document, page_no: usize) -> &Stream {
    let xobjects = deref(doc, resources_of(doc, page_no).get(b"XObject").unwrap()).as_dict().unwrap();
    let (_, value) = xobjects.iter().next().expect("XObject가 비었다");
    deref(doc, value).as_stream().unwrap()
}

fn decode_jpeg(data: &[u8]) -> RgbImage {
    image::load_from_memory_with_format(data, ImageFormat::Jpeg).expect("JPEG 해독").to_rgb8()
}

/// `_decoded`: 쪽의 첫 이미지(JPEG 원본 바이트)를 RGB로 해독한다.
pub fn decoded(path: &Path, page_no: usize) -> RgbImage {
    let doc = load(path);
    decode_jpeg(&first_xobject(&doc, page_no).content)
}

pub struct FirstImage {
    pub width: i64,
    pub height: i64,
    pub center: [u8; 3],
    pub box_w: f64,
}

/// `_first_image`: (너비, 높이, 중앙 픽셀 RGB, MediaBox 너비)
pub fn first_image(path: &Path, page_no: usize) -> FirstImage {
    let doc = load(path);
    let image = first_xobject(&doc, page_no);
    let decoded = decode_jpeg(&image.content);
    let b = mediabox_of(&doc, page_no);
    FirstImage {
        width: image.dict.get(b"Width").unwrap().as_i64().unwrap(),
        height: image.dict.get(b"Height").unwrap().as_i64().unwrap(),
        center: decoded.get_pixel(decoded.width() / 2, decoded.height() / 2).0,
        box_w: b[2] - b[0],
    }
}

pub fn first_image_size(path: &Path, page_no: usize) -> (i64, i64) {
    let i = first_image(path, page_no);
    (i.width, i.height)
}

/// `read_raw_bytes()`: 필터를 풀지 않은 스트림 바이트.
pub fn first_image_raw(path: &Path, page_no: usize) -> Vec<u8> {
    let doc = load(path);
    first_xobject(&doc, page_no).content.clone()
}

pub struct Info {
    pub filter: String,
    pub bpc: i64,
    pub cs: String,
}

fn name_str(obj: &Object) -> String {
    match obj {
        Object::Name(n) => format!("/{}", String::from_utf8_lossy(n)),
        Object::Array(items) => format!("[{}]", items.iter().map(name_str).collect::<Vec<_>>().join(", ")),
        _ => "?".to_string(),
    }
}

fn unpredict(data: Vec<u8>, predictor: i64, colors: usize, bpc: usize, columns: usize) -> Vec<u8> {
    if predictor < 10 {
        return data;
    }
    let row_bytes = (colors * bpc * columns).div_ceil(8);
    let bpp = (colors * bpc).div_ceil(8).max(1);
    let mut out: Vec<u8> = Vec::new();
    let mut prev = vec![0u8; row_bytes];
    for chunk in data.chunks(row_bytes + 1) {
        let kind = chunk[0];
        let mut row = chunk[1..].to_vec();
        row.resize(row_bytes, 0);
        for i in 0..row_bytes {
            let a = if i >= bpp { row[i - bpp] as i32 } else { 0 };
            let b = prev[i] as i32;
            let c = if i >= bpp { prev[i - bpp] as i32 } else { 0 };
            let add = match kind {
                0 => 0,
                1 => a,
                2 => b,
                3 => (a + b) / 2,
                _ => {
                    let p = a + b - c;
                    let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
            };
            row[i] = row[i].wrapping_add(add as u8);
        }
        out.extend_from_slice(&row);
        prev = row;
    }
    out
}

/// `_pixel_image`: (래스터, 필터/비트수/색공간 정보). 어떤 필터든 해독한다.
pub fn pixel_image(path: &Path, page_no: usize) -> (Raster, Info) {
    let doc = load(path);
    let x = first_xobject(&doc, page_no);
    let d = &x.dict;
    let (w, h) = (d.get(b"Width").unwrap().as_i64().unwrap() as usize, d.get(b"Height").unwrap().as_i64().unwrap() as usize);
    let bpc = d.get(b"BitsPerComponent").unwrap().as_i64().unwrap();
    let filter = d.get(b"Filter").map(|o| name_str(deref(&doc, o))).unwrap_or_else(|_| "None".to_string());
    let cs_obj = deref(&doc, d.get(b"ColorSpace").unwrap());
    let cs = match cs_obj {
        Object::Name(_) => name_str(cs_obj),
        _ => "array".to_string(),
    };
    let info = Info { filter: filter.clone(), bpc, cs: cs.clone() };

    if filter.contains("DCTDecode") {
        let img = decode_jpeg(&x.content);
        let rgb = img.pixels().map(|p| p.0).collect();
        return (Raster { mode: "RGB", w: img.width() as usize, h: img.height() as usize, rgb }, info);
    }

    let mut raw = Vec::new();
    flate2::read::ZlibDecoder::new(&x.content[..]).read_to_end(&mut raw).expect("Flate 해독");
    let parms = d.get(b"DecodeParms").ok().map(|o| deref(&doc, o)).and_then(|o| o.as_dict().ok());
    let get = |key: &[u8], default: i64| parms.and_then(|p| p.get(key).ok()).and_then(|o| o.as_i64().ok()).unwrap_or(default);
    let channels = match cs_obj {
        Object::Name(n) if n == b"DeviceRGB" => 3,
        Object::Name(_) => 1,
        Object::Array(items) => {
            let profile = deref(&doc, &items[1]).as_stream().unwrap();
            profile.dict.get(b"N").unwrap().as_i64().unwrap() as usize
        }
        _ => unreachable!(),
    };
    let raw = unpredict(raw, get(b"Predictor", 1), get(b"Colors", channels as i64) as usize, get(b"BitsPerComponent", bpc) as usize, get(b"Columns", w as i64) as usize);
    let (mode, rgb): (&'static str, Vec<[u8; 3]>) = match (channels, bpc) {
        (3, 8) => ("RGB", raw.chunks(3).take(w * h).map(|p| [p[0], p[1], p[2]]).collect()),
        (1, 8) => ("L", raw.iter().take(w * h).map(|&v| [v; 3]).collect()),
        (1, 1) => {
            let row_bytes = w.div_ceil(8);
            let mut px = Vec::new();
            for y in 0..h {
                for xx in 0..w {
                    px.push(if raw[y * row_bytes + xx / 8] & (0x80 >> (xx % 8)) != 0 { [255; 3] } else { [0; 3] });
                }
            }
            ("1", px)
        }
        other => panic!("지원하지 않는 조합: {other:?}"),
    };
    (Raster { mode, w, h, rgb }, info)
}

/// `ImageCms.createProfile("sRGB")` 대신 쓰는 최소 ICC 프로파일(128바이트 헤더, 태그 0개).
/// 구현이 헤더의 채널 수가 아니라 스트림의 /N만 본다고 가정한다.
pub fn srgb_profile() -> Vec<u8> {
    let mut p = vec![0u8; 132];
    p[0..4].copy_from_slice(&132u32.to_be_bytes());
    p[8] = 2; // 버전 2.x
    p[12..16].copy_from_slice(b"mntr");
    p[16..20].copy_from_slice(b"RGB ");
    p[20..24].copy_from_slice(b"XYZ ");
    p[36..40].copy_from_slice(b"acsp");
    p
}

/// `_edit_pdf`: PDF를 열어 edit로 고친 뒤 같은 경로에 저장한다.
pub fn edit_pdf(path: &Path, edit: impl FnOnce(&mut Document)) {
    let mut doc = Document::load(path).unwrap();
    edit(&mut doc);
    doc.save(path).unwrap();
}

pub fn page_id(doc: &Document, page_no: usize) -> ObjectId {
    *doc.get_pages().get(&(page_no as u32)).expect("쪽 번호")
}

pub fn page_dict_mut(doc: &mut Document, page_no: usize) -> &mut Dictionary {
    let id = page_id(doc, page_no);
    doc.get_object_mut(id).unwrap().as_dict_mut().unwrap()
}

/// 쪽의 첫 이미지 XObject의 객체 ID.
pub fn first_xobject_id(doc: &Document, page_no: usize) -> ObjectId {
    let xobjects = deref(doc, resources_of(doc, page_no).get(b"XObject").unwrap()).as_dict().unwrap();
    xobjects.iter().next().and_then(|(_, v)| v.as_reference().ok()).expect("XObject 참조")
}

pub fn first_xobject_mut(doc: &mut Document, page_no: usize) -> &mut Stream {
    let id = first_xobject_id(doc, page_no);
    doc.get_object_mut(id).unwrap().as_stream_mut().unwrap()
}
