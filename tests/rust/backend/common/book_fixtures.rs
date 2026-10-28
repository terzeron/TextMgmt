//! PDF/EPUB 편집 테스트 공용 fixture. Python `tests/test_pdf_delete_pages.py`의 헬퍼와 fixture에 대응한다.
//!
//! 사용법: 테스트 파일에서 `mod common;` 다음에 아래처럼 가져온다. (`common/mod.rs`는 수정하지 않으므로 `#[path]`를 쓴다.)
//! ```ignore
//! #[path = "common/book_fixtures.rs"]
//! mod fx;
//! ```
//!
//! `BookManagerDeps`에 `FileReader`, `ImageFetcher`, `FsOps`를 주입하고 `FakeEsManager`로 색인 호출을 확인한다.
//! 구 fixture의 seam 가정은 제거했다. 저장·미리보기·재색인 검증은 현재 trait 계약을 사용한다.
//!
//! 이 파일을 쓰는 테스트 크레이트는 루트에 `mod common;`이 있어야 한다.

#![allow(dead_code)]

use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use backend::book_manager::{BookManager, BookManagerDeps, FileReader};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::es_manager::Docs;
use base64::Engine;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use lopdf::{Dictionary, Document, Object, ObjectId, Stream, StringFormat, dictionary};
use serde_json::{Value, json};
use tempfile::TempDir;

// ── mock 대역: ES, file reader, image fetcher ──

pub type FakeEs = crate::common::es_fake::FakeEsManager;

/// `Loader.read_file`에 대응한다. `ReadOutcome::item`이 Python의 `{id: 문서}`다.
pub struct FakeReadFile {
    pub docs: Mutex<Docs>,
}

impl FileReader for FakeReadFile {
    fn read_file(&self, _path: &Path, _path_prefix: Option<&Path>) -> Docs {
        self.docs.lock().unwrap().clone()
    }
}

#[derive(Clone)]
pub enum FetchBehavior {
    Ok(Vec<u8>),
    Err(FetchError),
}

pub struct FakeFetch {
    pub behavior: Mutex<FetchBehavior>,
    pub calls: Mutex<Vec<String>>,
}

impl ImageFetcher for FakeFetch {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        self.calls.lock().unwrap().push(url.to_string());
        match self.behavior.lock().unwrap().clone() {
            FetchBehavior::Ok(data) => Ok(data),
            FetchBehavior::Err(e) => Err(e),
        }
    }
}

impl FakeFetch {
    pub fn set(&self, behavior: FetchBehavior) {
        *self.behavior.lock().unwrap() = behavior;
    }
}

// ── 환경(env, image_env, pixel_env) ──

/// 어느 mock을 BookManager에 꽂을지 정한다. Python의 `loader`, `fetch` fixture 요청 여부에 대응한다.
#[derive(Default)]
pub struct Opts {
    pub loader: bool,
    pub fetch: bool,
}

impl Opts {
    pub fn plain() -> Self {
        Self::default()
    }

    pub fn loader() -> Self {
        Self { loader: true, fetch: false }
    }

    pub fn fetch() -> Self {
        Self { loader: false, fetch: true }
    }

    pub fn loader_fetch() -> Self {
        Self { loader: true, fetch: true }
    }
}

pub struct Env {
    pub bm: BookManager,
    pub es: Arc<FakeEs>,
    pub reader: Arc<FakeReadFile>,
    pub fetch: Arc<FakeFetch>,
    pub pdf: PathBuf,
    pub root: PathBuf,
    _tmp: TempDir,
}

/// `delete_pdf_pages`의 인자 묶음. Python 호출의 기본값(`None`)을 그대로 둔다.
#[derive(Default, Clone)]
pub struct Edit {
    pub pages: Vec<i64>,
    pub crops: Option<Vec<Value>>,
    pub splits: Option<Vec<Value>>,
    pub inserts: Option<Vec<Value>>,
    pub order: Option<Vec<i64>>,
}

impl Env {
    pub async fn run(&self, edit: &Edit) -> (Option<Value>, Option<String>) {
        self.bm.delete_pdf_pages(1, &edit.pages, edit.crops.as_deref(), edit.splits.as_deref(), edit.inserts.as_deref(), edit.order.as_deref()).await
    }

