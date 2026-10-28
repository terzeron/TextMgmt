//! 원본: tests/test_book_manager.py (전반부: 파일 처음부터 `test_validate_pdf_producer_and_creation_date` 직전까지)
//!
//! 설계: docs/rust-test-seams.md 3장(통일안). 공용 대역은 `tests/rust/backend/common/`(es_fake, faulty_fs, runner, zipx, epub, fsx, clock)를 쓴다.
//!
//! 가정한 API (backend::book_manager 등은 아직 구현 전이다. 시그니처는 Python과 같다고 본다):
//! - `backend::book::Book { book_id: i64, category: String, title: String, author: String, file_type: String, file_path: PathBuf, file_size: i64, updated_time: NaiveDateTime }`
//! - `backend::book_manager::{BookManager, BookManagerDeps}`
//!   - 생성: `BookManager::with_deps(BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>).with_*(..)) -> BookManager` (구 `for_test`. 전역 `Book.path_prefix`를 건드리지 않고 매니저가 들고 있는다)
//!   - 생성: `BookManager::from_env(env: &dyn Fn(&str) -> Option<String>, es_factory: &dyn Fn(&str) -> Arc<dyn EsManagerApi>) -> Result<BookManager, RuntimeError>` (구 `new_from_env`/`new_with`/`with_es_url`)
//!   - `BookManagerDeps` 빌더(소비형, 3.12): `with_fs(Arc<dyn FsOps>)`, `with_runner(Arc<dyn CommandRunner>)`, `with_pdf_opener(Arc<dyn PdfOpener>)`, `with_pdf_pages(Arc<dyn PdfPageOps>)`,
//!     `with_file_reader(Arc<dyn FileReader>)`, `with_bulk_reindex_batch_size(usize)`, `with_scan_observer(Arc<dyn Fn() + Send + Sync>)`, `with_classifier_factory(Arc<dyn Fn() -> Arc<dyn Classify> + Send + Sync>)`
//!   - 문서 3.12에 없는 seam(3.0 규칙대로 `with_*`로 옮겨 가정한 것. 확정 필요): `with_get_category_mismatches_override(Arc<dyn Fn(bool) -> Value + Send + Sync>)`,
//!     `with_get_category_mismatch_details_override(Arc<dyn Fn(&str) -> Value + Send + Sync>)`, `with_iter_category_indexable_files_override(Arc<dyn Fn(&str, bool) -> Vec<PathBuf> + Send + Sync>)`,
//!     `with_is_high_confidence_override(Arc<dyn Fn(&dyn Any, f64) -> bool + Send + Sync>)`
//!   - `backend::book_manager::FileReader { fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs }` (구 `ReadFile`/`set_read_file`/`Loader.read_file`)
//!   - 상수: `CACHE_MAX_AGE_SECONDS: u64`, `EPUB_IMAGE_MAX_EDGE: u32`
//!   - 비동기(`async fn`, `&self`): `get_categories() -> (BTreeMap<String, i64>, Option<String>)`, `get_books_in_category(&str) -> (Vec<Book>, Option<String>)`,
//!     `get_book(i64) -> (Option<Book>, Option<String>)`, `get_book_content(i64) -> BookContent`, `search_by_keyword(&str, max_result_count: i64) -> (Vec<Book>, Option<String>)`,
//!     `search_similar_books(i64, max_result_count: i64) -> (Vec<Book>, Option<String>)`, `search_similar_books_paged(i64, size: i64, offset: i64) -> (Vec<Book>, i64, Option<String>)`,
//!     `search_by_keyword_paged(&str, size: i64, offset: i64, exclude_categories: Option<&[String]>, category: Option<&str>) -> (Vec<Book>, i64, Option<String>)`,
//!     `add_book(&Docs) -> (Option<i64>, Option<String>)`, `update_book(i64, &str, &str, &str, &Path, &str) -> (String, Option<String>)`,
//!     `delete_book(i64) -> (String, Option<String>)`, `delete_file(&str) -> (String, Option<String>)`, `index_single_file(&str) -> (Option<i64>, Option<String>)`,
//!     `rename_category(&str, &str) -> (Value, Option<String>)`, `delete_category(&str, delete_files: bool) -> (Value, Option<String>)`, `preview_delete_category(&str) -> (Value, Option<String>)`,
//!     `reload_category(&str, content_type: Option<&str>) -> (Value, Option<String>)`, `reload_category_mismatches(on_progress: Option<&dyn Fn(Value)>) -> (Value, Option<String>)`,
//!     `reload_category_mismatch_files(&str, on_progress: Option<&dyn Fn(Value)>) -> (Value, Option<String>)`, `_reload_category_mismatch_details(&str, &Value) -> Value`,
//!     `_bulk_index_files(&[&str], clean_existing: bool) -> (BTreeMap<String, i64>, Vec<Value>)`,
//!     `propose_category_changes(&str, mappings: Option<&Value>, ProposeOptions) -> (Value, Option<String>)`, `apply_category_changes(&[Value], &HashSet<String>, on_progress: Option<&dyn Fn(Value)>) -> (Value, Option<String>)`,
//!     `get_book_preview(i64, pages: Option<u32>, chapters: Option<u32>) -> BookResponse`, `get_pdf_pages(i64, start: u32, end: u32) -> BookResponse`,
//!     `validate_epub(i64) -> (Option<Value>, Option<String>)`, `validate_pdf(i64) -> (Option<Value>, Option<String>)`
//!   - 동기: `path_prefix() -> PathBuf`, `set_path_prefix(&mut self, &Path)`, `es_manager() -> &dyn EsManagerApi`, `get_category_mismatches(force_refresh: bool) -> Value`,
//!     `get_category_mismatch_details(&str) -> Value`, `fill_missing_book_ids(Vec<Value>, &Path) -> Vec<Value>`, `_mismatch_categories(&Value) -> Vec<String>`,
//!     `_mismatch_item_count(&Value) -> i64`, `_clear_mismatch_cache()`, `set_mismatch_cache(Option<Value>, Instant)` (값 설정자. 3.8 T6)
//!   - 정적: `_evict_old_cache(&Path)`, `_find_opf_path(&mut ZipArchive<File>) -> Option<String>`, `_validate_preview_epub(&Path) -> (bool, Option<String>)`,
//!     `_get_epub_total_chapters(&Path) -> i64`, `_downscale_epub_image(&[u8]) -> Vec<u8>`, `_book_id_for_path(&Path) -> Option<i64>`,
//!     `_convert_with_libreoffice_with(&dyn CommandRunner, bin: &str, work_dir: Option<&Path>, &Path, fmt: &str) -> String`, `_find_libreoffice_with(which: &dyn Fn(&str) -> Option<String>, exists: &dyn Fn(&Path) -> bool) -> String`,
//!     `_validate_preview_epub_with(&dyn FsOps, &Path) -> (bool, Option<String>)`, `_evict_old_cache_with(&dyn FsOps, &Path)`, `join_created_time_backfill_thread(Duration)`
//! - `backend::book_manager::{BookContent::{File(PathBuf), Text(String)}, BookResponse { status_code: u16, body: Vec<u8> }, RuntimeError(String), ProposeOptions { use_bookstore: Option<bool>, use_content_meta: Option<bool>, on_progress: Option<&dyn Fn(Value)> }: Default}`
//! - `sysdeps::{FsOps, CommandRunner, CommandOutput { returncode: i32, stdout: Vec<u8>, stderr: Vec<u8> }, CommandError::{Timeout, NotFound, Failed(String)}}` (문서 3.1. `FsOps`는 `common::faulty_fs::FaultyFs`, `CommandRunner`는 `common::runner::FakeRunner`가 구현한다)
//! - `backend::pdf::{PdfOpener { fn open(&self, &Path) -> Result<Box<dyn PdfDoc>, PdfError> }, PdfDoc (page_count/pdf_version/docinfo/check_pdf_syntax/close), PdfPageOps (page_count/extract_pages), PdfError { kind, message }}` (문서 3.11)
//! - `backend::classifier::Classify { fn is_available(&self) -> bool; fn classify_path(&self, &Path, Option<f64>) -> Result<Prediction, String>; fn classify_document(&self, &Value, Option<f64>) -> Result<Prediction, String> }`
//! - `backend::classifier::model::Prediction::new(category: Option<&str>, confidence: f64, top: Vec<(String, f64)>, reason: &str) -> Prediction`
//! - `backend::es_manager::{EsManagerApi, EsError, ESManager::connect(env, connector, clock)}` (문서 3.3. 대역은 `common::es_fake::FakeEsManager`)
//! - 파일 읽기(`Loader.read_file`)는 `backend::book_manager::DefaultFileReader`로 대신한다(실제 ES 테스트에서만). `loader` crate에는 의존하지 않는다.
//!
//! TODO(common): 아래 로컬 정의는 common에 같은 것이 생기면 지운다. `FnFileReader`(FileReader 대역), `StaticPdfDoc`/`FakePdfOpener`/`FakePdfPageOps`(문서 3.11의 `fake_pdf.rs`),
//! `BackfillEs`(`FakeEsManager::on_backfill_created_time`).

use crate::{common, elasticsearch_fixture};

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs;
use std::io::{self, Cursor};
use std::ops::{Deref, DerefMut};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use backend::book::{Book, BookPage};
use backend::book_manager::{BookContent, BookManager, BookManagerDeps, BookResponse, FileReader, ProposeOptions, RuntimeError};
use backend::classifier::Classify;
use backend::classifier::model::Prediction;
use backend::es_manager::{ESManager, EsError, EsManagerApi};
use backend::pdf::{PdfDoc, PdfError, PdfOpener, PdfPageOps};
use common::clock::Event;
use common::epub::{EpubSpec, make_epub};
use common::es_fake::{Docs, FakeEsManager};
use common::faulty_fs::{FaultyFs, fail_on_nth_call};
use common::fsx::{mkdir, set_mtime, write_file as write};
use common::runner::FakeRunner;
use common::zipx::{open_zip, write_zip};
use serde_json::{Value, json};
use sysdeps::{CommandError, CommandOutput};
use tempfile::TempDir;

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

const CATEGORY1: &str = "_epub";
const CATEGORY2: &str = "_txt";

const CONTAINER_OEBPS: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;

// ---------------------------------------------------------------------------
// 공용 헬퍼
// ---------------------------------------------------------------------------

/// `asyncio_runner`: 코루틴 하나를 끝까지 돌린다.
fn run<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

fn is_int(v: &Value) -> bool {
    v.is_i64() || v.is_u64()
}

fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

/// `make_manager(tmp_path, es)`의 앞 절반: ES 대역을 쓰는 `BookManagerDeps`를 만든다. 대역(`with_*`)을 꽂을 테스트는 이걸로 시작한다.
fn make_deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    common::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

/// `make_manager(tmp_path, es)`: ES 대역을 쓰는 BookManager를 만든다.
fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(make_deps(tmp_path, es))
}

/// `make_manager(tmp_path, doc)`: dict를 넘기면 `DummyES(doc)`가 된다.
fn make_manager_with_doc(tmp_path: &Path, doc: Value) -> BookManager {
    make_manager(tmp_path, &FakeEsManager::with_doc(doc))
}

/// `EpubSpec::minimal()`에서 챕터를 `ch1.xhtml` 하나로 맞춘 뼈대(구 `_make_minimal_epub`의 기본값). OPF는 루트의 `content.opf`다.
fn minimal_spec() -> EpubSpec {
    let mut spec = EpubSpec::minimal();
    spec.chapters = vec![("ch1.xhtml".to_string(), "<html><body><p>Hello</p></body></html>".to_string())];
    spec
}

// ---------------------------------------------------------------------------
// 로컬 대역 (TODO(common): common에 생기면 지운다)
// ---------------------------------------------------------------------------

type ReadFileFn = Box<dyn Fn(&Path, Option<&Path>) -> Docs + Send + Sync>;

/// `Loader.read_file` 대역. 클로저 하나가 경로와 `path_prefix`를 받아 문서 묶음을 돌려준다.
struct FnFileReader {
    f: ReadFileFn,
}

impl FnFileReader {
    fn new(f: impl Fn(&Path, Option<&Path>) -> Docs + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { f: Box::new(f) })
    }
}

impl FileReader for FnFileReader {
    fn read_file(&self, path: &Path, path_prefix: Option<&Path>) -> Docs {
        (self.f)(path, path_prefix)
    }
}

/// `pikepdf` 대역이 돌려주는 PDF 한 건. 구 `PdfInfo`를 `PdfDoc`으로 흡수했다(3.11 `StaticPdfDoc`).
#[derive(Clone)]
struct StaticPdfDoc {
    docinfo: HashMap<String, String>,
    page_count: usize,
    pdf_version: String,
    syntax_problems: Vec<String>,
}

impl PdfDoc for StaticPdfDoc {
    fn page_count(&self) -> usize {
        self.page_count
    }

    fn pdf_version(&self) -> String {
        self.pdf_version.clone()
    }

    fn docinfo(&self) -> HashMap<String, String> {
        self.docinfo.clone()
    }

    fn check_pdf_syntax(&self) -> Result<Vec<String>, PdfError> {
        Ok(self.syntax_problems.clone())
    }

    fn close(&self) {}
}

/// `pikepdf.open` 대역.
enum FakePdfOpener {
    Ok(StaticPdfDoc),
    OpenError(String),
}

impl PdfOpener for FakePdfOpener {
    fn open(&self, _path: &Path) -> Result<Box<dyn PdfDoc>, PdfError> {
        match self {
            FakePdfOpener::Ok(doc) => Ok(Box::new(doc.clone())),
            FakePdfOpener::OpenError(msg) => Err(PdfError { kind: "OpenError".to_string(), message: msg.clone() }),
        }
    }
}

/// `pypdf`(PdfReader/PdfWriter) 대역.
enum FakePdfPageOps {
    Ok { page_count: usize, output: Vec<u8> },
    ReaderError { kind: String, message: String },
}

impl PdfPageOps for FakePdfPageOps {
    fn page_count(&self, _path: &Path) -> Result<usize, PdfError> {
        match self {
            FakePdfPageOps::Ok { page_count, .. } => Ok(*page_count),
            FakePdfPageOps::ReaderError { kind, message } => Err(PdfError { kind: kind.clone(), message: message.clone() }),
        }
    }

    fn extract_pages(&self, _path: &Path, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
        match self {
            FakePdfPageOps::Ok { output, .. } => Ok(output.clone()),
            FakePdfPageOps::ReaderError { kind, message } => Err(PdfError { kind: kind.clone(), message: message.clone() }),
        }
    }
}

/// `backfill_created_time`만 클로저로 바꿀 수 있는 ES 대역. 나머지는 `FakeEsManager`에 맡긴다.
/// TODO(common): `FakeEsManager::on_backfill_created_time(Fn(&Path) -> Value)`가 생기면 이 구조체를 지운다.
struct BackfillEs {
    inner: Arc<FakeEsManager>,
    on_backfill: Box<dyn Fn(&Path) -> Value + Send + Sync>,
}

impl EsManagerApi for BackfillEs {
    fn create_index(&self) -> Result<Value, EsError> {
        self.inner.create_index()
    }

    fn refresh(&self) -> Result<(), EsError> {
        self.inner.refresh()
    }

    fn backfill_created_time(&self, path_prefix: &Path) -> Result<Value, EsError> {
        Ok((self.on_backfill)(path_prefix))
    }
}

// ---------------------------------------------------------------------------
// 실제 ES / Loader 대응 (TestBookManager 전용)
// ---------------------------------------------------------------------------

/// `Loader.read_file(path)` 대응. 기본 `FileReader`(`DefaultFileReader`)로 문서 하나를 만든다.
/// 운영용 `loader::record::read_file`은 사용자의 미커밋 crate라 쓰지 않는다. 이 테스트는 카테고리·제목·저자·크기·형식만 확인한다.
fn loader_read_file(path: &Path) -> Docs {
    let book_dir = PathBuf::from(std::env::var("TM_BOOK_DIR").unwrap());
    loader_read_file_from(&book_dir, path)
}

fn loader_read_file_from(book_dir: &Path, path: &Path) -> Docs {
    backend::book_manager::DefaultFileReader::new(book_dir.to_path_buf()).read_file(path, None)
}

/// `Loader.read_files(path, num_files=5)` 대응. 디렉토리를 재귀로 훑어 이름순으로 앞의 `num_files`개를 읽는다.
fn loader_read_files(book_dir: &Path, root: &Path, num_files: usize) -> Docs {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = fs::read_dir(dir) else { return };
        let mut paths: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                walk(&path, out);
            } else if path.is_file() {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(root, &mut files);
    let mut data = Docs::new();
    for file in files.into_iter().take(num_files) {
        data.extend(loader_read_file_from(book_dir, &file));
    }
    data
}

/// `tests/books`의 실제 파일을 옮기거나 지우는 테스트(`test_move_book`, `test_delete_book` 등)끼리 병렬로 돌지 않게 하는 잠금.
/// ES 인덱스는 테스트마다 따로 만들지만 디스크 파일은 공유한다. Python은 순차 실행이다.
static BOOKS_LOCK: Mutex<()> = Mutex::new(());

/// `book_manager_with_data()`가 돌려주는 값. 테스트가 끝날 때까지 `BOOKS_LOCK`을 쥔다.
struct LockedBookManager {
    _lock: MutexGuard<'static, ()>,
    bm: BookManager,
}

impl Deref for LockedBookManager {
    type Target = BookManager;
    fn deref(&self) -> &BookManager {
        &self.bm
    }
}

impl DerefMut for LockedBookManager {
    fn deref_mut(&mut self) -> &mut BookManager {
        &mut self.bm
    }
}

