//! PDF/EPUB 편집 테스트 공용 fixture.
//!
//! 사용법: 테스트 파일에서 `mod common;` 다음에 아래처럼 가져온다. (`common/mod.rs`는 수정하지 않으므로 `#[path]`를 쓴다.)
//! ```ignore
//! #[path = "<shared까지 상대 경로>/book_fixtures.rs"]
//! mod fx;
//! ```
//!
//! `BookManagerDeps`에 `FileReader`, `ImageFetcher`, `FsOps`를 주입하고 `FakeEsManager`로 색인 호출을 확인한다.
//! 구 fixture의 seam 가정은 제거했다. 저장·미리보기·재색인 검증은 현재 trait 계약을 사용한다.
//!
//! 이 파일을 쓰는 테스트 크레이트는 루트에 `mod common;`이 있어야 한다.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use book_manager::PdfEditResult;
use book_manager::{BookManager, BookManagerDeps, FileReader};
use cover_manager::cover_management::CoverSave;
use cover_manager::direct_cover_url::{FetchError, ImageFetcher};
use cover_manager::epub_cover::CreatedIdrefs;
use image::{DynamicImage, ImageFormat, Rgb, RgbImage};
use search_manager::Docs;
use serde_json::{Value, json};
use tempfile::TempDir;
// 사본을 지우고 common 원본을 다시 내보낸다. 이 fixture를 쓰는 테스트 crate마다 일부만 쓴다.
#[allow(unused_imports)]
pub use crate::common::edits::{LEFT, RIGHT, crop, crop_at, insert, insert_url, split, upload};
#[allow(unused_imports)]
pub use crate::common::image::{assert_approx, b64, color, encode, image_size, jpeg_bytes, multipart_file, with_exif_orientation};
#[allow(unused_imports)]
pub use crate::common::pdf::{
    FLATE_H, FLATE_W, FirstImage, Info, PdfBuilder, decoded, edit_pdf, first_image, first_image_raw, first_image_size, first_xobject, first_xobject_id, first_xobject_mut, flate_source, has_xobject,
    load, make_blank_pdf, make_image_pdf, make_pdf, make_pixel_pdf, mediabox_of, mediabox_size, page_count, page_count_bytes, page_dict, page_dict_mut, page_id, pixel_image, png_up_filter,
    resources_of, srgb_profile, widths, zlib,
};
#[allow(unused_imports)]
pub use crate::common::raster::{Raster, same_pixels};

// ── mock 대역: ES, file reader, image fetcher ──

pub type FakeEs = crate::common::es_fake::FakeEsManager;

/// 파일 하나를 읽은 결과. `ReadOutcome::item`은 `{id: 문서}`다.
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

/// 어느 대역을 BookManager에 꽂을지 정한다(파일 읽기, 내려받기).
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

/// `delete_pdf_pages`의 인자 묶음.
#[derive(Default, Clone)]
pub struct Edit {
    pub pages: Vec<i64>,
    pub crops: Option<Vec<Value>>,
    pub splits: Option<Vec<Value>>,
    pub inserts: Option<Vec<Value>>,
    pub order: Option<Vec<i64>>,
}

impl Env {
    pub async fn run(&self, edit: &Edit) -> (Option<PdfEditResult>, Option<String>) {
        match self.bm.delete_pdf_pages(1, &edit.pages, edit.crops.as_deref(), edit.splits.as_deref(), edit.inserts.as_deref(), edit.order.as_deref()).await {
            Ok(value) => (Some(value), None),
            Err(error) => (None, Some(error)),
        }
    }