    /// 인자를 따로 쓰는 호출: `delete_pdf_pages(1, pages)`
    pub async fn run_pages(&self, pages: &[i64]) -> (Option<Value>, Option<String>) {
        self.run(&Edit { pages: pages.to_vec(), ..Edit::default() }).await
    }

    pub fn trash_backup(&self) -> PathBuf {
        self.root.join("0_trash").join("book.pdf")
    }

    pub fn part_files(&self) -> Vec<String> {
        part_files(self.pdf.parent().unwrap())
    }

    /// 편집이 실패했을 때 파일, 휴지통, 임시 파일, ES가 모두 그대로인지 본다. (`not (root/"0_trash").exists()` 등)
    pub fn assert_nothing_changed(&self, original: &[u8]) {
        assert_eq!(std::fs::read(&self.pdf).unwrap(), original, "PDF 바이트가 바뀌었다");
        assert!(!self.root.join("0_trash").exists(), "0_trash가 생겼다");
        assert!(self.part_files().is_empty(), "임시 .part 파일이 남았다");
        assert!(self.es.deleted().is_empty(), "ES delete가 불렸다");
    }
}

pub fn part_files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.starts_with('.') && n.ends_with(".part")).collect()
}

/// `env` fixture: 5쪽짜리 PDF(쪽마다 너비 100+i)와 mock ES/add_book을 가진 BookManager.
pub fn env(opts: Opts) -> Env {
    crate::common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    let pdf = root.join("cat").join("book.pdf");
    make_pdf(&pdf, 5);
    let doc = json!({
        "category": "cat", "title": "T", "author": "A", "file_path": "cat/book.pdf", "file_type": "pdf",
        "file_size": std::fs::metadata(&pdf).unwrap().len(), "page_count": 5, "updated_time": "2024-01-01T00:00:00.000000"
    });
    let es = FakeEs::with_doc(doc);
    let reindexed = json!({"category": "x", "title": "x", "author": "x", "file_path": "x", "file_type": "pdf", "file_size": 1, "page_count": 3});
    let reader = Arc::new(FakeReadFile { docs: Mutex::new(Docs::from([(1, reindexed)])) });
    let fetch = Arc::new(FakeFetch { behavior: Mutex::new(FetchBehavior::Ok(png_halves(200, 300))), calls: Mutex::new(Vec::new()) });

    let mut deps = BookManagerDeps::new(root.clone(), es.clone());
    if opts.loader {
        deps = deps.with_file_reader(reader.clone());
    }
    if opts.fetch {
        deps = deps.with_image_fetcher(fetch.clone());
    }
    Env { bm: BookManager::with_deps(deps), es, reader, fetch, pdf, root, _tmp: tmp }
}

/// `image_env` fixture: 이미지 쪽 3장과 글 쪽 1장.
pub fn image_env(opts: Opts) -> Env {
    let e = env(opts);
    make_image_pdf(&e.pdf, &["image", "image", "text", "image"], (100, 60));
    e
}

/// `pixel_env` fixture: Flate/JPEG 이미지 쪽 6장.
pub fn pixel_env(opts: Opts) -> Env {
    let e = env(opts);
    make_pixel_pdf(&e.pdf, &["rgb", "rgb_png", "gray", "bw", "palette", "jpeg"]);
    e
}

// ── 요청 조각 ──

pub fn crop_at(page: i64, x0: f64, y0: f64, x1: f64, y1: f64, angle: f64) -> Value {
    // serde_json은 NaN을 null로 바꾼다. 비유한(non-finite) 각도는 null로 전달된다.
    json!({"page": page, "x0": x0, "y0": y0, "x1": x1, "y1": y1, "angle": angle})
}

pub fn crop(page: i64, x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    crop_at(page, x0, y0, x1, y1, 0.0)
}

pub fn split(page: i64, first: [f64; 4], second: [f64; 4]) -> Value {
    let rect = |r: [f64; 4]| json!({"x0": r[0], "y0": r[1], "x1": r[2], "y1": r[3]});
    json!({"page": page, "first": rect(first), "second": rect(second)})
}

/// image_env의 왼쪽은 빨강, 오른쪽은 파랑이다.
pub const LEFT: [f64; 4] = [0.0, 0.0, 0.5, 1.0];
pub const RIGHT: [f64; 4] = [0.5, 0.0, 1.0, 1.0];