/// 모듈 fixture `book_manager_with_data`. Docker Nori Elasticsearch를 사용한다.
fn book_manager_with_data() -> LockedBookManager {
    common::setup_env();
    book_manager_with_data_at(&PathBuf::from(std::env::var("TM_BOOK_DIR").unwrap()))
}

/// 실제 ES 통합 테스트를 격리된 책 디렉터리로 실행한다.
fn book_manager_with_data_at(book_dir: &Path) -> LockedBookManager {
    let lock = BOOKS_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    common::setup_env();
    let es_url = elasticsearch_fixture::elasticsearch_container();
    static NEXT_INDEX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let index = format!("bm1_{}_{}", std::process::id(), NEXT_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let book_index = index.clone();
    let book_dir = book_dir.to_path_buf();
    let bm = BookManager::from_env(
        &move |name: &str| match name {
            "TM_ES_BOOK_INDEX" => Some(book_index.clone()),
            "TM_BOOK_DIR" => Some(book_dir.to_string_lossy().into_owned()),
            _ => std::env::var(name).ok(),
        },
        &move |_default_url: &str| -> Arc<dyn EsManagerApi> {
            let es_index = index.clone();
            let endpoint = es_url.clone();
            let env = move |name: &str| match name {
                "TM_ES_URL" => Some(endpoint.clone()),
                "TM_ES_BOOK_INDEX" => Some(es_index.clone()),
                _ => std::env::var(name).ok(),
            };
            Arc::new(ESManager::connect(&env, &backend::es_manager::RealEsConnector, &sysdeps::SystemClock).expect("실제 ES 접속"))
        },
    )
    .expect("BookManager::from_env");

    // 실제 파일이 있으면 테스트 데이터로 적재한다
    let epub_path = bm.path_prefix().join(CATEGORY1);
    let txt_path = bm.path_prefix().join(CATEGORY2);
    for path in [epub_path, txt_path] {
        if path.exists() {
            let data = loader_read_files(&bm.path_prefix(), &path, 5);
            if !data.is_empty() {
                bm.es_manager().insert(&data).unwrap();
            }
        }
    }

    // 검색되도록 인덱스를 새로 고친다
    bm.es_manager().refresh().unwrap();
    LockedBookManager { _lock: lock, bm }
}

/// 기존 corpus에서 테스트에 필요한 책만 임시 디렉터리로 복사한다.
fn isolated_book_dir() -> TempDir {
    common::setup_env();
    fn copy_some(source: &Path, target: &Path, limit: usize) {
        if !source.is_dir() {
            return;
        }
        fs::create_dir_all(target).unwrap();
        let mut files = Vec::new();
        fn collect(dir: &Path, files: &mut Vec<PathBuf>) {
            let Ok(entries) = fs::read_dir(dir) else { return };
            let mut paths: Vec<_> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
            paths.sort();
            for path in paths {
                if path.is_dir() {
                    collect(&path, files);
                } else if path.is_file() {
                    files.push(path);
                }
            }
        }
        collect(source, &mut files);
        for path in files.into_iter().take(limit) {
            let relative = path.strip_prefix(source).unwrap();
            let destination = target.join(relative);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(path, destination).unwrap();
        }
    }

    let temp = TempDir::new().unwrap();
    let source = PathBuf::from(std::env::var("TM_BOOK_DIR").unwrap());
    for category in [CATEGORY1, CATEGORY2] {
        copy_some(&source.join(category), &temp.path().join(category), 5);
    }
    temp
}

/// Book 객체의 필드 타입을 확인한다. Rust에서는 컴파일 시점에 타입이 고정되므로 대입으로 확인한다.
fn inspect_book_info(book: &Book) {
    let _: i64 = book.book_id;
    let _: &String = &book.category;
    let _: &String = &book.title;
    let _: &String = &book.author;
    let _: &String = &book.file_type;
    let _: &PathBuf = &book.file_path;
    let _: i64 = book.file_size;
    let _: &chrono::NaiveDateTime = &book.updated_time;
}

/// 테스트 데이터에서 책 한 권을 고른다.
fn get_one_random_book(bm: &BookManager) -> Option<Book> {
    for category in [CATEGORY1, CATEGORY2, "_txt", "test"] {
        if let Some(book) = run(bm.get_books_in_category(category)).ok().and_then(|books| books.into_iter().next()) {
            return Some(book);
        }
    }
    None
}

/// 서로 다른 카테고리에서 책 두 권을 고른다.
fn get_two_random_books(bm: &BookManager) -> Option<(Book, Book)> {
    let mut book1 = None;
    let mut book2 = None;

    if let Some(book) = run(bm.get_books_in_category(CATEGORY1)).ok().and_then(|books| books.into_iter().next()) {
        book1 = Some(book);
    }

    if let Some(book) = run(bm.get_books_in_category(CATEGORY2)).ok().and_then(|books| books.into_iter().next()) {
        book2 = Some(book);
    }

    match (book1, book2) {
        (Some(a), Some(b)) => Some((a, b)),
        _ => None,
    }
}
// ---------------------------------------------------------------------------
// class TestBookManager: Docker Nori Elasticsearch 통합 테스트.
// ---------------------------------------------------------------------------

mod test_book_manager {
    use super::*;

    #[test]
    fn test_get_categories() {
        let bm = book_manager_with_data();
        let result = run(bm.get_categories()).unwrap_or_default();
        for (key, value) in result.iter() {
            // 키는 String, 값은 정수여야 한다
            let _: &String = key;
            assert!(is_int(&json!(value)), "카테고리 {key}의 값이 정수가 아니다: {value}");
        }
    }

    #[test]
    fn test_get_books_in_category() {
        let bm = book_manager_with_data();
        for category in [CATEGORY1, CATEGORY2] {
            let book_list = run(bm.get_books_in_category(category)).unwrap();
            if !book_list.is_empty() {
                for book in &book_list {
                    inspect_book_info(book);
                }
                return;
            }
        }
        panic!("No books found in test categories");
    }

    #[test]
    fn test_get_book() {
        let bm = book_manager_with_data();
        let randomly_chosen_book = get_one_random_book(&bm).expect("No books available for testing");

        let book_id = randomly_chosen_book.book_id;
        let book = run(bm.get_book(book_id)).expect("book");
        inspect_book_info(&book);
        assert_eq!(book.book_id, randomly_chosen_book.book_id);
        assert_eq!(book.category, randomly_chosen_book.category);
        assert_eq!(book.title, randomly_chosen_book.title);
    }

    #[test]
    fn test_get_book_content() {
        let bm = book_manager_with_data();
        let book = get_one_random_book(&bm).expect("No books available for testing");
        inspect_book_info(&book);
        let content = run(bm.get_book_content(book.book_id));
        // FileResponse 또는 str
        assert!(matches!(content, BookContent::File(_) | BookContent::Text(_)));
    }

    #[test]
    fn test_search_by_keyword() {
        let bm = book_manager_with_data();
        // 책 하나의 제목을 키워드로 쓴다
        let book = get_one_random_book(&bm).expect("No books available for testing");

        // 제목의 첫 단어를 키워드로 쓴다
        let keyword = if book.title.is_empty() { "테스트" } else { book.title.split_whitespace().next().unwrap_or("테스트") };
        let (book_list, _error) = run(bm.search_by_keyword(keyword, 20));
        // 테스트 데이터에 따라 결과가 비어 있을 수 있다
        let _: Vec<Book> = book_list;
    }

    #[test]
    fn test_search_similar_books() {
        let bm = book_manager_with_data();
        let book = get_one_random_book(&bm).expect("No books available for testing");
        inspect_book_info(&book);
        let (book_list, _error) = run(bm.search_similar_books(book.book_id, 20));
        let _: Vec<Book> = book_list;
    }

    #[test]
    fn test_add_book() {
        let bm = book_manager_with_data();
        let book = get_one_random_book(&bm).expect("No books available for testing");
        inspect_book_info(&book);
        let title = book.title.clone();
        let file_type = book.file_type.clone();
        let file_path = book.file_path.clone();

        assert!(file_path.exists(), "Source file does not exist");

        // 파일을 복사한다
        let new_file_name = format!("{title}.copy.{file_type}");
        let temp_file_path = file_path.with_file_name(new_file_name);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fs::copy(&file_path, &temp_file_path).unwrap();

            // 복사본을 추가한다
            let (book_id2, error) = run(bm.add_book(&loader_read_file(&temp_file_path)));
            let book_id2 = book_id2.expect("book_id2");
            assert!(error.is_none());
            let book2 = run(bm.get_book(book_id2)).expect("book2");
            inspect_book_info(&book2);

            // 복사본을 지운다
            let (result, error) = run(bm.delete_book(book_id2));
            assert!(!result.is_empty() && error.is_none());
        }));
        // finally: 복사본 정리
        if temp_file_path.exists() {
            fs::remove_file(&temp_file_path).unwrap();
        }
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn test_move_book() {
        let isolated = isolated_book_dir();
        let bm = book_manager_with_data_at(isolated.path());
        let (book1, book2) = get_two_random_books(&bm).expect("Need two books from different categories for this test");

        let book_id = book1.book_id;
        let category1 = book1.category.clone();
        let title1 = book1.title.clone();
        let author1 = book1.author.clone();
        let type1 = book1.file_type.clone();
        let path1 = book1.file_path.clone();

        assert!(path1.is_file(), "Source file does not exist");

        let category2 = book2.category.clone();
        let title2 = format!("renamed_{}", book1.title);
        let author2 = if book2.author.is_empty() { book1.author.clone() } else { book2.author.clone() };
        let type2 = book2.file_type.clone();
        let path2 = bm.path_prefix().join(&category2).join(format!("{title2}.{type2}"));

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert!(path1.is_file());
            assert!(!path2.is_file());
            // Python은 (status, msg) 튜플을 assert 하므로 늘 참이다. 결과는 아래 파일 상태와 get_book으로 확인한다.
            let _ = run(bm.update_book(book_id, &category2, &title2, &author2, &path2, &type2));
            assert!(!path1.is_file());
            assert!(path2.is_file());

            let book3 = run(bm.get_book(book_id)).expect("book3");
            inspect_book_info(&book3);
            assert_eq!(book3.category, category2);
            assert_eq!(book3.title, title2);
            assert_eq!(book3.author, author2);
            assert_eq!(book3.file_type, type2);
        }));
        // finally: 원래 자리로 되돌린다
        if path2.is_file() {
            run(bm.update_book(book_id, &category1, &title1, &author1, &path1, &type1));
        }
        if let Err(e) = result {
            std::panic::resume_unwind(e);
        }
    }

    #[test]
    fn test_update_book_rejects_path_traversal() {
        let bm = book_manager_with_data();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path();
        let book = get_one_random_book(&bm).expect("No books found for this test");

        assert!(book.file_path.is_file(), "Source file does not exist");

        // 경로 탈출 시도: path_prefix 외부
        let outside_path = tmp_path.join("outside.txt");
        let (result, error) = run(bm.update_book(book.book_id, &book.category, &book.title, &book.author, &outside_path, &book.file_type));

        assert_eq!(result, "Error");
        assert_eq!(error.as_deref(), Some("잘못된 경로입니다"));
        assert!(book.file_path.is_file());
    }

    #[test]
    fn test_get_category_mismatches() {
        let mut bm = book_manager_with_data();
        let tmp = TempDir::new().unwrap();
        let tmp_path = tmp.path();
        // 프로덕션 디렉토리 전체 스캔 방지: 임시 디렉토리로 교체
        let original_prefix = bm.path_prefix().to_path_buf();
        // 테스트용 디렉토리 구조 생성
        mkdir(tmp_path.join("_epub"));
        mkdir(tmp_path.join("_txt"));
        write(tmp_path.join("_epub/test.epub"), b"test");
        write(tmp_path.join("_txt/test.txt"), "test");

        bm.set_path_prefix(tmp_path);
        let result = bm.get_category_mismatches(false);
        bm.set_path_prefix(&original_prefix);

        // 반환 구조 검증
        assert!(result.is_object());
        assert!(result.get("mismatches").is_some());
        assert!(result.get("es_only").is_some());
        assert!(result.get("fs_only").is_some());
        assert!(result["mismatches"].is_array());
        assert!(result["es_only"].is_array());
        assert!(result["fs_only"].is_array());

        // mismatches 항목 구조 검증
        for item in result["mismatches"].as_array().unwrap() {
            assert!(item.get("category").is_some());
            assert!(item.get("es_count").is_some());
            assert!(item.get("fs_count").is_some());
            assert!(item.get("anomaly_count").is_some());
            assert!(item["category"].is_string());
            assert!(is_int(&item["es_count"]));
            assert!(is_int(&item["fs_count"]));
            // diff(건수 차이)는 0일 수 있다. 경로가 서로 어긋나면 건수가 같아도 이상이다.
            assert!(item["anomaly_count"].as_i64().unwrap() > 0);
        }

        // mismatches가 이상 항목 수 내림차순 정렬인지 검증
        let counts: Vec<i64> = result["mismatches"].as_array().unwrap().iter().map(|i| i["anomaly_count"].as_i64().unwrap()).collect();
        let mut sorted = counts.clone();
        sorted.sort_by(|a, b| b.cmp(a));
        assert_eq!(counts, sorted);

        // es_only 항목 구조 검증
        for item in result["es_only"].as_array().unwrap() {
            assert!(item.get("category").is_some());
            assert!(item.get("es_count").is_some());
            assert!(item["category"].is_string());
            assert!(is_int(&item["es_count"]));
        }

        // fs_only 항목 구조 검증
        for item in result["fs_only"].as_array().unwrap() {
            assert!(item.get("category").is_some());
            assert!(item.get("fs_count").is_some());
            assert!(item["category"].is_string());
            assert!(is_int(&item["fs_count"]));
        }
    }

    /// CATEGORY_SORT를 실제 ES가 거부하면 상세 조회와 카테고리 책 목록이 통째로 빈다.
    ///
    /// search_by_category_paged는 예외를 삼키고 빈 결과를 돌려주므로 장애가 조용하다.
    /// FakeEsManager는 정렬을 흉내만 내서 이 회귀를 단위 테스트로 잡을 수 없다.
    /// _id를 tie-breaker로 넣었다가 fielddata 금지로 전부 0건이 된 적이 있다.
    #[test]
    fn test_search_by_category_paged_sort_works_on_real_es() {
        let bm = book_manager_with_data();
        let categories = run(bm.get_categories()).unwrap();
        assert!(!categories.is_empty(), "테스트 인덱스에 문서가 없다");

        let category = categories.keys().next().unwrap().clone();
        let (page, total, _next_cursor) = bm.es_manager().search_by_category_paged(&category, 5, None).unwrap();
        assert!(total > 0);
        assert!(!page.is_empty());
    }

    #[test]
    fn test_delete_book() {
        let isolated = isolated_book_dir();
        let bm = book_manager_with_data_at(isolated.path());
        let book = get_one_random_book(&bm).expect("No books available for testing");
        inspect_book_info(&book);
        let book_id = book.book_id;
        let title = book.title.clone();
        let file_type = book.file_type.clone();
        let file_path = book.file_path.clone();

        assert!(file_path.exists(), "Source file does not exist");

        // 파일을 복사한다
        let new_file_name = format!("{title}.copy.{file_type}");
        let temp_file_path = file_path.with_file_name(new_file_name);
        fs::copy(&file_path, &temp_file_path).unwrap();

        let (result2, error) = run(bm.delete_book(book_id));
        assert!(!result2.is_empty() && error.is_none());

        assert!(run(bm.get_book(book_id)).is_err());

        // 지운 파일을 되돌린다
        fs::rename(&temp_file_path, &file_path).unwrap();
        let reader = backend::book_manager::DefaultFileReader::new(bm.path_prefix().to_path_buf());
        let (book_id3, error) = run(bm.add_book(&reader.read_file(&file_path, None)));
        assert!(book_id3.is_some() && error.is_none());
    }
}

// ---- merged from test_book_manager_extra.py ----

#[test]
fn test_evict_old_cache() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let old_file = tmp_path.join("old.txt");
    let new_file = tmp_path.join("new.txt");
    write(&old_file, "x");
    write(&new_file, "y");
    let past = SystemTime::now() - Duration::from_secs_f64(BookManager::CACHE_MAX_AGE_SECONDS as f64 + 10.0);
    set_mtime(&old_file, past);
    BookManager::_evict_old_cache(tmp_path);
    assert!(!old_file.exists());
    assert!(new_file.exists());
}

#[test]
fn test_find_opf_path_variants() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let zip_path = tmp_path.join("book.epub");
    write_zip(
        &zip_path,
        &[
            (
                "META-INF/container.xml",
                br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#,
            ),
            ("OEBPS/content.opf", b"<package></package>"),
        ],
    );
    assert_eq!(BookManager::_find_opf_path(&mut open_zip(&zip_path)).as_deref(), Some("OEBPS/content.opf"));

    let zip_path2 = tmp_path.join("book2.epub");
    write_zip(&zip_path2, &[("META-INF/container.xml", b"full-path='OPS/test.opf'"), ("OPS/test.opf", b"<package></package>")]);
    assert_eq!(BookManager::_find_opf_path(&mut open_zip(&zip_path2)).as_deref(), Some("OPS/test.opf"));

    let zip_path3 = tmp_path.join("book3.epub");
    write_zip(&zip_path3, &[("content.opf", b"<package></package>")]);
    assert_eq!(BookManager::_find_opf_path(&mut open_zip(&zip_path3)).as_deref(), Some("content.opf"));
}