    /// 인자를 따로 쓰는 호출: `delete_pdf_pages(1, pages)`
    pub async fn run_pages(&self, pages: &[i64]) -> (Option<PdfEditResult>, Option<String>) {
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

/// `_png()`: 위쪽 절반은 초록, 아래쪽 절반은 노랑인 PNG.
pub fn png_halves(width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_fn(width, height, |_, y| if y < height / 2 { Rgb([0, 200, 0]) } else { Rgb([230, 230, 0]) });
    encode(DynamicImage::ImageRgb8(img), ImageFormat::Png)
}

pub fn png_solid(width: u32, height: u32, rgb: [u8; 3]) -> Vec<u8> {
    encode(DynamicImage::ImageRgb8(RgbImage::from_pixel(width, height, Rgb(rgb))), ImageFormat::Png)
}

pub fn jpeg_with_orientation(orientation: u16, width: u32, height: u32) -> Vec<u8> {
    let img = RgbImage::from_pixel(width, height, Rgb([10, 120, 200]));
    with_exif_orientation(&jpeg_bytes(&img, 75), orientation)
}

// ── BookManagerApi 대역 (엔드포인트 테스트의 `MagicMock`) ──

pub struct FakeManager {
    pub preview_insert_image: Mutex<(Option<Vec<u8>>, Option<String>)>,
    pub preview_insert_upload: Mutex<(Option<Vec<u8>>, Option<String>)>,
    pub delete_pdf_pages: Mutex<(Option<PdfEditResult>, Option<String>)>,
    pub edit_epub_images: Mutex<(Option<CoverSave>, Option<String>)>,
    /// (메서드 이름, 인자 목록): `await_args.args`에 대응한다.
    pub calls: Mutex<Vec<(String, Vec<Value>)>>,
}

impl FakeManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            preview_insert_image: Mutex::new((Some(b"\xff\xd8\xffJPEG".to_vec()), None)),
            preview_insert_upload: Mutex::new((Some(b"\xff\xd8\xffJPEG".to_vec()), None)),
            delete_pdf_pages: Mutex::new((Some(PdfEditResult { remaining_pages: 5, ..PdfEditResult::default() }), None)),
            edit_epub_images: Mutex::new((Some(CoverSave { idrefs: Some(CreatedIdrefs::default()), ..CoverSave::default() }), None)),
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
impl book_manager::BookManagerApi for FakeManager {
    async fn preview_insert_image(&self, url: &str) -> Result<Vec<u8>, String> {
        self.calls.lock().unwrap().push(("preview_insert_image".into(), vec![json!(url)]));
        let (value, error) = self.preview_insert_image.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }

    async fn preview_insert_upload(&self, data: &[u8]) -> Result<Vec<u8>, String> {
        self.calls.lock().unwrap().push(("preview_insert_upload".into(), vec![json!(data)]));
        let (value, error) = self.preview_insert_upload.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }

    async fn delete_pdf_pages(
        &self,
        book_id: i64,
        pages: &[i64],
        crops: Option<&[Value]>,
        splits: Option<&[Value]>,
        inserts: Option<&[Value]>,
        order: Option<&[i64]>,
    ) -> Result<PdfEditResult, String> {
        self.calls.lock().unwrap().push(("delete_pdf_pages".into(), vec![json!(book_id), json!(pages), json!(crops), json!(splits), json!(inserts), json!(order)]));
        let (value, error) = self.delete_pdf_pages.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }

    async fn edit_epub_images(&self, book_id: i64, crops: &[Value], splits: &[Value], deletes: &[Value], inserts: &[Value]) -> Result<CoverSave, String> {
        self.calls.lock().unwrap().push(("edit_epub_images".into(), vec![json!(book_id), json!(crops), json!(splits), json!(deletes), json!(inserts)]));
        let (value, error) = self.edit_epub_images.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }
}

/// `env`가 만든 환경의 `BookManager`를 Deps 설정을 더해 다시 만든다. `loader`가 참이면 파일 읽기 대역도 꽂는다.
pub fn with_deps(mut e: Env, loader: bool, configure: impl FnOnce(BookManagerDeps) -> BookManagerDeps) -> Env {
    let mut deps = BookManagerDeps::new(e.root.clone(), e.es.clone());
    if loader {
        deps = deps.with_file_reader(e.reader.clone());
    }
    e.bm = BookManager::with_deps(configure(deps));
    e
}