pub fn insert_url(page: i64, url: &str) -> Value {
    json!({"page": page, "url": url})
}

pub fn insert(page: i64) -> Value {
    insert_url(page, "https://img.example.com/a.png")
}

pub fn b64(data: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(data)
}

pub fn upload(page: i64, data: &[u8]) -> Value {
    json!({"page": page, "image_data": b64(data)})
}

/// pytest.approx(기본 상대 오차 1e-6)에 대응한다.
pub fn assert_approx(actual: f64, expected: f64, what: &str) {
    let tolerance = (1e-6 * expected.abs()).max(1e-12);
    assert!((actual - expected).abs() <= tolerance, "{what}: {actual} != {expected}");
}

// ── 이미지 바이트 ──

pub fn jpeg_bytes(img: &RgbImage, quality: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(img).unwrap();
    out
}

pub fn encode(img: DynamicImage, format: ImageFormat) -> Vec<u8> {
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, format).unwrap();
    out.into_inner()
}

/// PIL 색 이름 중 테스트가 쓰는 것.
pub fn color(name: &str) -> [u8; 3] {
    match name {
        "red" => [255, 0, 0],
        "green" => [0, 128, 0],
        "blue" => [0, 0, 255],
        "yellow" => [255, 255, 0],
        "pink" => [255, 192, 203],
        "white" => [255, 255, 255],
        other => panic!("모르는 색: {other}"),
    }
}

/// `_png()`: 위쪽 절반은 초록, 아래쪽 절반은 노랑인 PNG.
pub fn png_halves(width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(width, height, |_, y| if y < height / 2 { Rgb([0, 200, 0]) } else { Rgb([230, 230, 0]) });
    encode(DynamicImage::ImageRgb8(img), ImageFormat::Png)
}

pub fn png_solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(rgb))), ImageFormat::Png)
}

pub fn image_size(data: &[u8]) -> (u32, u32) {
    let img = image::load_from_memory(data).expect("이미지 해독");
    (img.width(), img.height())
}

/// 이미 만든 JPEG의 SOI 바로 뒤에 EXIF 방향(Orientation) APP1 조각을 끼운다.
pub fn with_exif_orientation(jpeg: &[u8], orientation: u16) -> Vec<u8> {
    let mut tiff = Vec::new();
    tiff.extend_from_slice(b"MM\x00\x2a\x00\x00\x00\x08"); // big-endian, IFD0 오프셋 8
    tiff.extend_from_slice(&1u16.to_be_bytes()); // 항목 1개
    tiff.extend_from_slice(&0x0112u16.to_be_bytes()); // Orientation
    tiff.extend_from_slice(&3u16.to_be_bytes()); // SHORT
    tiff.extend_from_slice(&1u32.to_be_bytes());
    tiff.extend_from_slice(&orientation.to_be_bytes());
    tiff.extend_from_slice(&[0, 0]);
    tiff.extend_from_slice(&0u32.to_be_bytes()); // 다음 IFD 없음
    let mut app1 = b"Exif\0\0".to_vec();
    app1.extend_from_slice(&tiff);
    let mut out = jpeg[..2].to_vec();
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&((app1.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&app1);
    out.extend_from_slice(&jpeg[2..]);
    out
}

pub fn jpeg_with_orientation(orientation: u16, width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_pixel(width, height, Rgb([10, 120, 200]));
    with_exif_orientation(&jpeg_bytes(&img, 75), orientation)
}

pub fn multipart_file(boundary: &str, field: &str, filename: &str, content_type: &str, data: &[u8]) -> Vec<u8> {
    let mut body = format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"; filename=\"{filename}\"\r\nContent-Type: {content_type}\r\n\r\n").into_bytes();
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    body
}