#[test]
fn test_update_book_conflict_and_success_and_rollback() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);

    let original = tmp_path.join("A/old.txt");
    mkdir(original.parent().unwrap());
    write(&original, "x");
    let doc = make_doc("A/old.txt", ".txt");
    es.on_search_by_id({
        let doc = doc.clone();
        move |_| Ok(Some(doc.clone()))
    });

    let conflict = tmp_path.join("A/new.txt");
    write(&conflict, "y");
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &conflict, ".txt"));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("CONFLICT"));

    fs::remove_file(&conflict).unwrap();
    es.with(|s| s.update_result = true);
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &conflict, ".txt"));
    assert_eq!(status, "Ok");
    assert!(msg.is_none());
    assert!(conflict.exists());
    assert!(!original.exists());

    // ES 재색인 실패 시 롤백
    let doc2 = make_doc("A/old2.txt", ".txt");
    let old2 = tmp_path.join("A/old2.txt");
    write(&old2, "z");
    es.on_search_by_id(move |_| Ok(Some(doc2.clone())));

    es.on_insert(|data: &Docs| {
        let doc_to_insert = data.values().next().unwrap();
        if doc_to_insert["file_path"] == "A/new2.txt" {
            return Ok(vec![]);
        }
        Ok(data.keys().copied().collect())
    });
    let new2 = tmp_path.join("A/new2.txt");
    let (status, _msg) = run(manager.update_book(2, "A", "T", "U", &new2, ".txt"));
    assert_eq!(status, "Error");
    assert!(old2.exists());
}

#[test]
fn test_update_book_rejects_malformed_es_book_before_file_move() {
    let tmp = TempDir::new().unwrap();
    let mut doc = make_doc("A/old.txt", ".txt");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager_with_doc(tmp.path(), doc);
    let target = tmp.path().join("A/new.txt");

    let (status, error) = run(manager.update_book(1, "A", "T", "U", &target, ".txt"));

    assert_eq!(status, "Error");
    assert!(error.unwrap().contains("category"));
    assert!(!target.exists());
}

/// 이동 후 기존 ES 문서를 삭제하고 새 위치 문서를 다시 insert한다.
#[test]
fn test_update_book_reindexes_moved_file_by_delete_and_insert() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);

    let src = tmp_path.join("A/src.txt");
    mkdir(src.parent().unwrap());
    write(&src, "content");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/src.txt", ".txt"))));

    es.on_update(|_, _| panic!("update_book must replace the ES entry, not update it in place"));

    let dst = tmp_path.join("B/dst.txt");
    let (status, msg) = run(manager.update_book(1, "B", "Renamed", "Author", &dst, "txt"));

    assert_eq!(status, "Ok");
    assert!(msg.is_none());
    es.with(|s| {
        assert_eq!(s.deleted, vec![1]);
        assert_eq!(s.deleted_file_paths, vec![(vec!["B/dst.txt".to_string()], vec![1])]);
        assert_eq!(s.inserted.len(), 1);
        assert_eq!(s.inserted[0][&1]["category"], "B");
        assert_eq!(s.inserted[0][&1]["title"], "Renamed");
        assert_eq!(s.inserted[0][&1]["author"], "Author");
        assert_eq!(s.inserted[0][&1]["file_path"], "B/dst.txt");
    });
    assert!(!src.exists());
    assert!(dst.exists());
}

/// ES delete returns False -> file is rolled back and a meaningful error is returned.
#[test]
fn test_update_book_delete_false_rollback_succeeds() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);

    let src = tmp_path.join("A/src.txt");
    mkdir(src.parent().unwrap());
    write(&src, "content");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/src.txt", ".txt"))));
    es.with(|s| s.delete_result = false);

    let dst = tmp_path.join("A/dst.txt");
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &dst, ".txt"));

    assert_eq!(status, "Error");
    let msg = msg.expect("msg");
    assert!(msg.contains("ES 문서 삭제 실패"), "{msg}");
    assert!(src.exists(), "rollback should have restored the source file");
    assert!(!dst.exists(), "destination should not exist after rollback");
}

/// ES delete returns False and rollback rename also fails -> combined error message.
#[test]
fn test_update_book_delete_false_rollback_fails() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);

    let src = tmp_path.join("A/src2.txt");
    mkdir(src.parent().unwrap());
    write(&src, "content");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/src2.txt", ".txt"))));
    es.with(|s| s.delete_result = false);

    let dst = tmp_path.join("A/dst2.txt");

    // dst를 출발지로 하는 rename(롤백)만 실패시킨다
    let fs_ops = FaultyFs::new();
    {
        let dst = dst.clone();
        fs_ops.set_rename(move |from, _to| if from == dst { Some(io::Error::other("simulated rollback failure")) } else { None });
    }
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &dst, ".txt"));

    assert_eq!(status, "Error");
    let msg = msg.expect("msg");
    assert!(msg.contains("롤백"), "{msg}");
}

/// ES delete raises exception -> file is rolled back and a meaningful error is returned.
#[test]
fn test_update_book_delete_exception_rollback_succeeds() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);

    let src = tmp_path.join("A/src3.txt");
    mkdir(src.parent().unwrap());
    write(&src, "content");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/src3.txt", ".txt"))));

    es.on_delete(|_| Err(EsError::Other("ES connection error".to_string())));

    let dst = tmp_path.join("A/dst3.txt");
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &dst, ".txt"));

    assert_eq!(status, "Error");
    let msg = msg.expect("msg");
    assert!(msg.contains("ES 문서 삭제 예외"), "{msg}");
    assert!(src.exists(), "rollback should have restored the source file");
    assert!(!dst.exists());
}

#[test]
fn test_update_book_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", Path::new("/tmp/out.txt"), ".txt"));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("잘못된 경로"));
}

#[test]
fn test_delete_book_warning_on_missing_file() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(Some(make_doc("A/missing.txt", ".txt"))));
    let (status, msg) = run(manager.delete_book(1));
    assert_eq!(status, "Warning");
    assert!(msg.as_deref().unwrap().contains("이미 삭제"));
}

#[test]
fn test_delete_book_rejects_malformed_es_book_before_file_delete() {
    let tmp = TempDir::new().unwrap();
    let mut doc = make_doc("A/book.txt", ".txt");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager_with_doc(tmp.path(), doc);

    let (status, error) = run(manager.delete_book(1));

    assert_eq!(status, "Error");
    assert!(error.unwrap().contains("category"));
}

#[test]
fn test_delete_file_and_index_single_file() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);

    let (status, _msg) = run(manager.delete_file("../bad.txt"));
    assert_eq!(status, "Error");

    let (status, _msg) = run(manager.delete_file("A/none.txt"));
    assert_eq!(status, "Error");

    let ok = tmp_path.join("A/ok.txt");
    mkdir(ok.parent().unwrap());
    write(&ok, "x");
    let (status, _msg) = run(manager.delete_file("A/ok.txt"));
    assert_eq!(status, "Ok");
    assert!(!ok.exists());

    // Python은 여기서 Loader.read_file을 빈 dict로 바꿔치기하지만, 경로 검증이 먼저 실패하므로
    // 결과에 영향이 없다. 그래서 seam 없이 쓴다.
    let (status_id, msg) = run(manager.index_single_file("../bad.txt"));
    assert!(status_id.is_none());
    assert!(msg.as_deref().unwrap().contains("잘못된 경로"));
}

#[test]
fn test_category_mismatches_cache_and_details() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);

    // FS 구조
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/f1.txt"), "x");
    mkdir(tmp_path.join("A/sub"));
    write(tmp_path.join("A/sub/f2.txt"), "y");

    es.set_aggregate(&[("A", 2), ("B", 1)]);
    let result = manager.get_category_mismatches(false);
    assert!(result.get("mismatches").is_some());

    manager.set_mismatch_cache(Some(json!({"cached": true})), Instant::now());
    es.set_aggregate(&[("A", 0)]);
    assert_eq!(manager.get_category_mismatches(false), json!({"cached": true}));

    // 중복 문서가 있는 mismatch 상세
    let inode = tmp_path.join("A/dup.txt");
    write(&inode, "z");
    let inode_id = fs::metadata(&inode).unwrap().ino() as i64;
    es.with(|s| s.category_docs = vec![(inode_id, make_doc("A/dup.txt", ".txt"), 1.0), (999999, make_doc("A/dup.txt", ".txt"), 1.0)]);
    let details = manager.get_category_mismatch_details("A");
    assert!(!details["duplicates"].as_array().unwrap().is_empty());
    assert!(details["fs_count"].as_i64().unwrap() >= 1);
}

#[test]
fn test_category_mismatch_details_root() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let root_file = tmp_path.join("root.txt");
    write(&root_file, "x");
    let ino = fs::metadata(&root_file).unwrap().ino() as i64;
    es.with(|s| s.category_docs = vec![(ino, make_doc("root.txt", ".txt"), 1.0)]);
    let details = manager.get_category_mismatch_details("_root");
    assert!(details["fs_count"].as_i64().unwrap() >= 1);
}

#[test]
fn test_category_mismatches_ignore_non_indexable_files() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.with(|s| s.aggregate.clear());
    write(tmp_path.join("AGENTS.md"), "rules");
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/count.sh"), "#!/bin/sh\n");

    let result = manager.get_category_mismatches(false);
    assert_eq!(result, json!({"mismatches": [], "es_only": [], "fs_only": []}));
    assert_eq!(manager.get_category_mismatch_details("_root")["fs_count"], 0);
    assert_eq!(manager.get_category_mismatch_details("A")["fs_only"], json!([]));
}

#[test]
fn test_category_mismatches_include_detected_indexable_files() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.with(|s| s.aggregate.clear());
    mkdir(tmp_path.join("A"));
    let epub_with_wrong_extension = tmp_path.join("A/book.bak");
    write_zip(&epub_with_wrong_extension, &[("META-INF/container.xml", b"<container/>")]);

    let result = manager.get_category_mismatches(false);
    let fs_only: Vec<(String, i64, i64)> = result["fs_only"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| (item["category"].as_str().unwrap().to_string(), item["fs_count"].as_i64().unwrap(), item["anomaly_count"].as_i64().unwrap()))
        .collect();
    assert_eq!(fs_only, vec![("A".to_string(), 1, 1)]);
    let details = manager.get_category_mismatch_details("A");
    assert_eq!(details["fs_count"], 1);
    assert_eq!(details["fs_only"], json!([{"file_name": "book.bak", "file_path": "A/book.bak"}]));
}

/// 고아 문서 1건과 미색인 파일 1건은 건수가 같아도 서로 상쇄되면 안 된다.
#[test]
fn test_category_mismatches_do_not_cancel_out() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/present.txt"), "x");
    es.with(|s| s.all_file_paths = vec![("A".into(), "A/gone.txt".into())]);

    let item = manager.get_category_mismatches(false)["mismatches"][0].clone();
    assert_eq!(item["category"], "A");
    assert_eq!(item["diff"], 0);
    assert_eq!((item["es_only_count"].as_i64().unwrap(), item["fs_only_count"].as_i64().unwrap()), (1, 1));
    assert_eq!(item["anomaly_count"], 2);
}

/// 2레벨까지만 보던 이전 구현이 통째로 빠뜨리던 3레벨 이하 카테고리도 검사한다.
#[test]
fn test_category_mismatches_scan_deep_categories() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let deep = tmp_path.join("A/B/C");
    mkdir(&deep);
    write(deep.join("book.txt"), "x");
    es.with(|s| s.all_file_paths = vec![]);

    let result = manager.get_category_mismatches(false);
    let fs_only: Vec<(String, i64)> = result["fs_only"].as_array().unwrap().iter().map(|item| (item["category"].as_str().unwrap().to_string(), item["anomaly_count"].as_i64().unwrap())).collect();
    assert_eq!(fs_only, vec![("A/B/C".to_string(), 1)]);
}

/// FS 스캔이 건너뛰는 폴더는 ES 쪽에서도 빼야 한다.
///
/// 한쪽만 빼면 이미 색인된 문서가 전부 "파일 없는 고아"로 잡힌다. 그 상태에서
/// 재적재를 누르면 디스크에 멀쩡히 있는 파일의 문서를 지운다.
#[test]
fn test_category_mismatches_skip_ignored_dirs() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let intermediate = tmp_path.join("A/page_images");
    mkdir(&intermediate);
    write(intermediate.join("page_0001.png"), "x");
    es.with(|s| s.all_file_paths = vec![("A/page_images".into(), "A/page_images/page_0001.png".into())]);

    assert_eq!(manager._mismatch_categories(&manager.get_category_mismatches(false)), Vec::<String>::new());
}

/// 숨김 디렉토리는 FS 스캔에서 제외하므로 ES 문서만 남아도 이상으로 보고하지 않는다.
#[test]
fn test_category_mismatches_skip_hidden_categories() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let hidden = tmp_path.join(".preview_cache");
    mkdir(&hidden);
    write(hidden.join("cached.txt"), "x");
    es.with(|s| s.all_file_paths = vec![(".preview_cache".into(), ".preview_cache/gone.txt".into())]);

    assert_eq!(manager._mismatch_categories(&manager.get_category_mismatches(false)), Vec::<String>::new());
}

/// 같은 파일을 가리키는 ES 문서가 둘이면 경로 1건을 중복으로 센다.
#[test]
fn test_category_mismatches_count_duplicate_docs() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/book.txt"), "x");
    es.with(|s| s.all_file_paths = vec![("A".into(), "A/book.txt".into()), ("A".into(), "A/book.txt".into())]);

    let item = manager.get_category_mismatches(false)["mismatches"][0].clone();
    assert_eq!((item["es_only_count"].as_i64().unwrap(), item["fs_only_count"].as_i64().unwrap(), item["duplicate_count"].as_i64().unwrap()), (0, 0, 1));
    assert_eq!(item["anomaly_count"], 1);
}

fn _manager_with_one_anomaly(tmp_path: &Path) -> BookManager {
    BookManager::with_deps(_deps_with_one_anomaly(tmp_path))
}

/// `_manager_with_one_anomaly`의 Deps 판. 스캔 관찰자처럼 생성 때 꽂을 대역이 있는 테스트가 쓴다.
fn _deps_with_one_anomaly(tmp_path: &Path) -> BookManagerDeps {
    let deps = make_deps(tmp_path, &FakeEsManager::new());
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/one.txt"), "x");
    deps
}

/// TTL 안에서는 다시 세지 않고, 무효화하면 다시 센다.
#[test]
fn test_mismatch_cache_reuses_result_until_invalidated() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let manager = _manager_with_one_anomaly(tmp_path);
    assert_eq!(manager._mismatch_item_count(&manager.get_category_mismatches(false)), 1);

    write(tmp_path.join("A/two.txt"), "y");
    assert_eq!(manager._mismatch_item_count(&manager.get_category_mismatches(false)), 1);

    manager._clear_mismatch_cache();
    assert_eq!(manager._mismatch_item_count(&manager.get_category_mismatches(false)), 2);
}

/// 일괄 재적재의 전후 건수는 오래된 값을 쓰면 안 되므로 강제로 다시 센다.
#[test]
fn test_mismatch_cache_force_refresh_rescans() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let manager = _manager_with_one_anomaly(tmp_path);
    manager.get_category_mismatches(false);
    write(tmp_path.join("A/two.txt"), "y");

    assert_eq!(manager._mismatch_item_count(&manager.get_category_mismatches(false)), 1);
    assert_eq!(manager._mismatch_item_count(&manager.get_category_mismatches(true)), 2);
}

/// 동시 조회가 겹쳐도 ES를 두 번 훑지 않는다.
#[test]
fn test_mismatch_cache_runs_one_scan_at_a_time() {
    let tmp = TempDir::new().unwrap();
    let scans = Arc::new(Mutex::new(Vec::<i32>::new()));

    // 실제 스캔 앞에 끼워 넣는 관찰자: 호출 횟수를 세고 0.2초 지연한다
    let observer = {
        let scans = scans.clone();
        Arc::new(move || {
            scans.lock().unwrap().push(1);
            thread::sleep(Duration::from_millis(200));
        })
    };
    let manager = Arc::new(BookManager::with_deps(_deps_with_one_anomaly(tmp.path()).with_scan_observer(observer)));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let m = manager.clone();
            thread::spawn(move || {
                m.get_category_mismatches(false);
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    assert_eq!(*scans.lock().unwrap(), vec![1]);
}
// ---- merged from test_book_manager_epub_pdf_extra.py ----

#[test]
fn test_validate_preview_epub_ok_and_fail() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("ok.epub");
    write_zip(
        &epub,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_OEBPS.as_bytes()),
            ("OEBPS/ch1.xhtml", b"<html/>"),
            (
                "OEBPS/content.opf",
                br#"<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest><item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/></manifest>
          <spine><itemref idref="c1"/></spine>
        </package>"#,
            ),
        ],
    );
    let (ok, err) = BookManager::_validate_preview_epub(&epub);
    assert!(ok);
    assert!(err.is_none());

    let bad = tmp_path.join("bad.epub");
    write_zip(&bad, &[("META-INF/container.xml", b"x")]);
    let (ok, err) = BookManager::_validate_preview_epub(&bad);
    assert!(!ok);
    assert!(err.as_deref().unwrap().contains("mimetype"));
}

#[test]
fn test_get_epub_total_chapters() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("c.epub");
    write_zip(
        &epub,
        &[
            (
                "META-INF/container.xml",
                br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#,
            ),
            (
                "content.opf",
                br#"<package xmlns="http://www.idpf.org/2007/opf">
          <manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/><item id="c2" href="c2.xhtml" media-type="application/xhtml+xml"/></manifest>
          <spine><itemref idref="c1"/><itemref idref="c2"/></spine>
        </package>"#,
            ),
            ("c1.xhtml", b"<html/>"),
            ("c2.xhtml", b"<html/>"),
        ],
    );
    assert_eq!(BookManager::_get_epub_total_chapters(&epub), 2);
}

/// EPUB_IMAGE_MAX_EDGE보다 큰 이미지는 줄어들고 형식은 유지된다.
#[test]
fn test_downscale_epub_image_shrinks_large_jpeg() {
    let img = image::RgbImage::from_pixel(4000, 3000, image::Rgb([200, 30, 30]));
    let mut original = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut original, 95).encode_image(&img).unwrap();

    let resized = BookManager::_downscale_epub_image(&original);

    let format = image::guess_format(&resized).unwrap();
    let out = image::load_from_memory(&resized).unwrap();
    assert_eq!(format, image::ImageFormat::Jpeg);
    assert!(out.width().max(out.height()) <= BookManager::EPUB_IMAGE_MAX_EDGE);
    assert!(resized.len() < original.len());
}

/// 이미 충분히 작은 이미지는 바이트를 그대로 둔다.
#[test]
fn test_downscale_epub_image_keeps_small_image() {
    let mut original = Vec::new();
    image::RgbImage::from_pixel(800, 600, image::Rgb([10, 10, 10])).write_to(&mut Cursor::new(&mut original), image::ImageFormat::Jpeg).unwrap();

    assert_eq!(BookManager::_downscale_epub_image(&original), original);
}

/// 깨진 이미지 바이트는 원본 그대로 돌려준다(뷰어 동작 유지).
#[test]
fn test_downscale_epub_image_invalid_bytes_passthrough() {
    let broken = b"not-an-image".to_vec();
    assert_eq!(BookManager::_downscale_epub_image(&broken), broken);
}

#[test]
fn test_downscale_epub_image_png_stays_png() {
    let mut original = Vec::new();
    image::RgbImage::from_pixel(3000, 2000, image::Rgb([0, 128, 0])).write_to(&mut Cursor::new(&mut original), image::ImageFormat::Png).unwrap();

    let resized = BookManager::_downscale_epub_image(&original);

    let format = image::guess_format(&resized).unwrap();
    let out = image::load_from_memory(&resized).unwrap();
    assert_eq!(format, image::ImageFormat::Png);
    assert!(out.width().max(out.height()) <= BookManager::EPUB_IMAGE_MAX_EDGE);
}

#[test]
fn test_convert_with_libreoffice_success_and_fallback() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let fake_bin = tmp_path.join("libreoffice");
    write(&fake_bin, "");

    // 출력 파일 이름(stem)이 달라서 glob 대체 경로를 타게 한다
    let runner = FakeRunner::new(|argv| {
        let idx = argv.iter().position(|a| a == "--outdir").unwrap();
        let outdir = PathBuf::from(&argv[idx + 1]);
        mkdir(&outdir);
        write(outdir.join("other.txt"), "ok");
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    });
    let content = BookManager::_convert_with_libreoffice_with(&*runner, fake_bin.to_str().unwrap(), None, &tmp_path.join("x.doc"), "txt");
    assert_eq!(content, "ok");

    let runner_empty = FakeRunner::new(|_argv| Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] }));
    let content = BookManager::_convert_with_libreoffice_with(&*runner_empty, fake_bin.to_str().unwrap(), None, &tmp_path.join("x.doc"), "txt");
    assert_eq!(content, "");
}

#[test]
fn test_validate_epub_success_and_errors() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write(tmp_path.join("a.epub"), "x");
    let doc = make_doc("a.epub", "epub");
    let mut deps = make_deps(tmp_path, &FakeEsManager::with_doc(doc));

    // 지정된 경로(argv[3])에 JSON을 쓴다
    deps = deps.with_runner(FakeRunner::new(|argv| {
        let data = json!({"messages": [], "checker": {"nFatal": 0, "nError": 0, "nWarning": 0, "nUsage": 0, "nInfo": 0}});
        write(&argv[3], data.to_string());
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    }));
    let manager = BookManager::with_deps(deps);
    let (result, err) = split(run(manager.validate_epub(1)));
    assert!(err.is_none());
    assert_eq!(result.unwrap()["valid"], true);

    // 잘못된 형식
    let manager = make_manager_with_doc(tmp_path, make_doc("a.epub", "pdf"));
    let (_result, err) = split(run(manager.validate_epub(1)));
    assert!(err.as_deref().is_some_and(|e| e.contains("Not an EPUB")));
}

#[test]
fn test_validate_pdf_success_and_open_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write(tmp_path.join("a.pdf"), b"%PDF");
    let doc = make_doc("a.pdf", "pdf");
    let es = FakeEsManager::with_doc(doc);

    let manager = BookManager::with_deps(make_deps(tmp_path, &es).with_pdf_opener(Arc::new(FakePdfOpener::Ok(StaticPdfDoc {
        docinfo: HashMap::from([("/Title".to_string(), "T".to_string()), ("/Author".to_string(), "A".to_string())]),
        page_count: 2,
        pdf_version: "1.4".to_string(),
        syntax_problems: vec![],
    }))));
    let (result, err) = split(run(manager.validate_pdf(1)));
    assert!(err.is_none());
    assert_eq!(result.unwrap()["valid"], true);

    let manager = BookManager::with_deps(make_deps(tmp_path, &es).with_pdf_opener(Arc::new(FakePdfOpener::OpenError("bad".to_string()))));
    let (_result, err) = split(run(manager.validate_pdf(1)));
    assert!(err.as_deref().is_some_and(|e| e.contains("Failed to open PDF")));
}

#[test]
fn test_validate_pdf_reports_reconstructed_cross_reference() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("damaged.pdf");
    let objects = ["<< /Type /Catalog /Pages 2 0 R >>", "<< /Type /Pages /Kids [3 0 R] /Count 1 >>", "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>"];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    pdf.truncate(pdf.len() - 10);
    write(&path, &pdf);

    let manager = make_manager_with_doc(tmp.path(), make_doc("damaged.pdf", "pdf"));
    let (result, err) = split(run(manager.validate_pdf(1)));

    assert!(err.is_none(), "{err:?}");
    let result = result.unwrap();
    assert_eq!(result["valid"], false);
    assert!(result["summary"]["warning"].as_u64().unwrap() > 0);
    assert!(!result["messages"].as_array().unwrap().is_empty());
}

#[test]
fn test_validate_epub_not_found_and_missing_file() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(None));
    let (result, err) = split(run(manager.validate_epub(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("Book not found"));

    let doc = make_doc("missing.epub", "epub");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));
    let (result, err) = split(run(manager.validate_epub(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("File not found"));
}

#[test]
fn test_validate_epub_not_installed() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write(tmp_path.join("a.epub"), "x");
    let doc = make_doc("a.epub", "epub");
    let mut deps = make_deps(tmp_path, &FakeEsManager::with_doc(doc));

    deps = deps.with_runner(FakeRunner::new(|_argv| Err(CommandError::NotFound)));
    let manager = BookManager::with_deps(deps);
    let (result, err) = split(run(manager.validate_epub(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("not installed"));
}

#[test]
fn test_validate_epub_timeout() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write(tmp_path.join("b.epub"), "x");
    let doc = make_doc("b.epub", "epub");
    let mut deps = make_deps(tmp_path, &FakeEsManager::with_doc(doc));

    deps = deps.with_runner(FakeRunner::new(|_argv| Err(CommandError::Timeout)));
    let manager = BookManager::with_deps(deps);
    let (result, err) = split(run(manager.validate_epub(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("timed out"));
}

#[test]
fn test_validate_epub_parse_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write(tmp_path.join("c.epub"), "x");
    let doc = make_doc("c.epub", "epub");
    let mut deps = make_deps(tmp_path, &FakeEsManager::with_doc(doc));

    deps = deps.with_runner(FakeRunner::new(|argv| {
        write(&argv[3], "{bad");
        Ok(CommandOutput { returncode: 1, stdout: vec![], stderr: vec![] })
    }));
    let manager = BookManager::with_deps(deps);
    let (result, err) = split(run(manager.validate_epub(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("Failed to parse epubveri output"));
}

#[test]
fn test_validate_pdf_wrong_type_and_missing_file() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let doc = make_doc("a.txt", "txt");
    let manager = make_manager_with_doc(tmp_path, doc);
    let (result, err) = split(run(manager.validate_pdf(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("Not a PDF"));

    let doc2 = make_doc("missing.pdf", "pdf");
    let manager2 = make_manager_with_doc(tmp_path, doc2);
    let (result, err) = split(run(manager2.validate_pdf(1)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("File not found"));
}

// ---- merged from test_book_manager_preview_extra.py ----

fn build_epub(epub_path: &Path) {
    write_zip(
        epub_path,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_OEBPS.as_bytes()),
            ("OEBPS/ch1.xhtml", b"<html><body>Hi</body></html>"),
            ("OEBPS/toc.ncx", b"<ncx/>"),
            ("OEBPS/styles.css", b"@font-face{font-family:'X';src:url('fonts/missing.ttf'),url('fonts/f.ttf');}"),
            ("OEBPS/fonts/f.ttf", b"fontdata"),
            (
                "OEBPS/content.opf",
                br#"<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
                <item id="css" href="styles.css" media-type="text/css"/>
                <item id="f1" href="fonts/f.ttf" media-type="font/ttf"/>
              </manifest>
              <spine toc="toc"><itemref idref="c1"/></spine>
            </package>"#,
            ),
        ],
    );
}

#[test]
fn test_get_book_preview_epub_success() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("book.epub");
    build_epub(&epub);
    let doc = make_doc("book.epub", "epub");
    let manager = make_manager_with_doc(tmp_path, doc);
    let resp: BookResponse = run(manager.get_book_preview(1, None, Some(1)));
    assert_eq!(resp.status_code, 200);
}

#[test]
fn test_get_book_preview_epub_missing_opf() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("bad.epub");
    write_zip(&epub, &[("mimetype", b"application/epub+zip")]);
    let doc = make_doc("bad.epub", "epub");
    let manager = make_manager_with_doc(tmp_path, doc);
    let resp = run(manager.get_book_preview(1, None, Some(1)));
    assert_eq!(resp.status_code, 422);
}

#[test]
fn test_get_book_preview_doc_and_unsupported() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write(tmp_path.join("a.doc"), "x");
    let doc = make_doc("a.doc", "doc");
    // 변환기가 고정 HTML을 돌려주는 대역(3.6 `office_output`)
    let manager = BookManager::with_deps(make_deps(tmp_path, &FakeEsManager::with_doc(doc)).with_runner(FakeRunner::office_output("<p>ok</p>")));
    let resp = run(manager.get_book_preview(1, None, None));
    assert_eq!(resp.status_code, 200);

    write(tmp_path.join("a.bin"), "x");
    let doc2 = make_doc("a.bin", "bin");
    let manager2 = make_manager_with_doc(tmp_path, doc2);
    let resp = run(manager2.get_book_preview(1, None, None));
    assert_eq!(resp.status_code, 400);
}

// ---- merged from test_book_manager_more.py ----

#[test]
fn test_search_by_keyword_and_similar() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.with(|s| s.keyword = vec![(1, make_doc("a.txt", ".txt"), 1.0)]);
    let (books, err) = run(manager.search_by_keyword("k", -1));
    assert!(!books.is_empty() && err.is_none());

    es.with(|s| s.keyword = vec![]);
    let (books, err) = run(manager.search_by_keyword("k", -1));
    assert!(books.is_empty() && err.is_some());

    es.with(|s| s.similar = vec![(2, make_doc("b.txt", ".txt"), 1.0)]);
    let es_doc = make_doc("a.txt", ".txt");
    es.on_search_by_id(move |_| Ok(Some(es_doc.clone())));
    let (books, err) = run(manager.search_similar_books(1, -1));
    assert!(!books.is_empty() && err.is_none());

    es.with(|s| s.similar = vec![]);
    let (books, err) = run(manager.search_similar_books(1, -1));
    assert!(books.is_empty() && err.is_some());
}

#[test]
fn test_search_similar_books_paged_and_add_book() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.with(|s| s.similar_paged = (vec![(2, make_doc("b.txt", ".txt"), 10.0)], 1));
    es.on_search_by_id(|_| Ok(Some(make_doc("a.txt", ".txt"))));
    assert_eq!(run(manager.search_similar_books_paged(1, 10, 0)).unwrap().total, 1);

    es.with(|s| s.similar_paged = (vec![], 0));
    assert!(run(manager.search_similar_books_paged(1, 10, 0)).is_err());

    let (result, err) = run(manager.add_book(&Docs::from([(1, make_doc("a.txt", ".txt"))])));
    assert!(result == Some(1) && err.is_none());
}

#[test]
fn test_search_by_keyword_paged() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.with(|s| s.keyword_paged = (vec![(1, make_doc("a.txt", ".txt"), 1.0)], 1));
    let BookPage { books, total, .. } = run(manager.search_by_keyword_paged("k", 10, 0, None, None)).unwrap();
    assert_eq!(total, 1);
    assert!(!books.is_empty());

    es.with(|s| s.keyword_paged = (vec![], 0));
    let BookPage { books, total, .. } = run(manager.search_by_keyword_paged("k", 10, 0, None, None)).unwrap();
    assert!(books.is_empty());
    assert_eq!(total, 0);
}

#[test]
fn test_search_by_keyword_paged_forwards_category() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);

    let _ = run(manager.search_by_keyword_paged("k", 20, 10, None, Some("과학")));

    // 호출 인자는 FakeEsManager가 `{keyword, size, offset, exclude_categories, category}` JSON으로 기록한다
    assert_eq!(es.calls_of("search_by_keyword_paged").last().unwrap(), &json!({"keyword": "k", "size": 20, "offset": 10, "exclude_categories": null, "category": "과학"}));
}

#[test]
fn test_delete_book_when_missing_doc() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(None));
    let (status, msg) = run(manager.delete_book(1));
    assert_eq!(status, "Ok");
    assert!(msg.is_none());
}

#[test]
fn test_rename_delete_category_errors() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);

    let (_result, err) = run(manager.rename_category("", "B"));
    assert!(err.is_some());

    let (_result, err) = run(manager.rename_category("A", "A"));
    assert!(err.is_some());

    let (_result, err) = run(manager.rename_category("A", "../B"));
    assert!(err.is_some());

    let (_result, err) = run(manager.delete_category("", false));
    assert!(err.is_some());

    let (_result, err) = run(manager.delete_category("../A", false));
    assert!(err.is_some());

    es.with(|s| {
        s.counts.insert("A".into(), 0);
    });
    let (_result, err) = run(manager.delete_category("A", false));
    assert!(err.is_some());

    es.with(|s| {
        s.counts.insert("A".into(), 1);
        s.deleted_by_category = json!({"deleted": 0, "failures": ["x"]});
    });
    let (_result, err) = run(manager.delete_category("A", false));
    assert!(err.is_some());
}

fn _make_epub(tmp_path: &Path, files: &[(&str, &[u8])]) -> PathBuf {
    let epub_path = tmp_path.join("case.epub");
    write_zip(&epub_path, files);
    epub_path
}

#[test]
fn test_validate_preview_epub_missing_opf_and_manifest() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let container = br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OPS/content.opf"/></rootfiles></container>"#;
    let missing_opf = _make_epub(tmp_path, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container)]);
    let (ok, err) = BookManager::_validate_preview_epub(&missing_opf);
    assert!(!ok);
    assert!(err.as_deref().unwrap().contains("OPF file missing in archive"));

    let opf_no_manifest = br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><spine><itemref idref="c1"/></spine></package>"#;
    let no_manifest = _make_epub(tmp_path, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container), ("OPS/content.opf", opf_no_manifest)]);
    let (ok, err) = BookManager::_validate_preview_epub(&no_manifest);
    assert!(!ok);
    assert_eq!(err.as_deref(), Some("manifest element missing"));

    let opf_no_spine = br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf"><manifest/></package>"#;
    let no_spine = _make_epub(tmp_path, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container), ("OPS/content.opf", opf_no_spine)]);
    let (ok, err) = BookManager::_validate_preview_epub(&no_spine);
    assert!(!ok);
    assert_eq!(err.as_deref(), Some("spine element missing"));
}

#[test]
fn test_validate_preview_epub_bad_zip() {
    let tmp = TempDir::new().unwrap();
    let bad = tmp.path().join("bad.epub");
    write(&bad, "not a zip");
    let (ok, err) = BookManager::_validate_preview_epub(&bad);
    assert!(!ok);
    assert_eq!(err.as_deref(), Some("corrupted ZIP file"));
}

#[test]
fn test_find_opf_path_regex_and_direct() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let bad_container = br#"<container full-path="OPS/content.opf">"#;
    let epub = _make_epub(tmp_path, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", bad_container), ("OPS/content.opf", b"<package/>")]);
    assert_eq!(BookManager::_find_opf_path(&mut open_zip(&epub)).as_deref(), Some("OPS/content.opf"));

    let epub2 = _make_epub(tmp_path, &[("mimetype", b"application/epub+zip"), ("OPS/only.opf", b"<package/>")]);
    assert_eq!(BookManager::_find_opf_path(&mut open_zip(&epub2)).as_deref(), Some("OPS/only.opf"));
}

#[test]
fn test_find_libreoffice_mac_path() {
    let mac_path = "/Applications/LibreOffice.app/Contents/MacOS/soffice";
    let found = BookManager::_find_libreoffice_with(&|_cmd: &str| None, &|p: &Path| p.to_str() == Some(mac_path));
    assert_eq!(found, mac_path);
}

#[test]
fn test_convert_with_libreoffice_direct_output() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let sample = tmp_path.join("sample.docx");
    write(&sample, "doc");

    // TemporaryDirectory 대신 tmp_path를 작업 디렉토리로 쓴다
    let work_dir = tmp_path.to_path_buf();
    let runner = FakeRunner::new(move |_argv| {
        write(work_dir.join("sample.html"), "ok");
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    });
    assert_eq!(BookManager::_convert_with_libreoffice_with(&*runner, "lo", Some(tmp_path), &sample, "html"), "ok");
}