// ── PDF 만들기 (pikepdf/pypdf 대응) ──

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

    pub fn save(mut self, path: &Path) {
        let count = self.kids.len() as i64;
        let pages = dictionary! {"Type" => "Pages", "Kids" => self.kids, "Count" => count};
        self.doc.objects.insert(self.pages_id, Object::Dictionary(pages));
        let catalog = self.doc.add_object(dictionary! {"Type" => "Catalog", "Pages" => self.pages_id});
        self.doc.trailer.set("Root", Object::Reference(catalog));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
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

/// 픽셀 비교용 래스터. PIL 이미지에 대응한다. mode는 "RGB" | "L" | "1" | "P".
#[derive(Clone, Debug)]
pub struct Raster {
    pub mode: &'static str,
    pub w: usize,
    pub h: usize,
    pub rgb: Vec<[u8; 3]>,
}

impl Raster {
    pub fn pixel(&self, x: usize, y: usize) -> [u8; 3] {
        self.rgb[y * self.w + x]
    }

    /// L/1 모드의 `getpixel`처럼 단일 값을 돌려준다.
    pub fn gray(&self, x: usize, y: usize) -> u8 {
        self.pixel(x, y)[0]
    }

    pub fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Raster {
        let mut rgb = Vec::new();
        for y in y0..y1 {
            for x in x0..x1 {
                rgb.push(self.pixel(x, y));
            }
        }
        Raster { mode: self.mode, w: x1 - x0, h: y1 - y0, rgb }
    }
}

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

/// `_same_pixels`
pub fn same_pixels(a: &Raster, b: &Raster) -> bool {
    a.mode == b.mode && a.w == b.w && a.h == b.h && a.rgb == b.rgb
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

// ── BookManagerApi 대역 (엔드포인트 테스트의 `MagicMock`) ──

pub struct FakeManager {
    pub preview_insert_image: Mutex<(Option<Vec<u8>>, Option<String>)>,
    pub preview_insert_upload: Mutex<(Option<Vec<u8>>, Option<String>)>,
    pub delete_pdf_pages: Mutex<(Option<Value>, Option<String>)>,
    pub edit_epub_images: Mutex<(Option<Value>, Option<String>)>,
    /// (메서드 이름, 인자 목록): `await_args.args`에 대응한다.
    pub calls: Mutex<Vec<(String, Vec<Value>)>>,
}

impl FakeManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            preview_insert_image: Mutex::new((Some(b"\xff\xd8\xffJPEG".to_vec()), None)),
            preview_insert_upload: Mutex::new((Some(b"\xff\xd8\xffJPEG".to_vec()), None)),
            delete_pdf_pages: Mutex::new((Some(json!({"remaining_pages": 5})), None)),
            edit_epub_images: Mutex::new((Some(json!({"idrefs": {"inserted": [], "split": {}}})), None)),
            calls: Mutex::new(Vec::new()),
        })
    }

    pub fn args(&self, method: &str) -> Vec<Value> {
        self.calls.lock().unwrap().iter().rev().find(|(m, _)| m == method).map(|(_, a)| a.clone()).unwrap_or_else(|| panic!("{method}가 불리지 않았다"))
    }

    pub fn call_count(&self, method: &str) -> usize {
        self.calls.lock().unwrap().iter().filter(|(m, _)| m == method).count()
    }
}

#[async_trait::async_trait]
impl backend::main::BookManagerApi for FakeManager {
    async fn preview_insert_image(&self, url: &str) -> (Option<Vec<u8>>, Option<String>) {
        self.calls.lock().unwrap().push(("preview_insert_image".into(), vec![json!(url)]));
        self.preview_insert_image.lock().unwrap().clone()
    }

    async fn preview_insert_upload(&self, data: &[u8]) -> (Option<Vec<u8>>, Option<String>) {
        self.calls.lock().unwrap().push(("preview_insert_upload".into(), vec![json!(data)]));
        self.preview_insert_upload.lock().unwrap().clone()
    }

    async fn delete_pdf_pages(
        &self,
        book_id: i64,
        pages: &[i64],
        crops: Option<&[Value]>,
        splits: Option<&[Value]>,
        inserts: Option<&[Value]>,
        order: Option<&[i64]>,
    ) -> (Option<Value>, Option<String>) {
        self.calls.lock().unwrap().push(("delete_pdf_pages".into(), vec![json!(book_id), json!(pages), json!(crops), json!(splits), json!(inserts), json!(order)]));
        self.delete_pdf_pages.lock().unwrap().clone()
    }

    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> Result<Value, String> {
        self.calls.lock().unwrap().push(("edit_epub_images".into(), vec![json!(book_id), json!(crops), json!(splits), json!(deletes), json!(inserts)]));
        let (value, error) = self.edit_epub_images.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }
}