#[test]
fn test_convert_with_libreoffice_no_output() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let sample = tmp_path.join("sample.docx");
    write(&sample, "doc");

    let work_dir = tmp_path.to_path_buf();
    let runner = FakeRunner::new(move |_argv| {
        for entry in fs::read_dir(&work_dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_some_and(|e| e == "html") {
                fs::remove_file(path).unwrap();
            }
        }
        Ok(CommandOutput { returncode: 1, stdout: vec![], stderr: b"err".to_vec() })
    });
    assert_eq!(BookManager::_convert_with_libreoffice_with(&*runner, "lo", Some(tmp_path), &sample, "html"), "");
}

#[test]
fn test_book_manager_init_requires_env() {
    // TM_BOOK_DIR을 지운 환경을 흉내낸다. 프로세스 환경을 바꾸지 않도록 조회 함수를 주입한다.
    let get_env = |_name: &str| -> Option<String> { None };
    let result = BookManager::from_env(&get_env, &|_url: &str| -> Arc<dyn EsManagerApi> { FakeEsManager::new() });
    assert!(matches!(result, Err(RuntimeError(_))));
}

#[test]
fn test_book_manager_from_env_with_es_applies_manager_customization() {
    let tmp = TempDir::new().unwrap();
    let root = tmp.path().to_string_lossy().into_owned();
    let get_env = move |name: &str| -> Option<String> { (name == "TM_BOOK_DIR").then(|| root.clone()) };
    let mut customized = false;
    let manager = BookManager::from_env_with_es(&get_env, FakeEsManager::new(), |deps| {
        customized = true;
        deps.with_bulk_reindex_batch_size(17)
    })
    .unwrap();
    assert_eq!(manager.path_prefix(), tmp.path());
    assert!(customized);
}

#[test]
fn test_book_manager_created_time_backfill_does_not_block_init() {
    let tmp = TempDir::new().unwrap();
    let started = Event::new();
    let release = Event::new();
    let finished = Event::new();

    let es: Arc<dyn EsManagerApi> = {
        let (started, release, finished) = (started.clone(), release.clone(), finished.clone());
        Arc::new(BackfillEs {
            inner: FakeEsManager::new(),
            on_backfill: Box::new(move |_path_prefix: &Path| {
                started.set();
                release.wait(2);
                finished.set();
                json!({"updated": 0, "skipped": 0, "failed": 0})
            }),
        })
    };

    let tm_book_dir = tmp.path().to_string_lossy().into_owned();
    let get_env = move |name: &str| -> Option<String> {
        match name {
            "TM_BOOK_DIR" => Some(tm_book_dir.clone()),
            "TM_BACKFILL_CREATED_TIME_ON_STARTUP" => Some("true".to_string()),
            _ => None,
        }
    };

    let release_timer = {
        let release = release.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(350));
            release.set();
        })
    };
    let start = Instant::now();
    let manager = BookManager::from_env(&get_env, &|_url: &str| es.clone()).unwrap();
    let elapsed = start.elapsed();

    assert!(started.wait(1));
    assert!(elapsed < Duration::from_millis(200), "init이 백필을 기다렸다: {elapsed:?}");
    // finally: 백필 스레드를 정리한다
    release.set();
    release_timer.join().unwrap();
    manager.join_created_time_backfill_thread(Duration::from_secs(2));
    assert!(finished.is_set());
}

#[test]
fn test_get_books_in_category_empty() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    es.with(|s| s.category_docs = vec![]);
    let manager = make_manager(tmp.path(), &es);
    assert!(run(manager.get_books_in_category("missing")).unwrap_err().contains("No books found"));
}

#[test]
fn test_get_book_reports_missing_required_es_fields_like_python_book() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager_with_doc(tmp.path(), json!({"title": "T", "author": "U", "file_path": "A/t.txt", "file_type": ".txt", "file_size": 1, "updated_time": "2024-01-01T00:00:00"}));
    let error = run(manager.get_book(1)).expect_err("누락 필드를 빈 기본값으로 채워 Book을 반환하면 안 된다");
    assert!(error.to_string().contains("category"));
}

#[test]
fn test_catalog_search_paths_report_missing_required_es_fields() {
    let tmp = TempDir::new().unwrap();
    let malformed = json!({"title": "T", "author": "U", "file_path": "A/t.txt", "file_type": ".txt", "file_size": 1, "updated_time": "2024-01-01T00:00:00"});
    let es = FakeEsManager::with_doc(make_doc("A/base.txt", ".txt"));
    es.with(|state| {
        state.category_docs = vec![(2, malformed.clone(), 1.0)];
        state.keyword = state.category_docs.clone();
        state.keyword_paged = (state.category_docs.clone(), 1);
        state.similar = state.category_docs.clone();
        state.similar_paged = (state.category_docs.clone(), 1);
        state.latest_docs = (state.category_docs.clone(), 1);
    });
    let manager = make_manager(tmp.path(), &es);

    assert!(run(manager.get_books_in_category("A")).unwrap_err().contains("category"));
    assert!(run(manager.get_books_in_category_paged("A", 10, "")).unwrap_err().contains("category"));
    let (_, error) = run(manager.search_by_keyword("x", 10));
    assert!(error.unwrap().contains("category"));
    assert!(run(manager.search_by_keyword_paged("x", 10, 0, None, None)).unwrap_err().contains("category"));
    let (_, error) = run(manager.search_similar_books(1, 10));
    assert!(error.unwrap().contains("category"));
    assert!(run(manager.search_similar_books_paged(1, 10, 0)).unwrap_err().contains("category"));
    assert!(run(manager.get_latest_books(10, None)).unwrap_err().contains("category"));
}

// ---- coverage: additional uncovered lines ----

#[test]
fn test_find_libreoffice_which_found() {
    let found = BookManager::_find_libreoffice_with(
        &|cmd: &str| {
            if cmd == "libreoffice" { Some("/usr/bin/libreoffice".to_string()) } else { None }
        },
        &|p: &Path| p.exists(),
    );
    assert_eq!(found, "/usr/bin/libreoffice");
}

#[test]
fn test_find_libreoffice_fallback() {
    let found = BookManager::_find_libreoffice_with(&|_cmd: &str| None, &|_p: &Path| false);
    assert_eq!(found, "libreoffice");
}

#[test]
fn test_get_book_content_missing_file() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(Some(make_doc("A/missing.txt", ".txt"))));
    let result = run(manager.get_book_content(1));
    assert!(matches!(&result, BookContent::Text(s) if s.is_empty()), "{result:?}");
}

#[test]
fn test_get_book_preview_pdf_generate() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/a.pdf"), b"%PDF");
    let doc = make_doc("A/a.pdf", "pdf");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));

    // 3쪽짜리 문서를 읽고, 쓰기는 고정 바이트를 낸다
    deps = deps.with_pdf_pages(Arc::new(FakePdfPageOps::Ok { page_count: 3, output: b"PDFPREVIEW".to_vec() }));
    let manager = BookManager::with_deps(deps);
    let resp = run(manager.get_book_preview(1, Some(2), None));
    assert_eq!(resp.status_code, 200);
}

#[test]
fn test_get_book_preview_epub_chapters_zero() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("book.epub");
    build_epub(&epub);
    let doc = make_doc("book.epub", "epub");
    let manager = make_manager_with_doc(tmp_path, doc);
    // Python은 isinstance(resp, Response)만 본다. 타입이 BookResponse이면 충분하다.
    let _resp: BookResponse = run(manager.get_book_preview(1, None, Some(0)));
}

#[test]
fn test_get_book_preview_epub_old_cache_cleanup() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("book.epub");
    build_epub(&epub);
    let doc = make_doc("book.epub", "epub");
    let manager = make_manager_with_doc(tmp_path, doc);
    let cache_dir = tmp_path.join(".preview_cache");
    mkdir(&cache_dir);
    let old_cache = cache_dir.join("1.epub");
    write(&old_cache, b"old");
    let old_html = cache_dir.join("1.html");
    write(&old_html, "old");
    run(manager.get_book_preview(1, None, Some(1)));
    assert!(!old_cache.exists());
    assert!(!old_html.exists());
}

#[test]
fn test_get_book_preview_epub_img_and_css_and_font() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write_zip(
        &tmp_path.join("book.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_OEBPS.as_bytes()),
            ("OEBPS/ch1.xhtml", br#"<html><body><img src="img/cover.png"/><link href="styles.css"/></body></html>"#),
            ("OEBPS/img/cover.png", b"PNG"),
            ("OEBPS/styles.css", b"@font-face{font-family:'X';src:url('fonts/missing.woff2');}body{color:red;}"),
            ("OEBPS/toc.ncx", br#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint><content src="ch1.xhtml"/></navPoint></ncx>"#),
            (
                "OEBPS/content.opf",
                br#"<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest>
            <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
            <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
            <item id="css" href="styles.css" media-type="text/css"/>
            <item id="img" href="img/cover.png" media-type="image/png"/>
          </manifest>
          <spine toc="toc"><itemref idref="c1"/></spine>
        </package>"#,
            ),
        ],
    );
    let doc = make_doc("book.epub", "epub");
    let manager = make_manager_with_doc(tmp_path, doc);
    let _resp: BookResponse = run(manager.get_book_preview(1, None, Some(1)));
}

#[test]
fn test_get_book_preview_epub_ncx_filter_fail() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    write_zip(
        &tmp_path.join("book.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_OEBPS.as_bytes()),
            ("OEBPS/ch1.xhtml", b"<html><body>Hi</body></html>"),
            ("OEBPS/toc.ncx", b"not valid xml at all <<<"),
            (
                "OEBPS/content.opf",
                br#"<?xml version="1.0"?>
        <package xmlns="http://www.idpf.org/2007/opf">
          <manifest>
            <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
            <item id="toc" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
          </manifest>
          <spine toc="toc"><itemref idref="c1"/></spine>
        </package>"#,
            ),
        ],
    );
    let doc = make_doc("book.epub", "epub");
    let manager = make_manager_with_doc(tmp_path, doc);
    let _resp: BookResponse = run(manager.get_book_preview(1, None, Some(1)));
}

#[test]
fn test_get_book_preview_doc_cache_hit() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    let doc_file = tmp_path.join("A/a.hwp");
    write(&doc_file, "hwp content");
    let doc = make_doc("A/a.hwp", "hwp");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));
    let cache_dir = tmp_path.join(".preview_cache");
    mkdir(&cache_dir);
    let cache_file = cache_dir.join("1.html");
    write(&cache_file, "<p>cached</p>");
    let newer = fs::metadata(&doc_file).unwrap().modified().unwrap() + Duration::from_secs(10);
    set_mtime(&cache_file, newer);
    let resp = run(manager.get_book_preview(1, None, None));
    assert_eq!(resp.status_code, 200);
}
#[test]
fn test_search_similar_books_not_found() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(None));
    let (books, err) = run(manager.search_similar_books(999, -1));
    assert!(books.is_empty());
    assert!(err.as_deref().unwrap().contains("No book found"));
}

#[test]
fn test_search_similar_books_paged_not_found() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(None));
    assert!(run(manager.search_similar_books_paged(999, 10, 0)).unwrap_err().contains("No book found"));
}

#[test]
fn test_add_book_es_failure() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_insert(|_data: &Docs| Ok(vec![]));
    let (result, err) = run(manager.add_book(&Docs::from([(1, make_doc("a.txt", ".txt"))])));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("can't add book"));
}

#[test]
fn test_update_book_samefile_os_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    let original = tmp_path.join("A/old.txt");
    write(&original, "x");
    let doc = make_doc("A/old.txt", ".txt");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));

    let fs_ops = FaultyFs::new();
    fs_ops.set_same_file(|_a, _b| Some(io::Error::other("samefile error")));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &original, ".txt"));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("CONFLICT"));
}

#[test]
fn test_category_mismatch_details_scandir_error() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp.path(), &es);
    es.with(|s| s.category_docs = vec![]);

    let fs_ops = FaultyFs::new();
    fs_ops.set_read_dir(|_path| Some(io::Error::from(io::ErrorKind::PermissionDenied)));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let details = manager.get_category_mismatch_details("A");
    assert_eq!(details["fs_count"], 0);
}

#[test]
fn test_index_single_file_not_found() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    let (result, err) = run(manager.index_single_file("A/nonexistent.txt"));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().contains("파일을 찾을 수 없습니다"));
}

#[test]
fn test_index_single_file_success() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/test.txt"), "hello world");
    let fake_data: Docs = Docs::from([(123, make_doc("A/test.txt", ".txt"))]);
    deps = deps.with_file_reader(FnFileReader::new(move |_p: &Path, _prefix: Option<&Path>| fake_data.clone()));
    let manager = BookManager::with_deps(deps);
    let (result, err) = run(manager.index_single_file("A/test.txt"));
    assert_eq!(result, Some(123));
    assert!(err.is_none());
}

#[test]
fn test_index_single_file_removes_existing_path_docs() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/test.txt"), "hello world");
    let fake_data: Docs = Docs::from([(123, make_doc("A/test.txt", ".txt"))]);
    deps = deps.with_file_reader(FnFileReader::new(move |_p: &Path, _prefix: Option<&Path>| fake_data.clone()));
    let manager = BookManager::with_deps(deps);

    let (result, err) = run(manager.index_single_file("A/test.txt"));

    assert_eq!(result, Some(123));
    assert!(err.is_none());
    es.with(|s| assert_eq!(s.deleted_file_paths, vec![(vec!["A/test.txt".to_string()], vec![123])]));
}

#[test]
fn test_bulk_index_files_merges_multiple_files_into_one_insert_call() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);

    let book_id_by_relpath: Vec<(&str, Option<i64>)> = vec![("A/one.txt", Some(501)), ("A/two.txt", Some(502)), ("A/bad.txt", None)];
    let mut abs_to_relpath: HashMap<PathBuf, &str> = HashMap::new();
    for (rel_path, _) in &book_id_by_relpath {
        let file_path = tmp_path.join(rel_path);
        mkdir(file_path.parent().unwrap());
        write(&file_path, "x");
        abs_to_relpath.insert(file_path.canonicalize().unwrap(), rel_path);
    }

    deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, _prefix: Option<&Path>| {
        let rel_path = abs_to_relpath[abs_path];
        let book_id = book_id_by_relpath.iter().find(|(r, _)| *r == rel_path).unwrap().1;
        match book_id {
            None => Docs::new(), // 파싱 실패 시뮬레이션
            Some(id) => Docs::from([(id, make_doc(rel_path, ".txt"))]),
        }
    }));
    let manager = BookManager::with_deps(deps);

    let (indexed, failures) = run(manager._bulk_index_files(&["A/one.txt", "A/two.txt", "A/bad.txt"], true));

    assert_eq!(indexed, BTreeMap::from([("A/one.txt".to_string(), 501), ("A/two.txt".to_string(), 502)]));
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["file_path"], "A/bad.txt");
    es.with(|s| {
        // 파일마다 insert하지 않고 여러 파일이 insert 1번으로 묶여 나간다
        assert_eq!(s.inserted.len(), 1);
        let mut keys: Vec<i64> = s.inserted[0].keys().copied().collect();
        keys.sort();
        assert_eq!(keys, vec![501, 502]);
        assert_eq!(s.deleted_file_paths, vec![(vec!["A/one.txt".to_string(), "A/two.txt".to_string()], vec![501, 502])]);
    });
}

#[test]
fn test_reload_category_mismatch_details_batches_large_fs_only_list() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    deps = deps.with_bulk_reindex_batch_size(2);

    let rel_paths = ["A/f1.txt", "A/f2.txt", "A/f3.txt"];
    let mut abs_to_relpath: HashMap<PathBuf, (&str, i64)> = HashMap::new();
    for (i, rel_path) in rel_paths.iter().enumerate() {
        let file_path = tmp_path.join(rel_path);
        mkdir(file_path.parent().unwrap());
        write(&file_path, "x");
        abs_to_relpath.insert(file_path.canonicalize().unwrap(), (rel_path, 2000 + i as i64 + 1));
    }

    deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, _prefix: Option<&Path>| {
        let (rel_path, book_id) = abs_to_relpath[abs_path];
        Docs::from([(book_id, make_doc(rel_path, ".txt"))])
    }));
    let manager = BookManager::with_deps(deps);

    let details = json!({"fs_only": rel_paths.iter().map(|p| json!({"file_path": p})).collect::<Vec<_>>(), "es_only": [], "duplicates": []});
    let category_result = run(manager._reload_category_mismatch_details("A", &details));

    assert_eq!(category_result["indexed_count"], 3);
    assert_eq!(category_result["failures"], json!([]));
    // 배치 크기를 2로 바꿨으므로 파일 3개가 2건(2+1)의 insert 호출로 나뉜다
    es.with(|s| assert_eq!(s.inserted.iter().map(|b| b.len()).collect::<Vec<_>>(), vec![2, 1]));
}

#[test]
fn test_reload_category_mismatches_indexes_missing_files_and_deletes_stale_docs() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);

    let book_id_by_relpath: HashMap<&str, i64> = HashMap::from([("A/new.txt", 1001), ("A/dup.txt", 1002), ("C/new.txt", 1003)]);
    let mut abs_to_relpath: HashMap<PathBuf, &str> = HashMap::new();
    for rel_path in book_id_by_relpath.keys() {
        let file_path = tmp_path.join(rel_path);
        mkdir(file_path.parent().unwrap());
        write(&file_path, "x");
        abs_to_relpath.insert(file_path.canonicalize().unwrap(), rel_path);
    }

    let details_by_category: HashMap<&str, Value> = HashMap::from([
        (
            "A",
            json!({
                "fs_only": [{"file_path": "A/new.txt"}],
                "es_only": [{"book_id": 10}],
                "duplicates": [
                    {"file_path": "A/dup.txt", "file_exists": true, "docs": [{"book_id": 20, "file_linked": false}, {"book_id": 21, "file_linked": false}]},
                    {"file_path": "A/linked.txt", "file_exists": true, "docs": [{"book_id": 22, "file_linked": true}, {"book_id": 23, "file_linked": false}]},
                ],
            }),
        ),
        ("B", json!({"fs_only": [], "es_only": [{"book_id": 30}], "duplicates": []})),
        ("C", json!({"fs_only": [{"file_path": "C/new.txt"}], "es_only": [], "duplicates": []})),
    ]);

    deps = deps.with_get_category_mismatches_override(Arc::new(
        |_force_refresh: bool| json!({"mismatches": [{"category": "A", "diff": 3}], "es_only": [{"category": "B", "es_count": 1}], "fs_only": [{"category": "C", "fs_count": 1}]}),
    ));
    deps = deps.with_get_category_mismatch_details_override(Arc::new(move |category: &str| details_by_category[category].clone()));
    deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, _prefix: Option<&Path>| {
        let rel_path = abs_to_relpath[abs_path];
        Docs::from([(book_id_by_relpath[rel_path], make_doc(rel_path, ".txt"))])
    }));
    let manager = BookManager::with_deps(deps);

    let (result, err) = run(manager.reload_category_mismatches(None));

    assert!(err.is_none());
    es.with(|s| {
        // 색인은 파일 1건씩이 아니라 배치(카테고리당 fs_only/duplicates 그룹 단위)로 묶여 나간다.
        let mut inserted_ids: Vec<i64> = s.inserted.iter().flat_map(|b| b.keys().copied()).collect();
        inserted_ids.sort();
        assert_eq!(inserted_ids, vec![1001, 1002, 1003]);
        // fs_only(clean_existing=True)만 delete_by_file_paths로 기존 문서를 정리한다.
        assert_eq!(s.deleted_file_paths, vec![(vec!["A/new.txt".to_string()], vec![1001]), (vec!["C/new.txt".to_string()], vec![1003])]);
        assert_eq!(s.deleted_ids, vec![10, 20, 21, 23, 30]);
    });
    assert_eq!(result["category_count"], 3);
    assert_eq!(result["indexed_count"], 3);
    assert_eq!(result["deleted_count"], 5);
    assert_eq!(result["failed_count"], 0);
}

#[test]
fn test_reload_category_mismatch_files_limits_to_selected_category() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let detail_calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
    let details = Arc::new(Mutex::new(VecDeque::from(vec![
        json!({"fs_only": [{"file_path": "A/new.txt"}], "es_only": [{"book_id": 10}], "duplicates": [{"file_path": "A/dup.txt", "file_exists": true, "docs": [{"book_id": 20, "file_linked": false}, {"book_id": 21, "file_linked": false}]}]}),
        json!({"fs_only": [], "es_only": [], "duplicates": []}),
    ])));

    let book_id_by_relpath: HashMap<&str, i64> = HashMap::from([("A/new.txt", 1001), ("A/dup.txt", 1002)]);
    let mut abs_to_relpath: HashMap<PathBuf, &str> = HashMap::new();
    for rel_path in book_id_by_relpath.keys() {
        let file_path = tmp_path.join(rel_path);
        mkdir(file_path.parent().unwrap());
        write(&file_path, "x");
        abs_to_relpath.insert(file_path.canonicalize().unwrap(), rel_path);
    }

    {
        let detail_calls = detail_calls.clone();
        deps = deps.with_get_category_mismatch_details_override(Arc::new(move |category: &str| {
            detail_calls.lock().unwrap().push(category.to_string());
            details.lock().unwrap().pop_front().unwrap()
        }));
    }
    deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, _prefix: Option<&Path>| {
        let rel_path = abs_to_relpath[abs_path];
        Docs::from([(book_id_by_relpath[rel_path], make_doc(rel_path, ".txt"))])
    }));
    let manager = BookManager::with_deps(deps);

    let (result, err) = run(manager.reload_category_mismatch_files("A", None));

    assert!(err.is_none());
    assert_eq!(*detail_calls.lock().unwrap(), vec!["A".to_string(), "A".to_string()]);
    es.with(|s| {
        // fs_only(clean_existing=True)만 delete_by_file_paths 호출 - duplicates 재색인(clean_existing=False)은 호출 없음
        assert_eq!(s.deleted_file_paths, vec![(vec!["A/new.txt".to_string()], vec![1001])]);
        let mut inserted_ids: Vec<i64> = s.inserted.iter().flat_map(|b| b.keys().copied()).collect();
        inserted_ids.sort();
        assert_eq!(inserted_ids, vec![1001, 1002]);
        assert_eq!(s.deleted_ids, vec![10, 20, 21]);
    });
    assert_eq!(result["category"], "A");
    assert_eq!(result["category_count"], 1);
    assert_eq!(result["before_count"], 3);
    assert_eq!(result["after_count"], 0);
    assert_eq!(result["indexed_count"], 2);
    assert_eq!(result["deleted_count"], 3);
    assert_eq!(result["failed_count"], 0);
}

/// 카테고리 전용 재적재도 진행 중에 indexed_count/deleted_count를 올려야 한다.
///
/// 이전에는 시작 시 before_count 1회만 emit하고, 실제 카운트는 작업이 끝난 뒤에야
/// 기록돼서 화면의 잔여 건수가 작업 내내 최초 대상 건수에 고정됐다.
#[test]
fn test_reload_category_mismatch_files_reports_progress_during_run() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let details = Arc::new(Mutex::new(VecDeque::from(vec![
        json!({"fs_only": [{"file_path": "A/f1.txt"}, {"file_path": "A/f2.txt"}, {"file_path": "A/f3.txt"}], "es_only": [{"book_id": 10}], "duplicates": []}),
        json!({"fs_only": [], "es_only": [], "duplicates": []}),
    ])));

    let mut abs_to_relpath: HashMap<PathBuf, (&str, i64)> = HashMap::new();
    for (i, rel_path) in ["A/f1.txt", "A/f2.txt", "A/f3.txt"].iter().enumerate() {
        let file_path = tmp_path.join(rel_path);
        mkdir(file_path.parent().unwrap());
        write(&file_path, "x");
        abs_to_relpath.insert(file_path.canonicalize().unwrap(), (rel_path, 3000 + i as i64 + 1));
    }

    deps = deps.with_get_category_mismatch_details_override(Arc::new(move |_category: &str| details.lock().unwrap().pop_front().unwrap()));
    deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, _prefix: Option<&Path>| {
        let (rel_path, book_id) = abs_to_relpath[abs_path];
        Docs::from([(book_id, make_doc(rel_path, ".txt"))])
    }));
    let manager = BookManager::with_deps(deps);

    let progress_updates: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![]));
    let (result, err) = {
        let sink = progress_updates.clone();
        run(manager.reload_category_mismatch_files("A", Some(&move |u: Value| sink.lock().unwrap().push(u))))
    };

    assert!(err.is_none());
    assert_eq!(result["indexed_count"], 3);
    assert_eq!(result["deleted_count"], 1);

    // 파일 단위로 indexed_count가 단조 증가하며 보고돼야 한다
    let updates = progress_updates.lock().unwrap();
    let indexed_series: Vec<i64> = updates.iter().filter_map(|u| u.get("indexed_count").and_then(|v| v.as_i64())).collect();
    assert!(!indexed_series.is_empty(), "진행 중 indexed_count가 한 번도 보고되지 않았다");
    let mut sorted = indexed_series.clone();
    sorted.sort();
    assert_eq!(indexed_series, sorted);
    assert_eq!(*indexed_series.iter().max().unwrap(), 3);
    // 파일 3건이므로 중간 보고가 최소 3회는 있어야 한다 (끝에 1회만 몰리면 안 됨)
    assert!(indexed_series.len() >= 3);
    let deleted_series: Vec<i64> = updates.iter().filter_map(|u| u.get("deleted_count").and_then(|v| v.as_i64())).collect();
    assert_eq!(*deleted_series.iter().max().unwrap(), 1);
}

/// 일괄 재적재도 카테고리 경계가 아니라 파일 단위로 누적 진행률을 보고해야 한다.
#[test]
fn test_reload_category_mismatches_reports_progress_within_category() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);

    let rel_paths = ["A/f1.txt", "A/f2.txt", "A/f3.txt", "A/f4.txt"];
    let mut abs_to_relpath: HashMap<PathBuf, (&str, i64)> = HashMap::new();
    for (i, rel_path) in rel_paths.iter().enumerate() {
        let file_path = tmp_path.join(rel_path);
        mkdir(file_path.parent().unwrap());
        write(&file_path, "x");
        abs_to_relpath.insert(file_path.canonicalize().unwrap(), (rel_path, 4000 + i as i64 + 1));
    }

    deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, _prefix: Option<&Path>| {
        let (rel_path, book_id) = abs_to_relpath[abs_path];
        Docs::from([(book_id, make_doc(rel_path, ".txt"))])
    }));
    let n = rel_paths.len();
    deps = deps.with_get_category_mismatches_override(Arc::new(move |_force_refresh: bool| json!({"mismatches": [], "es_only": [], "fs_only": [{"category": "A", "fs_count": n}]})));
    deps = deps.with_get_category_mismatch_details_override(Arc::new(
        move |_category: &str| json!({"fs_only": rel_paths.iter().map(|p| json!({"file_path": p})).collect::<Vec<_>>(), "es_only": [], "duplicates": []}),
    ));
    let manager = BookManager::with_deps(deps);

    let progress_updates: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![]));
    let (result, err) = {
        let sink = progress_updates.clone();
        run(manager.reload_category_mismatches(Some(&move |u: Value| sink.lock().unwrap().push(u))))
    };

    assert!(err.is_none());
    assert_eq!(result["indexed_count"], 4);

    let updates = progress_updates.lock().unwrap();
    let indexed_series: Vec<i64> = updates.iter().filter_map(|u| u.get("indexed_count").and_then(|v| v.as_i64())).collect();
    let mut sorted = indexed_series.clone();
    sorted.sort();
    assert_eq!(indexed_series, sorted);
    assert_eq!(*indexed_series.iter().max().unwrap(), 4);
    // 카테고리가 1개뿐이므로, 카테고리 경계에서만 보고하면 1회밖에 안 나온다
    assert!(indexed_series.len() >= 4);
}

/// propose로 만든 제안을 그대로 apply에 넣으면 실제 auto-classify와 같은 결과(이동+재색인)가 나온다.
///
/// Task 5에서 auto_classify_category가 사라지면서, 예전에 이 한 메서드가 하던
/// "스캔 -> 분류 -> 이동 -> 재색인 -> 진행률 보고"를 propose/apply 두 메서드로 나눴다.
/// 이 테스트는 그 나눈 두 메서드를 이어 붙여도 예전과 같은 결과가 나오는지 본다.
#[test]
fn test_propose_then_apply_moves_file_and_reindexes() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path().to_path_buf();
    let es = FakeEsManager::new();
    let mut deps = make_deps(&tmp_path, &es);
    let source_dir = tmp_path.join("0_inbox");
    mkdir(&source_dir);
    let source_file = source_dir.join("쉬운 과학 이야기.txt");
    write(&source_file, "hello");

    {
        let tmp_path = tmp_path.clone();
        deps = deps.with_file_reader(FnFileReader::new(move |abs_path: &Path, path_prefix: Option<&Path>| {
            assert_eq!(path_prefix, Some(tmp_path.as_path()));
            let rel_path = abs_path.strip_prefix(&tmp_path).unwrap().to_str().unwrap().to_string();
            let mut doc = make_doc(&rel_path, "txt");
            doc["category"] = json!(rel_path.split('/').next().unwrap());
            doc["file_path"] = json!(rel_path);
            Docs::from([(fs::metadata(abs_path).unwrap().ino() as i64, doc)])
        }));
    }
    let manager = BookManager::with_deps(deps);

    let propose_progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![]));
    let mappings = json!({"0_inbox": ["미분류"], "1_fiction": ["소설"], "2_science": ["과학"], "2_science/physics": ["물리"]});
    let (propose_result, propose_err) = {
        let sink = propose_progress.clone();
        let on_progress = move |u: Value| sink.lock().unwrap().push(u);
        run(manager.propose_category_changes(
            "0_inbox",
            Some(&mappings),
            ProposeOptions { use_bookstore: Some(false), use_content_meta: Some(false), on_progress: Some(&on_progress), ..Default::default() },
        ))
    };

    assert!(propose_err.is_none());
    assert_eq!(propose_result["total_count"], 1);
    assert_eq!(propose_progress.lock().unwrap().last().unwrap()["processed_count"], 1);
    assert_eq!(propose_result["items"][0]["file_path"], "0_inbox/쉬운 과학 이야기.txt");
    assert_eq!(propose_result["items"][0]["target_category"], "2_science");
    // 제안 단계는 파일을 옮기지 않는다
    assert!(source_file.exists());

    let items = propose_result["items"].as_array().unwrap().clone();
    let allowed: HashSet<String> = items.iter().map(|item| item["file_path"].as_str().unwrap().to_string()).collect();
    let apply_progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![]));
    let (apply_result, apply_err) = {
        let sink = apply_progress.clone();
        run(manager.apply_category_changes(&items, &allowed, Some(&move |u: Value| sink.lock().unwrap().push(u))))
    };

    let target_file = tmp_path.join("2_science/쉬운 과학 이야기.txt");
    assert!(apply_err.is_none());
    assert!(!source_file.exists());
    assert!(target_file.exists());
    assert_eq!(apply_result["applied_count"], 1);
    assert_eq!(apply_result["failed_count"], 0);
    assert_eq!(apply_progress.lock().unwrap().last().unwrap()["applied_count"], 1);
    es.with(|s| {
        assert_eq!(s.deleted_file_paths[0], (vec!["0_inbox/쉬운 과학 이야기.txt".to_string()], vec![]));
        let indexed_doc = s.inserted[0].values().next().unwrap();
        assert_eq!(indexed_doc["category"], "2_science");
        assert_eq!(indexed_doc["file_path"], "2_science/쉬운 과학 이야기.txt");
    });
}

/// 제안 항목에 book_id(파일 inode)를 싣는다.
///
/// 편집 화면은 /book-edit/{book_id} 로만 열 수 있다. 제안 표에서 원본을 확인하려면
/// 행마다 book_id 가 있어야 한다 - file_path 만으로는 링크를 만들 수 없다.
#[test]
fn test_propose_category_changes_carries_book_id() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let source = tmp_path.join("0_inbox");
    mkdir(&source);
    let book = source.join("어떤 책.txt");
    write(&book, "본문");

    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(vec![]));
    let sink = progress.clone();
    let on_progress = move |u: Value| sink.lock().unwrap().push(u);
    let (result, err) = run(manager.propose_category_changes(
        "0_inbox",
        Some(&json!({})),
        ProposeOptions { use_bookstore: Some(false), use_content_meta: Some(false), on_progress: Some(&on_progress), ..Default::default() },
    ));
    assert!(err.is_none());

    let expected = fs::metadata(&book).unwrap().ino() as i64;
    let ids: Vec<i64> = result["items"].as_array().unwrap().iter().map(|item| item["book_id"].as_i64().unwrap()).collect();
    assert_eq!(ids, vec![expected]);
    // 분류가 끝나기 전에 먼저 깔리는 자리표시자 행에도 실어야, 진행 중에도 링크가 보인다.
    let placeholders: Vec<i64> =
        progress.lock().unwrap().iter().flat_map(|report| report.get("new_items").and_then(|v| v.as_array()).cloned().unwrap_or_default()).map(|item| item["book_id"].as_i64().unwrap()).collect();
    assert_eq!(placeholders, vec![expected]);
}

/// book_id 없이 저장된 옛 제안 항목은 읽을 때 채운다.
///
/// 없으면 표의 편집·삭제 버튼이 안 그려져, 옛 제안을 쓰려면 통째로 다시 만들어야 한다.
#[test]
fn test_fill_missing_book_ids() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    mkdir(tmp_path.join("0_inbox"));
    let book = tmp_path.join("0_inbox/있는 책.txt");
    write(&book, "본문");

    let items =
        vec![json!({"file_path": "0_inbox/있는 책.txt"}), json!({"file_path": "0_inbox/없는 책.txt"}), json!({"file_path": "0_inbox/이미 있는 값.txt", "book_id": 123}), json!({"file_path": ""})];
    let filled = manager.fill_missing_book_ids(items, &manager.path_prefix());

    assert_eq!(filled[0]["book_id"].as_i64().unwrap(), fs::metadata(&book).unwrap().ino() as i64);
    // 파일이 사라졌어도 그 행만 링크가 없을 뿐, 나머지는 그대로 쓴다.
    assert_eq!(filled[1]["book_id"], Value::Null);
    // 이미 값이 있으면 stat 하지 않고 그대로 둔다.
    assert_eq!(filled[2]["book_id"], 123);
    assert!(filled[3].get("book_id").is_none());
}

/// 제안을 만드는 동안 파일이 사라져도 제안 자체는 계속 만든다.
#[test]
fn test_book_id_for_path_returns_none_when_file_is_gone() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(BookManager::_book_id_for_path(&tmp.path().join("없는파일.txt")), None);
}

/// propose_category_changes도 auto_classify_category와 같은 카테고리 이름 검증을 한다.
#[test]
fn test_propose_category_changes_rejects_invalid_category() {
    let tmp = TempDir::new().unwrap();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let (result1, err1) = run(manager.propose_category_changes("", None, ProposeOptions::default()));
    assert_eq!(result1, json!({}));
    assert_eq!(err1.as_deref(), Some("카테고리 이름이 비어있습니다"));

    let (result2, err2) = run(manager.propose_category_changes("non_existent_dir", None, ProposeOptions::default()));
    assert_eq!(result2, json!({}));
    assert!(err2.as_deref().unwrap().contains("디렉토리를 찾을 수 없습니다"));
}

/// 스캔된 경로가 corpus 밖이면(ValueError) 그 항목만 실패로 남기고 계속한다.
#[test]
fn test_propose_category_changes_records_value_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let mut deps = make_deps(tmp_path, &FakeEsManager::new());
    mkdir(tmp_path.join("0_inbox"));
    deps = deps.with_iter_category_indexable_files_override(Arc::new(|_cat: &str, _recursive: bool| vec![PathBuf::from("/outside/book.txt")]));
    let manager = BookManager::with_deps(deps);
    let (result, err) = run(manager.propose_category_changes("0_inbox", None, ProposeOptions::default()));
    assert!(err.is_none());
    assert!(result["failures"].as_array().unwrap().iter().any(|f| f.get("error").and_then(|e| e.as_str()).unwrap_or("").contains("잘못된 파일 경로")));
}

/// 제안 생성이 학습된 모델의 판정을 그대로 제안에 담는지 본다. 파일은 옮기지 않는다.
#[test]
fn test_propose_category_changes_uses_the_supervised_model() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let source_dir = tmp_path.join("0_inbox");
    mkdir(&source_dir);
    let rofan_book = source_dir.join("[로판] 황녀님이 너무해.txt");
    write(&rofan_book, "황녀님과 기사단장 이야기");

    struct StubClassifier;
    impl Classify for StubClassifier {
        fn is_available(&self) -> bool {
            true
        }
        fn classify_path(&self, _fpath: &Path, _min_confidence: Option<f64>) -> Result<Prediction, String> {
            Ok(Prediction::new(Some("3_여성향"), 0.96, vec![("3_여성향".to_string(), 0.96)], "모델 확신도 0.960 -> 3_여성향"))
        }
        fn classify_document(&self, _doc: &Value, _min_confidence: Option<f64>) -> Result<Prediction, String> {
            self.classify_path(Path::new(""), None)
        }
    }

    deps = deps.with_classifier_factory(Arc::new(|| Arc::new(StubClassifier) as Arc<dyn Classify>));
    // 신뢰도 보정 경계는 정책 파일에서 오는데(_is_high_confidence), 테스트 환경엔 정책 파일이
    // 없어 늘 "낮음"으로 본다. 그 경계값 자체는 test_high_confidence_needs_a_calibrated_boundary가
    // 따로 검증하므로, 여기서는 실제 배선(BookClassifierService -> classify_file -> 모델)이
    // 제안까지 이어지는지만 본다.
    deps = deps.with_is_high_confidence_override(Arc::new(|_classifier_service, _confidence: f64| true));

    let fake_doc = json!({"title": "황녀님이 너무해", "author": "작가", "category": "0_inbox", "file_path": rofan_book.strip_prefix(tmp_path).unwrap().to_str().unwrap()});
    deps = deps.with_file_reader(FnFileReader::new(move |_p: &Path, _prefix: Option<&Path>| Docs::from([(12345, fake_doc.clone())])));
    let manager = BookManager::with_deps(deps);

    let (result, err) =
        run(manager.propose_category_changes("0_inbox", Some(&json!({})), ProposeOptions { use_bookstore: Some(false), use_content_meta: Some(true), on_progress: None, ..Default::default() }));

    assert!(err.is_none());
    assert!(rofan_book.exists()); // 제안 단계는 파일을 옮기지 않는다
    assert_eq!(result["items"][0]["target_category"], "3_여성향");
    assert_eq!(result["items"][0]["source"], "model");
    assert_eq!(result["items"][0]["grade"], "certain");
}

#[test]
fn test_rename_category_target_dir_exists() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    mkdir(tmp_path.join("old"));
    mkdir(tmp_path.join("new"));
    let (_result, err) = run(manager.rename_category("old", "new"));
    assert!(err.as_deref().unwrap().contains("이미 존재합니다"));
}

#[test]
fn test_rename_category_no_dir_es_only() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    es.with(|s| s.rename_result = json!({"updated": 3, "failures": []}));
    let (result, err) = run(manager.rename_category("old", "new"));
    assert!(err.is_none());
    assert_eq!(result["fs_renamed"], false);
}

#[test]
fn test_rename_category_es_failure_with_rollback() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    mkdir(tmp_path.join("old"));

    es.fail("rename_category", "ES failure");
    let (_result, err) = run(manager.rename_category("old", "new"));
    assert!(err.as_deref().unwrap().contains("ES 업데이트 실패"));
    assert!(tmp_path.join("old").exists());
}

#[test]
fn test_rename_category_partial_es_failure() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    mkdir(tmp_path.join("old"));
    es.with(|s| s.rename_result = json!({"updated": 2, "failures": ["some error"]}));
    let (_result, err) = run(manager.rename_category("old", "new"));
    assert!(err.as_deref().unwrap().contains("부분 실패"));
    assert!(tmp_path.join("old").exists());
}

#[test]
fn test_delete_book_io_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/ok.txt"), "x");
    let doc = make_doc("A/ok.txt", ".txt");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));

    let fs_ops = FaultyFs::new();
    fs_ops.set_remove_file(|_path| Some(io::Error::other("disk error")));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (status, msg) = run(manager.delete_book(1));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("can't delete a book"));
}

#[test]
fn test_delete_book_es_failure() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/ok.txt"), "x");
    let doc = make_doc("A/ok.txt", ".txt");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));
    es.with(|s| s.delete_result = false);
    let (status, msg) = run(manager.delete_book(1));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("can't delete book information"));
}
// ---- coverage: book_manager additional uncovered lines ----

/// Lines 433-436: get_book_content returns FileResponse when file exists
#[test]
fn test_get_book_content_existing_file() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/ok.txt"), "content");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/ok.txt", ".txt"))));
    let result = run(manager.get_book_content(1));
    assert!(matches!(result, BookContent::File(_)), "{result:?}");
}

/// Lines 458-459: PDF preview cache hit
#[test]
fn test_get_book_preview_pdf_cache_hit() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    let pdf = tmp_path.join("A/a.pdf");
    write(&pdf, b"%PDF");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/a.pdf", "pdf"))));
    let cache_dir = tmp_path.join(".preview_cache");
    mkdir(&cache_dir);
    let cache_file = cache_dir.join("1_n5.pdf"); // 기본 5쪽 미리보기 캐시
    write(&cache_file, b"CACHED");
    set_mtime(&cache_file, fs::metadata(&pdf).unwrap().modified().unwrap() + Duration::from_secs(10));
    let resp = run(manager.get_book_preview(1, None, None));
    assert_eq!(resp.status_code, 200);
}

/// Lines 476-478: PDF preview generation exception
#[test]
fn test_get_book_preview_pdf_exception() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/a.pdf"), b"%PDF");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/a.pdf", "pdf"))));

    deps = deps.with_pdf_pages(Arc::new(FakePdfPageOps::ReaderError { kind: "RuntimeError".to_string(), message: "boom".to_string() }));
    let manager = BookManager::with_deps(deps);
    let resp = run(manager.get_book_preview(1, None, None));
    assert_eq!(resp.status_code, 500);
    assert_eq!(String::from_utf8(resp.body).unwrap(), "PDF preview failed");
}

/// Lines 805-806: OSError during path resolution
#[test]
fn test_update_book_os_error_on_resolve() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let call_count = Arc::new(Mutex::new(0));

    let fs_ops = FaultyFs::new();
    {
        let call_count = call_count.clone();
        fs_ops.set_canonicalize(move |_path| {
            let mut n = call_count.lock().unwrap();
            *n += 1;
            if *n <= 2 { Some(io::Error::other("bad resolve")) } else { None }
        });
    }
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &tmp_path.join("A/f.txt"), ".txt"));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("잘못된 경로"));
}

/// ES reindex exception + rollback failure
#[test]
fn test_update_book_reindex_exception_rollback_failure() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let doc = make_doc("A/old.txt", ".txt");
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));
    let original = tmp_path.join("A/old.txt");
    mkdir(original.parent().unwrap());
    write(&original, "x");
    let new_path = tmp_path.join("A/new.txt");

    es.on_insert(|_data: &Docs| Err(EsError::Other("es fail".to_string())));
    let fs_ops = FaultyFs::new();
    fs_ops.set_rename(fail_on_nth_call(2, "rollback fail"));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (status, msg) = run(manager.update_book(1, "A", "T", "U", &new_path, ".txt"));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("롤백"));
}

/// Lines 875-876: PermissionError in count_files helper
#[test]
fn test_category_mismatch_count_files_permission_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    es.with(|s| s.aggregate.clear());
    mkdir(tmp_path.join("cat"));
    let calls = Arc::new(Mutex::new(0));

    let fs_ops = FaultyFs::new();
    {
        let calls = calls.clone();
        fs_ops.set_read_dir(move |_path| {
            let mut n = calls.lock().unwrap();
            *n += 1;
            // 루트의 count_files
            if *n == 2 { Some(io::Error::from(io::ErrorKind::PermissionDenied)) } else { None }
        });
    }
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let result = manager.get_category_mismatches(false);
    assert!(result.is_object());
}

/// Lines 899-900: PermissionError during L2 scandir
#[test]
fn test_category_mismatch_l2_scandir_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    es.with(|s| s.aggregate.clear());
    let l1 = tmp_path.join("cat1");
    mkdir(&l1);
    write(l1.join("file.txt"), "x");
    let calls = Arc::new(Mutex::new(0));

    let fs_ops = FaultyFs::new();
    {
        let calls = calls.clone();
        fs_ops.set_read_dir(move |_path| {
            let mut n = calls.lock().unwrap();
            *n += 1;
            if *n >= 3 { Some(io::Error::from(io::ErrorKind::PermissionDenied)) } else { None }
        });
    }
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let result = manager.get_category_mismatches(false);
    assert!(result.is_object());
}

/// Lines 910-911: PermissionError scanning base directory
#[test]
fn test_category_mismatch_top_level_error() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp.path(), &es);
    es.with(|s| s.aggregate.clear());

    let fs_ops = FaultyFs::new();
    fs_ops.set_read_dir(|_path| Some(io::Error::from(io::ErrorKind::PermissionDenied)));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let result = manager.get_category_mismatches(false);
    assert_eq!(result["mismatches"], json!([]));
}

/// Lines 970-971: ES-only file detection
#[test]
fn test_category_mismatch_details_es_only() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.with(|s| s.category_docs = vec![(1, make_doc("A/missing.txt", ".txt"), 1.0)]);
    mkdir(tmp.path().join("A"));
    let details = manager.get_category_mismatch_details("A");
    assert!(!details["es_only"].as_array().unwrap().is_empty());
}

#[test]
fn test_category_mismatch_details_normalizes_absolute_es_paths() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let category_dir = tmp_path.join("A");
    mkdir(&category_dir);
    let file_path = category_dir.join("same.txt");
    write(&file_path, "x");

    let ino = fs::metadata(&file_path).unwrap().ino() as i64;
    es.with(|s| s.category_docs = vec![(ino, make_doc(file_path.to_str().unwrap(), ".txt"), 1.0)]);

    let details = manager.get_category_mismatch_details("A");

    assert_eq!(details["es_only"], json!([]));
    assert_eq!(details["fs_only"], json!([]));
}

/// Lines 991-992: OSError during stat for duplicates
#[test]
fn test_category_mismatch_details_stat_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    write(tmp_path.join("dup.txt"), "x");
    es.with(|s| s.category_docs = vec![(1, make_doc("dup.txt", ".txt"), 1.0), (2, make_doc("dup.txt", ".txt"), 1.0)]);

    let fs_ops = FaultyFs::new();
    fs_ops.set_metadata(|_path| Some(io::Error::other("no stat")));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let details = manager.get_category_mismatch_details("_root");
    assert!(!details["duplicates"].as_array().unwrap().is_empty());
}

/// Lines 1024-1025: IOError during file deletion
#[test]
fn test_delete_file_io_error() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/x.txt"), "x");

    let fs_ops = FaultyFs::new();
    fs_ops.set_remove_file(|_path| Some(io::Error::other("boom")));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (status, msg) = run(manager.delete_file("A/x.txt"));
    assert_eq!(status, "Error");
    assert!(msg.as_deref().unwrap().contains("파일 삭제 실패"));
}

/// Lines 1029-1073: reload_category subprocess flow
#[test]
fn test_reload_category_full_flow() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    mkdir(tmp_path.join("A"));
    // 단계마다 다른 외부 명령 대역을 꽂은 매니저를 새로 만든다(생성 뒤에는 대역을 바꾸지 않는다)
    let manager_with = |runner: Arc<FakeRunner>| BookManager::with_deps(make_deps(tmp_path, &es).with_runner(runner));

    // 시간 초과
    let manager = manager_with(FakeRunner::new(|_argv| Err(CommandError::Timeout)));
    let (_result, err) = run(manager.reload_category("A", None));
    assert!(err.as_deref().unwrap().contains("초과"));

    // 실행 오류
    let manager = manager_with(FakeRunner::new(|_argv| Err(CommandError::Failed("boom".to_string()))));
    let (_result, err) = run(manager.reload_category("A", None));
    assert!(err.as_deref().unwrap().contains("재적재 실행 실패"));

    // 0이 아닌 종료 코드
    let manager = manager_with(FakeRunner::new(|_argv| Ok(CommandOutput { returncode: 2, stdout: vec![], stderr: b"error message".to_vec() })));
    let (_result, err) = run(manager.reload_category("A", None));
    assert!(err.as_deref().unwrap().contains("exit 2"));

    // 성공
    let manager = manager_with(FakeRunner::new(|_argv| Ok(CommandOutput { returncode: 0, stdout: "총 5개 파일 처리됨".as_bytes().to_vec(), stderr: b"warn".to_vec() })));
    let (result, err) = run(manager.reload_category("A", Some("comic")));
    assert!(err.is_none());
    assert_eq!(result["processed_count"], 5);
}

/// '_root'는 실제 카테고리가 아니라 최상위 파일 묶음이라 삭제 대상이 될 수 없다.
#[test]
fn test_delete_category_rejects_root() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("_root", 3)]);

    let (result, err) = run(manager.delete_category("_root", false));
    assert_eq!(result, json!({}));
    assert!(err.as_deref().unwrap().contains("최상위 디렉토리"));
    es.with(|s| assert_eq!(s.deleted_by_category["deleted"], 2)); // delete_by_category 미호출
}

/// 카테고리 관리 서브탭은 ES 문서만 지운다. delete_files를 주지 않으면 디스크를 건드리지 않는다.
#[test]
fn test_delete_category_keeps_files_by_default() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("A", 1)]);
    es.with(|s| {
        s.deleted_by_category = json!({"deleted": 1, "failures": []});
    });
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/one.pdf"), b"x");

    let (result, err) = run(manager.delete_category("A", false));

    assert!(err.is_none());
    assert_eq!(result["deleted_count"], 1);
    assert!(result.get("fs_deleted").is_none());
    assert!(tmp_path.join("A/one.pdf").exists());
}

/// UI가 '하위 디렉토리의 모든 파일을 삭제'라고 안내하므로 ES뿐 아니라 디스크도 지워야 한다.
#[test]
fn test_delete_category_removes_directory_tree() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("A", 2)]);
    es.with(|s| {
        s.deleted_by_category = json!({"deleted": 2, "failures": []});
    });
    mkdir(tmp_path.join("A/sub"));
    write(tmp_path.join("A/one.pdf"), b"x");
    write(tmp_path.join("A/sub/two.pdf"), b"x");
    mkdir(tmp_path.join("B"));
    write(tmp_path.join("B/keep.pdf"), b"x");

    let (result, err) = run(manager.delete_category("A", true));

    assert!(err.is_none());
    assert_eq!(result["deleted_count"], 2);
    assert_eq!(result["fs_deleted"], true);
    assert!(!tmp_path.join("A").exists());
    assert!(tmp_path.join("B/keep.pdf").exists());
}

/// 디스크에 디렉토리가 없어도 ES 문서는 지우고 성공으로 돌려준다.
#[test]
fn test_delete_category_es_only_when_directory_missing() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("A", 1)]);
    es.with(|s| {
        s.deleted_by_category = json!({"deleted": 1, "failures": []});
    });

    let (result, err) = run(manager.delete_category("A", true));

    assert!(err.is_none());
    assert_eq!(result["deleted_count"], 1);
    assert_eq!(result["fs_deleted"], false);
}

/// ES에 문서가 없어도(빈 디렉토리, 미적재 파일) 편집 탭에서는 디스크 디렉토리를 지운다.
#[test]
fn test_delete_category_removes_directory_without_es_docs() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("A", 0)]);
    es.with(|s| {
        s.deleted_by_category = json!({"deleted": 0, "failures": []});
    });
    mkdir(tmp_path.join("A/sub"));
    write(tmp_path.join("A/sub/unindexed.jpg"), b"x");

    let (result, err) = run(manager.delete_category("A", true));

    assert!(err.is_none());
    assert_eq!(result["deleted_count"], 0);
    assert_eq!(result["fs_deleted"], true);
    assert!(!tmp_path.join("A").exists());
}

/// ES 문서도 디스크 디렉토리도 없으면 지울 대상이 없다고 알린다.
#[test]
fn test_delete_category_errors_when_nothing_to_delete() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("A", 0)]);

    let (result, err) = run(manager.delete_category("A", true));

    assert_eq!(result, json!({}));
    assert!(err.as_deref().unwrap().contains("문서가 없습니다"));
}

/// 디렉토리 삭제가 실패하면 ES 문서를 남겨 다시 시도할 수 있게 한다.
#[test]
fn test_delete_category_keeps_es_when_fs_delete_fails() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    es.set_counts(&[("A", 1)]);
    es.with(|s| {
        s.deleted_by_category = json!({"deleted": 99, "failures": []});
    });
    mkdir(tmp_path.join("A"));

    let fs_ops = FaultyFs::new();
    fs_ops.set_remove_dir_all(|_path| Some(io::Error::other("denied")));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);

    let (result, err) = run(manager.delete_category("A", true));

    assert_eq!(result, json!({}));
    assert!(err.as_deref().unwrap().contains("디렉토리 삭제 실패"));
    // ES 문서를 지우는 delete_by_category는 한 번도 불리면 안 된다
    assert_eq!(es.call_count("delete_by_category"), 0);
    assert!(tmp_path.join("A").exists());
}

/// '.'는 path_prefix 자신으로 해석되어 라이브러리 전체가 지워진다. 거부해야 한다.
#[test]
fn test_delete_category_rejects_library_root_alias() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[(".", 5)]);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/one.pdf"), b"x");

    let (result, err) = run(manager.delete_category(".", true));

    assert_eq!(result, json!({}));
    assert!(err.is_some());
    assert!(tmp_path.join("A/one.pdf").exists());
}

/// 삭제 확인 모달용 미리보기: 하위 디렉토리 수, 디스크 파일 수, ES 문서 수를 센다.
#[test]
fn test_preview_delete_category_counts_subtree() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("A", 7)]);
    mkdir(tmp_path.join("A/s1/deep"));
    mkdir(tmp_path.join("A/s2"));
    write(tmp_path.join("A/one.pdf"), b"x");
    write(tmp_path.join("A/s1/two.pdf"), b"x");
    write(tmp_path.join("A/s1/deep/three.pdf"), b"x");
    mkdir(tmp_path.join("B"));
    write(tmp_path.join("B/other.pdf"), b"x");

    let (result, err) = run(manager.preview_delete_category("A"));

    assert!(err.is_none());
    assert_eq!(result, json!({"category": "A", "directory_count": 3, "file_count": 3, "es_count": 7}));
    assert!(tmp_path.join("A/one.pdf").exists());
}

/// 디스크에 디렉토리가 없으면 파일·디렉토리는 0으로 돌려준다.
#[test]
fn test_preview_delete_category_without_directory() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("A", 2)]);

    let (result, err) = run(manager.preview_delete_category("A"));

    assert!(err.is_none());
    assert_eq!(result, json!({"category": "A", "directory_count": 0, "file_count": 0, "es_count": 2}));
}

/// 삭제와 같은 이름 검증을 쓴다: 빈 값, '..', '_root', 라이브러리 루트 별칭.
#[test]
fn test_preview_delete_category_rejects_unsafe_names() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    mkdir(tmp.path().join("A"));
    for name in ["", "../A", "_root", "."] {
        let (result, err) = run(manager.preview_delete_category(name));
        assert_eq!(result, json!({}), "{name}");
        assert!(err.is_some(), "{name}");
    }
}

/// '_root'를 이름 변경하면 파일은 최상위에 남고 ES category만 바뀌어 불일치가 생긴다.
#[test]
fn test_rename_category_rejects_root_as_source() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("_root", 3), ("새이름", 0)]);

    let (result, err) = run(manager.rename_category("_root", "새이름"));
    assert_eq!(result, json!({}));
    assert!(err.as_deref().unwrap().contains("최상위 디렉토리"));
}

/// '_root'는 예약된 이름이라 실제 디렉토리 이름으로 쓸 수 없다.
#[test]
fn test_rename_category_rejects_root_as_target() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    es.set_counts(&[("A", 3), ("_root", 0)]);

    let (result, err) = run(manager.rename_category("A", "_root"));
    assert_eq!(result, json!({}));
    assert!(err.as_deref().unwrap().contains("_root"));
    assert!(tmp_path.join("A").is_dir());
}

/// '_root'는 실제 디렉토리가 아니라 최상위 파일 묶음이라, loader 대신 최상위 파일만 재적재한다.
#[test]
fn test_reload_category_root_reloads_top_level_files_only() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    write(tmp_path.join("root.txt"), "x");
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/sub.txt"), "y");

    deps = deps.with_runner(FakeRunner::new(|_argv| panic!("_root는 loader subprocess를 실행하면 안 된다")));
    let manager = BookManager::with_deps(deps);

    let (result, err) = run(manager.reload_category("_root", None));
    assert!(err.is_none());
    assert_eq!(result["category"], "_root");
    assert_eq!(result["processed_count"], 1);
    es.with(|s| {
        let indexed_paths: Vec<String> = s.inserted.iter().flat_map(|batch| batch.values().map(|doc| doc["file_path"].as_str().unwrap().to_string())).collect();
        assert_eq!(indexed_paths, vec!["root.txt".to_string()]);
    });
}

/// Line 1083: get_pdf_pages file not found
#[test]
fn test_get_pdf_pages_file_not_found() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(Some(make_doc("A/missing.pdf", "pdf"))));
    let resp = run(manager.get_pdf_pages(1, 1, 1));
    assert_eq!(resp.status_code, 404);
}

/// Lines 1123-1125: PDF pages extraction exception
#[test]
fn test_get_pdf_pages_exception() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    mkdir(tmp_path.join("A"));
    write(tmp_path.join("A/a.pdf"), b"%PDF");
    es.on_search_by_id(|_| Ok(Some(make_doc("A/a.pdf", "pdf"))));

    deps = deps.with_pdf_pages(Arc::new(FakePdfPageOps::ReaderError { kind: "RuntimeError".to_string(), message: "boom".to_string() }));
    let manager = BookManager::with_deps(deps);
    let resp = run(manager.get_pdf_pages(1, 1, 1));
    assert_eq!(resp.status_code, 500);
    // 원인 파악이 가능하도록 예외 타입과 메시지를 본문에 포함한다
    assert_eq!(String::from_utf8(resp.body).unwrap(), "PDF pages extraction failed (RuntimeError): boom");
}

/// Lines 1147, 1149: rename_category path traversal
#[test]
fn test_rename_category_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("A", 1), ("B", 0)]);
    let evil = tmp_path.join("evil");
    std::os::unix::fs::symlink("/tmp", &evil).unwrap();
    let (_result, err) = run(manager.rename_category("evil", "B"));
    assert!(err.is_some());
}

/// Lines 1173-1174: OSError during directory rename
#[test]
fn test_rename_category_os_error_rename() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    mkdir(tmp_path.join("old"));

    let fs_ops = FaultyFs::new();
    fs_ops.set_rename(|_from, _to| Some(io::Error::other("rename failed")));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (_result, err) = run(manager.rename_category("old", "new"));
    assert!(err.as_deref().unwrap().contains("이름 변경 실패"));
}

/// Lines 1187-1189: ES exception + FS rollback failure
#[test]
fn test_rename_category_es_failure_rollback_fail() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    mkdir(tmp_path.join("old"));

    es.fail("rename_category", "ES fail");
    let fs_ops = FaultyFs::new();
    fs_ops.set_rename(fail_on_nth_call(2, "rollback fail"));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (_result, err) = run(manager.rename_category("old", "new"));
    assert!(err.as_deref().unwrap().contains("수동 복구 필요"));
}

/// Lines 1198-1200: partial ES failure + rollback failure
#[test]
fn test_rename_category_partial_es_rollback_fail() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    es.set_counts(&[("old", 1), ("new", 0)]);
    mkdir(tmp_path.join("old"));
    es.with(|s| s.rename_result = json!({"updated": 2, "failures": ["err"]}));
    let fs_ops = FaultyFs::new();
    fs_ops.set_rename(fail_on_nth_call(2, "rollback fail"));
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);
    let (_result, err) = run(manager.rename_category("old", "new"));
    assert!(err.as_deref().unwrap().contains("수동 복구 필요"));
}

/// Line 1222: delete_category path traversal
#[test]
fn test_delete_category_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let evil = tmp_path.join("evil");
    std::os::unix::fs::symlink("/tmp", &evil).unwrap();
    let (_result, err) = run(manager.delete_category("evil", false));
    assert!(err.is_some());
}

/// Lines 1232-1233: ES delete exception
#[test]
fn test_delete_category_es_exception() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("A", 1)]);

    es.fail("delete_by_category", "boom");
    let (_result, err) = run(manager.delete_category("A", false));
    assert!(err.as_deref().unwrap().contains("ES 삭제 실패"));
}

/// Line 1265: delete_book returns warning when file already deleted
#[test]
fn test_delete_book_warning_message() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.on_search_by_id(|_| Ok(Some(make_doc("A/missing.txt", ".txt"))));
    let (status, msg) = run(manager.delete_book(1));
    assert_eq!(status, "Warning");
    assert!(msg.as_deref().unwrap().contains("이미 삭제"));
}

// ---- coverage: validate_preview_epub edge cases ----

/// Line 67: mimetype content is not application/epub+zip
#[test]
fn test_validate_epub_invalid_mimetype() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    make_epub(&epub, &minimal_spec().without_mimetype().with_extra("mimetype", "application/zip"));
    let (valid, err) = BookManager::_validate_preview_epub(&epub);
    assert!(!valid);
    assert!(err.as_deref().unwrap().contains("invalid mimetype"));
}

/// Line 72: OPF file not found
#[test]
fn test_validate_epub_no_opf() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    make_epub(&epub, &EpubSpec { opf_path: String::new(), ..minimal_spec() }.without_container());
    let (valid, err) = BookManager::_validate_preview_epub(&epub);
    assert!(!valid);
    assert!(err.as_deref().unwrap().contains("OPF file not found"));
}

/// Lines 79-80: OPF parse error (severely malformed XML)
#[test]
fn test_validate_epub_opf_parse_error() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    // lxml은 recover=True로 대부분의 깨진 XML을 처리하지만, 복구가 불가능한 바이너리 쓰레기로 예외 경로를 시험한다.
    make_epub(&epub, &minimal_spec().with_opf("\u{0}\u{1}\u{2}"));
    // 복구 모드가 쓰레기를 파싱해 낼 수도 있다. 중요한 것은 죽지 않고 bool을 돌려주는 것이다.
    let (valid, _err): (bool, Option<String>) = BookManager::_validate_preview_epub(&epub);
    let _ = valid;
}

/// Lines 103-105: spine idref not in manifest -> removal + rewrite
#[test]
fn test_validate_epub_spine_idref_not_in_manifest() {
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="missing_item"/>
  </spine>
</package>"#
    );
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let (valid, err) = BookManager::_validate_preview_epub(&epub);
    assert!(valid);
    assert!(err.is_none());
}

/// Lines 113-121: spine item's file missing from ZIP
#[test]
fn test_validate_epub_spine_file_missing_from_zip() {
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
  </spine>
</package>"#
    );
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    // ch2.xhtml은 manifest/spine에 있지만 zip에는 없다
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let (valid, err) = BookManager::_validate_preview_epub(&epub);
    assert!(valid);
    assert!(err.is_none());
}

/// Line 126: all spine chapters removed -> no valid chapters
#[test]
fn test_validate_epub_no_valid_spine_chapters() {
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="missing.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    // zip 안에 ch1.xhtml도 missing.xhtml도 없다
    make_epub(&epub, &EpubSpec { chapters: vec![], ..minimal_spec() }.with_opf(&opf));
    let (valid, err) = BookManager::_validate_preview_epub(&epub);
    assert!(!valid);
    assert!(err.as_deref().unwrap().contains("no valid spine chapters"));
}

/// Lines 132-134: toc id not in manifest
#[test]
fn test_validate_epub_toc_not_in_manifest() {
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine toc="missing_ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let (valid, _err) = BookManager::_validate_preview_epub(&epub);
    assert!(valid);
}

/// Lines 139-141: toc NCX in manifest but missing from ZIP
#[test]
fn test_validate_epub_toc_ncx_missing_from_zip() {
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    // toc.ncx는 manifest에 있지만 zip에는 없다
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let (valid, _err) = BookManager::_validate_preview_epub(&epub);
    assert!(valid);
}

/// Lines 159-160: corrupted ZIP
#[test]
fn test_validate_epub_corrupted_zip() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    write(&epub, b"not a zip file");
    let (valid, err) = BookManager::_validate_preview_epub(&epub);
    assert!(!valid);
    assert!(err.as_deref().unwrap().contains("corrupted ZIP"));
}

/// Lines 161-162: general exception during validation
#[test]
fn test_validate_epub_general_exception() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    make_epub(&epub, &minimal_spec());

    // ZIP 열기가 PermissionError로 실패하는 파일 시스템
    let fs_ops = FaultyFs::new();
    fs_ops.set_open(|_path| Some(io::Error::from(io::ErrorKind::PermissionDenied)));
    let (valid, err) = BookManager::_validate_preview_epub_with(&*fs_ops, &epub);
    assert!(!valid);
    assert!(err.as_deref().unwrap().contains("validation error"));
}

// ---- coverage: _evict_old_cache exception paths ----

/// Lines 176-177: per-file exception during eviction
#[test]
fn test_evict_old_cache_file_exception() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let old_file = tmp_path.join("old.txt");
    write(&old_file, "x");
    let past = SystemTime::now() - Duration::from_secs_f64(BookManager::CACHE_MAX_AGE_SECONDS as f64 + 10.0);
    set_mtime(&old_file, past);

    let fs_ops = FaultyFs::new();
    fs_ops.set_remove_file(|_path| Some(io::Error::from(io::ErrorKind::PermissionDenied)));
    // 예외가 밖으로 나오면 안 된다
    BookManager::_evict_old_cache_with(&*fs_ops, tmp_path);
    assert!(old_file.exists());
}

/// Lines 178-179: iterdir exception
#[test]
fn test_evict_old_cache_iterdir_exception() {
    let tmp = TempDir::new().unwrap();

    let fs_ops = FaultyFs::new();
    fs_ops.set_read_dir(|_path| Some(io::Error::other("boom")));
    BookManager::_evict_old_cache_with(&*fs_ops, tmp.path());
}

// ---- coverage: _get_epub_total_chapters edge cases ----

/// Line 241: spine not found in OPF
#[test]
fn test_get_epub_total_chapters_no_spine() {
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"#
    );
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    assert_eq!(BookManager::_get_epub_total_chapters(&epub), 0);
}

/// Lines 243-245: exception during chapter counting
#[test]
fn test_get_epub_total_chapters_exception() {
    let tmp = TempDir::new().unwrap();
    let epub = tmp.path().join("test.epub");
    write(&epub, b"not a zip");
    assert_eq!(BookManager::_get_epub_total_chapters(&epub), 0);
}

// ---- coverage: validate_epub (epubcheck) edge cases ----

/// Lines 339-340: os.unlink in finally fails
#[test]
fn test_validate_epub_epubcheck_json_unlink_oserror() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let epub = tmp_path.join("A/test.epub");
    mkdir(epub.parent().unwrap());
    make_epub(&epub, &minimal_spec());
    let doc = make_doc("A/test.epub", "epub");
    es.with(|s| s.default_doc = Some(doc.clone()));
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));

    // epubcheck가 만든 것처럼 유효한 JSON 출력을 쓴다
    let epubcheck_json = r#"{"messages": [], "publication": {"title": "T"}, "checker": {"nFatal": 0, "nError": 0, "nWarning": 0, "nUsage": 0, "nInfo": 0}}"#;
    deps = deps.with_runner(FakeRunner::new(move |argv| {
        for arg in argv {
            if arg.ends_with(".json") {
                write(arg, epubcheck_json);
            }
        }
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    }));

    let unlink_calls: Arc<Mutex<Vec<PathBuf>>> = Arc::new(Mutex::new(vec![]));
    let fs_ops = FaultyFs::new();
    {
        let unlink_calls = unlink_calls.clone();
        fs_ops.set_remove_file(move |path| {
            unlink_calls.lock().unwrap().push(path.to_path_buf());
            Some(io::Error::other("can't delete"))
        });
    }
    deps = deps.with_fs(fs_ops);
    let manager = BookManager::with_deps(deps);

    assert!(run(manager.validate_epub(1)).is_ok());
    assert!(!unlink_calls.lock().unwrap().is_empty());
}

/// Lines 345-347, 353, 361: messages with empty locations + publication metadata
#[test]
fn test_validate_epub_message_empty_locations() {
    let tmp = TempDir::new().unwrap();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let mut deps = make_deps(tmp_path, &es);
    let epub = tmp_path.join("A/test.epub");
    mkdir(epub.parent().unwrap());
    make_epub(&epub, &minimal_spec());
    let doc = make_doc("A/test.epub", "epub");
    es.with(|s| s.default_doc = Some(doc.clone()));
    es.on_search_by_id(move |_| Ok(Some(doc.clone())));

    let epubcheck_json = r#"{"messages": [{"severity": "WARNING", "id": "W1", "message": "test warn", "locations": []}], "publication": {"title": "T", "creator": "C", "date": "2024", "publisher": "P"}, "checker": {"nFatal": 0, "nError": 0, "nWarning": 1, "nUsage": 0, "nInfo": 0}}"#;
    deps = deps.with_runner(FakeRunner::new(move |argv| {
        for arg in argv {
            if arg.ends_with(".json") {
                write(arg, epubcheck_json);
            }
        }
        Ok(CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    }));
    let manager = BookManager::with_deps(deps);

    let result = run(manager.validate_epub(1)).expect("result");
    assert_eq!(result["messages"][0]["location"], Value::Null);
    assert!(result.get("publication").is_some());
    assert_eq!(result["publication"]["title"], "T");
}

// ---- coverage: validate_pdf edge cases ----

/// Line 372: book not found in ES
#[test]
fn test_validate_pdf_not_found() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.with(|s| s.default_doc = None);
    let (result, err) = split(run(manager.validate_pdf(999)));
    assert!(result.is_none());
    assert!(err.as_deref().unwrap().to_lowercase().contains("not found"));
}
