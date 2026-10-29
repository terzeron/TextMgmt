//! 원본: tests/test_book_manager.py (3259번째 줄 `test_validate_pdf_producer_and_creation_date`부터 파일 끝까지)
//! 전반부(1~3258줄)는 book_manager_1.rs가 맡는다. 후반부가 쓰는 헬퍼(make_manager, make_doc, asyncio_runner)는 이 파일에 복사해 자립시켰고,
//! ES 대역·zip·epub·env·로그 캡처·파일 시스템 실패 주입은 `common`의 공용 모듈을 쓴다.
//!
//! 가정한 API (아직 구현 전이다). 설계: docs/rust-test-seams.md 3장(통일안), 3.13(이름 매핑표).
//!
//! 생성과 주입 (3.12): 모든 seam은 `BookManagerDeps`의 소비형 `with_*` 빌더다. 전역 가드(`override_*`, `fs_*_override`, `testing::*`)는 없다.
//! - `BookManager::with_deps(BookManagerDeps::new(path_prefix: PathBuf, es: Arc<dyn EsManagerApi>).with_*(..)) -> BookManager`
//!   (구 `BookManager::new_for_test`. 인자 순서는 `(path, es)`다. 전역 값을 건드리지 않아 `restore_book_path_prefix` fixture가 필요 없다.)
//! - `with_fs(Arc<dyn sysdeps::FsOps>)`: 구 `fs_{rename,resolve,samefile,unlink,is_file,iterdir}_override`. resolve는 `canonicalize`, iterdir는 `read_dir`다.
//!   `shutil.move`(구 `fs_move_override`)는 `FsOps`에 별도 메서드가 없어 `rename`으로 합쳤다.
//! - `with_runner(Arc<dyn sysdeps::CommandRunner>)`: 구 `override_convert_with_libreoffice`. 변환 문자열은 `FakeRunner::office_output`, 실패는 `CommandError`.
//! - `with_file_reader(Arc<dyn FileReader>)`: 구 `loader_read_file_override`. `FileReader::read_file(&self, &Path) -> Result<Value, LoaderReadError>`.
//! - `with_book_adder(Arc<dyn BookAdder>)`: 구 `manager.override_add_book`. `BookAdder::add_book(&self) -> Result<i64, AddBookError>`.
//! - `with_task_runner(Arc<dyn TaskRunner>)`: 구 `override_thread_start(_run_inline)`. 인라인은 `InlineRunner`다.
//! - `with_pdf_cache_limits(PdfCacheLimits { max_file_bytes, max_bytes, max_entries })`, `BookManager::pdf_cache() -> &PdfReaderCache` (`clear/contains/len`, 문서 밖: `insert_raw(name, mtime, pages, size)`, `evict()`).
//! - `with_pdf_pages(Arc<dyn PdfPageOps>)`: 구 `override_get_cached_page_count`(쪽수 조회 실패 주입).
//! - `with_epub_hooks(EpubCoverHooks)`: 구 `override_find_opf_path` (`EpubCoverHooks::with_find_opf_path`).
//!
//! 문서에 아직 없는 seam (이 파일에서 `#[ignore]`로 남은 테스트만 쓴다. 구현을 정할 때 이름과 모양을 확정해야 한다):
//! - `with_max_category_result_count(i64)` (구 `override_max_category_result_count`, 값 설정자)
//! - `with_page_count_cache_max(usize)` + `BookManager::page_count_cache()` (`clear/insert/touch/evict/len/contains`) (구 `_page_count_cache_*` 정적 캐시)
//! - `with_preview_epub_validator(Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>)` (구 `override_validate_preview_epub`)
//! - `with_normpath(Arc<dyn Fn(&str) -> String + Send + Sync>)`, `with_path_is_relative_to(Arc<dyn Fn(&Path, &Path) -> io::Result<bool> + Send + Sync>)`
//! - `with_get_category_mismatches_override(Arc<dyn Fn(bool) -> Value + ..>)`, `with_get_category_mismatch_details_override(Arc<dyn Fn(&str) -> Value + ..>)` (구 인스턴스 메서드 바꿔치기)
//! - `with_category_name_check(Arc<dyn Fn(&str, &dyn Fn(&str) -> bool) -> io::Result<bool> + ..>)` (구 `override_is_safe_category_name`)
//!
//! 메서드 (Python 시그니처와 같은 이름을 쓴다. 정수는 모두 `i64`다, 3.0 규칙 6):
//! - `async fn validate_pdf(&self, book_id: i64) -> Result<Value, String>`
//! - `async fn get_book_content(&self, book_id: i64) -> BookContent` (`Text(String)` | `File(BookResponse)`. 문서의 `File(PathBuf)`와 달리 헤더를 읽는 테스트가 있어 응답을 그대로 담는다.)
//! - `async fn get_book_preview(&self, book_id: i64, chapters: Option<i64>, resource_base_url: Option<&str>) -> BookResponse`
//! - `async fn get_pdf_pages(&self, book_id: i64, start: i64, end: i64) -> BookResponse`
//! - `async fn get_html_resource(&self, book_id: i64, path: &str) -> BookResponse`
//!   (`BookResponse { status_code: u16, media_type, headers: Vec<(String, String)>, body: Vec<u8> }`. 이 파일은 `.header(name) -> Option<&str>`, `.body_text()`, `.is_file()` 도우미도 쓴다.)
//! - `async fn update_book(&self, book_id: i64, new_category: &str, new_title: &str, new_author: &str, new_path: &Path, new_type: &str, force: bool) -> Result<Value, String>`
//! - `async fn rename_category / delete_category / reload_category(&self, ..) -> Result<Value, String>`
//! - `async fn index_single_file(&self, rel: &str) -> Result<i64, String>`
//! - `async fn get_categories(&self) -> Result<Value, String>`, `get_books_in_category(&self, c) -> Result<Vec<Book>, String>`
//! - `async fn get_books_in_category_paged(&self, c, size: Option<i64>, cursor: Option<&str>) -> Result<BookPage, String>`
//! - `async fn get_latest_books(&self, size: i64, exclude: Option<Vec<String>>) -> Result<BookPage, String>`
//! - `async fn get_book / search_by_keyword_paged` (ES 읽기 경로, `spawn_blocking`으로 넘겨야 한다)
//! - `async fn _move_classified_file(&self, src: &Path, category: &str, kind: &str, opts: MoveOpts) -> Result<Value, String>`
//! - `async fn propose_category_changes(&self, category, mappings: &[(Value, Vec<Value>)], opts: ProposeOpts) -> Result<Value, String>`
//! - `async fn apply_category_changes(&self, items: &[Value], opts: ApplyOpts) -> Result<Value, String>`
//! - `fn _propose_category_for_file(&self, path, category, mappings, classifier: &dyn BookClassifierApi, use_bookstore: bool, use_content_meta: bool) -> Value`
//! - `fn _match_category_by_keywords(&self, path, category, mappings) -> (Option<String>, Vec<String>, Option<String>, Vec<(String, Vec<String>)>)`
//! - `fn _is_high_confidence(&self, classifier: &dyn BookClassifierApi, confidence: f64) -> bool`, `BookManager::GRADE_UNKNOWN: &str`
//! - `fn get_category_mismatches(&self, force_refresh: bool) -> Result<Value, String>` (3.2 `BookManagerApi`), `get_category_mismatch_details(&self, c) -> Value`
//! - 정적 함수: `BookManager::{_get_epub_total_chapters, _validate_preview_epub, _build_html_resource_url, _sanitize_html_for_viewer}`,
//!   모듈 함수 `{_safe_xml_parser, encode_category_cursor, decode_category_cursor, _declare_unbound_prefixes, _prune_dead_ncx_navpoints}`
//! - 인스턴스 메서드: `_get_cached_pdf_reader(&self, &Path) -> (Arc<Reader>, i64)` (캐시를 인스턴스가 가진다, 3.11)
//!
//! ES 대역은 `common::es_fake::FakeEsManager`(`EsManagerApi` 구현)다. 오류 주입은 `es.fail("insert", "msg")`, 그 밖의 상태는 `es.with(|s| ..)`로 바꾼다.
//! 로그 캡처는 `common::log_capture::LogCapture`, 환경 변수는 `common::env::EnvGuard`다.
//!
//! 외부 의존 가정: dev-dependency로 `zip`, `scraper`, `roxmltree`, `base64`, `tempfile`, `tokio`, `log`를 쓰고, workspace crate `sysdeps`를 쓴다.

mod common;

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::fs;
use std::io;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use backend::book::BookPage;
use backend::book_manager::{
    _declare_unbound_prefixes, _prune_dead_ncx_navpoints, _safe_xml_parser, AddBookError, ApplyOptions, BackfillThread, BookAdder, BookContent, BookManager, BookManagerDeps,
    CREATED_TIME_BACKFILL_ENV, FileReader, MAX_CATEGORY_RESULT_COUNT, MoveOpts, ProposeOptions, ReadFileError, decode_category_cursor, encode_category_cursor,
};
use backend::classifier::BookClassifierApi;
use backend::epub_cover::EpubCoverHooks;
use backend::main::{BookManagerApi, BookResponse, InlineRunner, TaskHandle, TaskRunner};
use backend::pdf::{PdfCacheLimits, PdfError, PdfPageOps};
use base64::Engine;
use common::{
    env::EnvGuard,
    epub::{EpubSpec, make_epub},
    es_fake::{Docs, FakeEsManager},
    faulty_fs::{FaultyFs, fail_on_nth_call, other_err},
    fsx::{mtime_key, set_mtime},
    log_capture::LogCapture,
    runner::FakeRunner,
    zipx::{read_zip_entry, write_zip, zip_header_offset},
};
use serde_json::{Value, json};
use sysdeps::{CommandError, FsOps, RealFs};
use tempfile::TempDir;

/// 검증 결과를 Python 원본의 `(유효 여부, 사유)`로 펼친다.
fn verdict(result: Result<(), String>) -> (bool, Option<String>) {
    match result {
        Ok(()) => (true, None),
        Err(reason) => (false, Some(reason)),
    }
}

/// `Result<Value, _>`를 Python 원본의 `(결과, 오류)`로 펼친다. 실패하면 원본처럼 빈 객체다.
fn outcome(result: Result<serde_json::Value, String>) -> (serde_json::Value, Option<String>) {
    match result {
        Ok(value) => (value, None),
        Err(error) => (serde_json::json!({}), Some(error)),
    }
}

/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split<T>(result: Result<T, String>) -> (Option<T>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

/// `Result<(), _>`를 Python 원본의 `("Ok" | "Error", 메시지)`로 펼친다.
fn as_status(result: Result<(), String>) -> (String, Option<String>) {
    match result {
        Ok(()) => ("Ok".to_string(), None),
        Err(error) => ("Error".to_string(), Some(error)),
    }
}

// ---------------------------------------------------------------------------
// 공용 헬퍼 (전반부 정의를 복사)
// ---------------------------------------------------------------------------

/// pytest `tmp_path`에 대응한다. 반환값이 drop되면 디렉토리가 지워진다.
fn tmp_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

/// `asyncio.run(coro)`에 대응한다. 동기 테스트 안에서 future를 끝까지 돌린다.
fn asyncio_runner<F: std::future::Future>(fut: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(fut)
}

/// 기본 구성의 `BookManagerDeps`. `with_*`를 더 붙인 뒤 `BookManager::with_deps`로 만든다.
fn deps(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManagerDeps {
    common::setup_env();
    BookManagerDeps::new(tmp_path.to_path_buf(), es.clone())
}

fn make_manager(tmp_path: &Path, es: &Arc<FakeEsManager>) -> BookManager {
    BookManager::with_deps(deps(tmp_path, es))
}

fn make_doc(rel_path: &str, file_type: &str) -> Value {
    json!({"category": "A", "title": "T", "author": "U", "file_path": rel_path, "file_type": file_type, "file_size": 1, "updated_time": "2024-01-01T00:00:00.000000", "summary": "S"})
}

const CONTAINER_XML_CONTENT_OPF: &str = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;

/// `_make_minimal_epub`의 기본 OPF.
const MINIMAL_OPF: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#;

/// `_make_minimal_epub`의 뼈대. OPF는 루트의 `content.opf`, 챕터는 `ch1.xhtml` 하나다.
/// OPF를 바꾸려면 `.with_opf(..)`, 멤버를 더하려면 `.with_extra(..)`를 붙인다.
fn minimal_spec() -> EpubSpec {
    let mut spec = EpubSpec::minimal();
    spec.chapters = vec![("ch1.xhtml".to_string(), "<html><body><p>Hello</p></body></html>".to_string())];
    spec.with_opf(MINIMAL_OPF)
}

// TODO(common): lopdf가 아니라 /Info 사전을 직접 쓰는 최소 PDF 생성기. common::pdf::PdfBuilder에 `info` 지원이 생기면 지운다.
/// pypdf `PdfWriter.add_blank_page`로 만든 PDF에 대응한다. `info`는 /Info 사전의 (키, 값)이다.
fn pdf_bytes(pages: usize, width: u32, height: u32, info: &[(&str, &str)]) -> Vec<u8> {
    let mut objs: Vec<String> = vec!["<< /Type /Catalog /Pages 2 0 R >>".to_string()];
    let kids: Vec<String> = (0..pages).map(|i| format!("{} 0 R", 3 + i)).collect();
    objs.push(format!("<< /Type /Pages /Kids [{}] /Count {pages} >>", kids.join(" ")));
    for _ in 0..pages {
        objs.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] >>"));
    }
    if !info.is_empty() {
        let body: String = info.iter().map(|(k, v)| format!("/{k} ({v}) ")).collect();
        objs.push(format!("<< {body}>>"));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, obj) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{obj}\nendobj\n", i + 1).as_bytes());
    }
    let xref_pos = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for off in &offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    let info_ref = if info.is_empty() { String::new() } else { format!(" /Info {} 0 R", 3 + pages) };
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R{info_ref} >>\nstartxref\n{xref_pos}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

/// 612x792 빈 페이지 `pages`장짜리 PDF.
fn blank_pdf_bytes(pages: usize, info: &[(&str, &str)]) -> Vec<u8> {
    pdf_bytes(pages, 612, 792, info)
}

fn blank_pdf_bytes_sized(pages: usize, width: u32, height: u32) -> Vec<u8> {
    pdf_bytes(pages, width, height, &[])
}

fn allowed_set(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|p| p.to_string()).collect()
}

/// 카테고리 -> 키워드 매핑. Python dict의 삽입 순서를 지키려고 Vec로 둔다.
/// 키/키워드가 문자열이 아닌 경우(예: 정수)를 만들 수 있도록 값은 `Value`다.
type Mappings = Vec<(Value, Vec<Value>)>;

fn mappings(pairs: &[(&str, &[&str])]) -> Mappings {
    pairs.iter().map(|(cat, kws)| (json!(cat), kws.iter().map(|k| json!(k)).collect())).collect()
}

/// `MagicMock()`으로 만든 스레드 대역. `is_alive`와 `start` 호출 횟수만 본다.
struct FakeBackfillThread {
    alive: bool,
    start_calls: Mutex<usize>,
}

impl FakeBackfillThread {
    fn alive() -> Self {
        Self { alive: true, start_calls: Mutex::new(0) }
    }

    fn start_calls(&self) -> usize {
        *self.start_calls.lock().unwrap()
    }
}

impl BackfillThread for FakeBackfillThread {
    fn is_alive(&self) -> bool {
        self.alive
    }

    fn start(&self) -> io::Result<()> {
        *self.start_calls.lock().unwrap() += 1;
        Ok(())
    }
}

// ---- seam 어댑터 (공용 모듈에 아직 없는 것만 이 파일에 둔다) ----

/// 구 `utils.parser_timeout.ParserTimeout`. `FileReader`는 오류를 돌려주지 못해(빈 묶음이 곧 실패다) 이 값은 어댑터가 빈 묶음으로 바꾼다.
#[derive(Debug)]
#[allow(dead_code)]
struct ParserTimeout {
    message: String,
}

/// 구 `Loader.read_file`이 던지는 오류.
#[derive(Debug)]
#[allow(dead_code)]
enum LoaderReadError {
    Timeout(ParserTimeout),
    Other(String),
}

type ReadFileFn = dyn Fn(&Path) -> Result<Value, LoaderReadError> + Send + Sync;

// TODO(common): 클로저를 `FileReader`로 감싸는 어댑터. 문서 3.7의 `fx::fetcher`처럼 `common::fx`에 두면 된다.
struct FnFileReader(Box<ReadFileFn>);

impl FileReader for FnFileReader {
    /// 객체 `{"<book_id>": doc}`를 문서 묶음으로 바꾼다. 오류와 객체가 아닌 값은 빈 묶음이다.
    fn read_file(&self, path: &Path, _path_prefix: Option<&Path>) -> Docs {
        match (self.0)(path) {
            Ok(Value::Object(map)) => map.into_iter().filter_map(|(k, v)| Some((k.parse().ok()?, v))).collect(),
            _ => Docs::new(),
        }
    }

    fn try_read_file(&self, path: &Path, path_prefix: Option<&Path>, _skip_text: bool) -> Result<Docs, ReadFileError> {
        match (self.0)(path) {
            Err(LoaderReadError::Timeout(timeout)) => Err(ReadFileError::Timeout(timeout.message)),
            Err(LoaderReadError::Other(message)) => Err(ReadFileError::Other(message)),
            Ok(_) => Ok(self.read_file(path, path_prefix)),
        }
    }
}

/// `BookResponse` 읽기 도우미(테스트 로컬). `is_file`은 본문이 있으면 참이다(파일을 읽어 담은 응답).
trait ResponseExt {
    fn header(&self, name: &str) -> Option<&str>;
    fn body_text(&self) -> String;
    fn is_file(&self) -> bool;
}

impl ResponseExt for BookResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }

    fn body_text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    fn is_file(&self) -> bool {
        !self.body.is_empty()
    }
}

/// `apply_category_changes`의 Python 키워드 인자 묶음(테스트 로컬). 구현의 `ApplyOptions`로 바꿔 부른다.
struct ApplyOpts {
    allowed: BTreeSet<String>,
    on_item_done: Option<Box<dyn Fn(Value) + Send + Sync>>,
    should_continue: Option<Box<dyn Fn() -> bool + Send + Sync>>,
}

impl ApplyOpts {
    fn new(allowed: BTreeSet<String>) -> Self {
        Self { allowed, on_item_done: None, should_continue: None }
    }

    fn on_item_done(mut self, f: Box<dyn Fn(Value) + Send + Sync>) -> Self {
        self.on_item_done = Some(f);
        self
    }

    fn should_continue(mut self, f: Box<dyn Fn() -> bool + Send + Sync>) -> Self {
        self.should_continue = Some(f);
        self
    }
}

async fn apply_opts(manager: &BookManager, items: &[Value], opts: ApplyOpts) -> (Value, Option<String>) {
    let allowed: std::collections::HashSet<String> = opts.allowed.iter().cloned().collect();
    outcome(
        manager
            .apply_category_changes_with(items, &allowed, ApplyOptions { on_item_done: opts.on_item_done.as_deref(), should_continue: opts.should_continue.as_deref(), ..ApplyOptions::default() })
            .await,
    )
}

/// `propose_category_changes`의 Python 키워드 인자 묶음(테스트 로컬).
struct ProposeOpts {
    use_bookstore: bool,
    use_content_meta: bool,
    on_progress: Option<Box<dyn Fn(Value) + Send + Sync>>,
}

/// 매핑은 입력 순서를 지킨 목록이다. 문자열이 아닌 카테고리는 JSON 객체 키가 될 수 없어 구현이 어차피 건너뛰므로 여기서 뺀다.
async fn propose_opts(manager: &BookManager, category: &str, mappings: &Mappings, opts: ProposeOpts) -> (Value, Option<String>) {
    let object: serde_json::Map<String, Value> = mappings.iter().filter_map(|(k, v)| Some((k.as_str()?.to_string(), Value::Array(v.clone())))).collect();
    let options = ProposeOptions {
        content_type: Some("book"),
        delay: Some(0.0),
        use_bookstore: Some(opts.use_bookstore),
        use_content_meta: Some(opts.use_content_meta),
        on_progress: opts.on_progress.as_deref(),
    };
    outcome(manager.propose_category_changes(category, Some(&Value::Object(object)), options).await)
}

/// 구 `loader_read_file_override(closure)`.
fn file_reader(f: impl Fn(&Path) -> Result<Value, LoaderReadError> + Send + Sync + 'static) -> Arc<dyn FileReader> {
    Arc::new(FnFileReader(Box::new(f)))
}

type AddBookFn = dyn Fn() -> Result<i64, AddBookError> + Send + Sync;

// TODO(common): 클로저를 `BookAdder`로 감싸는 어댑터.
struct FnBookAdder(Box<AddBookFn>);

impl BookAdder for FnBookAdder {
    fn add_book(&self) -> Result<i64, AddBookError> {
        (self.0)()
    }
}

/// 구 `manager.override_add_book(closure)`.
fn book_adder(f: impl Fn() -> Result<i64, AddBookError> + Send + Sync + 'static) -> Arc<dyn BookAdder> {
    Arc::new(FnBookAdder(Box::new(f)))
}

type ListDirFn = dyn Fn(&Path) -> Vec<PathBuf> + Send + Sync;

// TODO(common): `FaultyFs`는 실패만 주입하고 `read_dir` 결과를 바꾸지 못한다. `FaultyFs::set_read_dir_result(Fn(&Path) -> Vec<PathBuf>)`가 생기면 지운다.
/// 구 `fs_iterdir_override`: `read_dir`만 고정된 목록을 돌려주고 나머지는 안쪽 `FsOps`에 맡긴다.
struct ListDirFs {
    inner: Arc<dyn FsOps>,
    listing: Box<ListDirFn>,
}

impl ListDirFs {
    fn new(listing: impl Fn(&Path) -> Vec<PathBuf> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self { inner: Arc::new(RealFs), listing: Box::new(listing) })
    }
}

impl FsOps for ListDirFs {
    fn read_dir(&self, p: &Path) -> io::Result<Vec<PathBuf>> {
        Ok((self.listing)(p))
    }

    fn metadata(&self, p: &Path) -> io::Result<fs::Metadata> {
        self.inner.metadata(p)
    }

    fn is_file(&self, p: &Path) -> io::Result<bool> {
        self.inner.is_file(p)
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.inner.rename(from, to)
    }

    fn remove_file(&self, p: &Path) -> io::Result<()> {
        self.inner.remove_file(p)
    }

    fn remove_dir_all(&self, p: &Path) -> io::Result<()> {
        self.inner.remove_dir_all(p)
    }

    fn canonicalize(&self, p: &Path) -> io::Result<PathBuf> {
        self.inner.canonicalize(p)
    }

    fn same_file(&self, a: &Path, b: &Path) -> io::Result<bool> {
        self.inner.same_file(a, b)
    }

    fn open(&self, p: &Path) -> io::Result<fs::File> {
        self.inner.open(p)
    }

    fn write(&self, p: &Path, data: &[u8]) -> io::Result<()> {
        self.inner.write(p, data)
    }

    fn disk_free(&self, dir: &Path) -> io::Result<u64> {
        self.inner.disk_free(dir)
    }

    fn sync_file(&self, f: &fs::File) -> io::Result<()> {
        self.inner.sync_file(f)
    }

    fn lock_exclusive(&self, f: &fs::File, nonblocking: bool) -> io::Result<()> {
        self.inner.lock_exclusive(f, nonblocking)
    }
}

// TODO(common): `TaskRunner::spawn`이 `Result`가 아니라 스레드 시작 실패를 표현할 수 없다. 문서 3.2를 `io::Result<Box<dyn TaskHandle>>`로 확정하기 전까지 이 대역은 시작 시 panic한다.
/// 구 `override_thread_start(|| Err(..))`: 작업을 띄우려는 순간 실패하는 실행기.
struct FailingStartRunner {
    message: &'static str,
}

impl TaskRunner for FailingStartRunner {
    fn spawn(&self, _name: &str, _job: Box<dyn FnOnce() + Send>) -> Box<dyn TaskHandle> {
        panic!("{}", self.message)
    }

    fn spawn_blocking_observed(&self, _name: &str) {}
}

// TODO(common): `common::fake_pdf`(다음 단계)에 `PdfPageOps` 대역이 생기면 지운다.
/// 구 `override_get_cached_page_count(|_| Err(OSError(28)))`: 쪽수 조회가 항상 실패하는 `PdfPageOps`.
struct NoSpacePdfPages;

impl PdfPageOps for NoSpacePdfPages {
    fn page_count(&self, _path: &Path) -> Result<usize, PdfError> {
        Err(PdfError { kind: "OSError".to_string(), message: "[Errno 28] No space left on device".to_string() })
    }

    fn extract_pages(&self, _path: &Path, _start: usize, _end: usize) -> Result<Vec<u8>, PdfError> {
        Err(PdfError { kind: "OSError".to_string(), message: "[Errno 28] No space left on device".to_string() })
    }
}

// ---------------------------------------------------------------------------
// 테스트 본문 (원본 순서)
// ---------------------------------------------------------------------------
// ---- coverage: validate_pdf ----

#[test]
fn test_validate_pdf_producer_and_creation_date() {
    // Lines 395, 397: PDF with Producer and CreationDate metadata
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let pdf = tmp_path.join("A").join("test.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();

    // 최소한의 유효한 PDF를 만든다.
    fs::write(&pdf, blank_pdf_bytes(1, &[("Producer", "TestProducer"), ("CreationDate", "D:20240101"), ("Title", "Test"), ("Author", "Author")])).unwrap();

    es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

    let result = asyncio_runner(manager.validate_pdf(1)).expect("result is not None");
    assert!(result["publication"].get("producer").is_some());
    assert!(result["publication"].get("creation_date").is_some());
}

// ---- coverage: get_book_content edge case ----

#[test]
fn test_get_book_content_not_found() {
    // Line 429: book not in ES
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_doc(None);
    let result = asyncio_runner(manager.get_book_content(999));
    assert_eq!(result, BookContent::Text(String::new()));
}

#[test]
fn test_get_book_content_reports_malformed_es_book() {
    let tmp = tmp_dir();
    let mut doc = make_doc("A/missing.txt", "txt");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));

    let result = asyncio_runner(manager.get_book_content(1));
    assert!(matches!(result, BookContent::Error(error) if error.contains("category")));
}

// ---- coverage: get_book_preview edge cases ----

#[test]
fn test_get_book_preview_not_found() {
    // Lines 443-444: book not in ES
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_doc(None);
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 999, None, None));
    assert_eq!(result.status_code, 404);
}

#[test]
fn test_get_book_preview_file_not_found() {
    // Lines 447-448: file not found on disk
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_doc(Some(make_doc("A/missing.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(result.status_code, 404);
}

#[test]
fn test_get_book_preview_epub_cache_hit() {
    // Lines 493-494: EPUB preview cache hit
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec());
    es.set_doc(Some(make_doc("A/test.epub", "epub")));

    // 더 최근 mtime을 가진 캐시 파일을 만든다.
    let cache_dir = tmp_path.join(".preview_cache");
    fs::create_dir_all(&cache_dir).unwrap();
    BookManager::_get_epub_total_chapters(&epub);
    let cache_file = cache_dir.join(format!("1_ch3_v{}.epub", BookManager::EPUB_IMAGE_POLICY_VERSION));
    make_epub(&cache_file, &minimal_spec());

    set_mtime(&cache_file, SystemTime::now() + Duration::from_secs(1000));

    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, Some(3), None));
    assert_eq!(result.status_code, 200);
}

#[test]
fn test_get_book_preview_epub_opf_keyerror() {
    // Lines 516-518: OPF file in container.xml but missing from archive
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    let container_xml = r#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="MISSING.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    write_zip(&epub, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", container_xml.as_bytes())]);
    // zip 안에 MISSING.opf는 없다.
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(result.status_code, 422);
}

#[test]
fn test_get_book_preview_epub_opf_namespace_fix() {
    // Lines 523-524: OPF with opf: prefix but no xmlns:opf declaration
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <metadata>
    <opf:meta name="cover" content="cover-image"/>
  </metadata>
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    // 크래시하지 않아야 한다. 네임스페이스가 주입된다.
    assert!([200, 422, 500].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_no_spine() {
    // Lines 540-547: EPUB without spine element → fallback to manifest order
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
</package>"#
    );
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_chapter_missing_from_zip() {
    // Lines 580-582: chapter file missing from ZIP archive
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
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    // ch2.xhtml은 zip에 없다.
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, Some(5), None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_svg_image_tag() {
    // Lines 591-593: chapter with SVG <image> tags
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="cover.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let chapter = r#"<html><body><svg><image xlink:href="cover.png"/></svg></body></html>"#;
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(vec![0u8; 100]);
    write_zip(
        &epub,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()),
            ("content.opf", opf.as_bytes()),
            ("ch1.xhtml", chapter.as_bytes()),
            ("cover.png", &png),
        ],
    );
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_css_missing() {
    // Lines 613-614: CSS file missing from ZIP
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let chapter = r#"<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>"#;
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    // style.css는 zip에 없다.
    write_zip(&epub, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()), ("content.opf", opf.as_bytes()), ("ch1.xhtml", chapter.as_bytes())]);
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_large_font_skip() {
    // Lines 626-633: large font file skipped, missing font skipped
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
    <item id="font1" href="big.ttf" media-type="font/ttf"/>
    <item id="font2" href="missing.woff" media-type="font/woff"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let chapter = r#"<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>"#;
    let css = "@font-face { font-family: \"Big\"; src: url(\"big.ttf\"); }\n@font-face { font-family: \"Missing\"; src: url(\"missing.woff\"); }\nbody { font-family: serif; }";
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    // big.ttf는 500KB를 넘는다. missing.woff는 zip에 없다.
    let big = vec![0u8; 600 * 1024];
    write_zip(
        &epub,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()),
            ("content.opf", opf.as_bytes()),
            ("ch1.xhtml", chapter.as_bytes()),
            ("style.css", css.as_bytes()),
            ("big.ttf", &big),
        ],
    );
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_guide_element() {
    // Lines 648, 653-657: EPUB with guide element
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
  <guide>
    <reference type="toc" href="toc.xhtml"/>
    <reference type="text" href="ch1.xhtml"/>
  </guide>
</package>"#
    );
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec().with_opf(&opf));
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_ncx_navpoint_filtering() {
    // Lines 686-694: NCX navPoint filtering removes missing file references
    let opf_ns = "http://www.idpf.org/2007/opf";
    let ncx_ns = "http://www.daisy.org/z3986/2005/ncx/";
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
    let ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}">
  <navMap>
    <navPoint id="np1"><navLabel><text>Ch1</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2"><navLabel><text>Ch99</text></navLabel><content src="missing.xhtml"/></navPoint>
  </navMap>
</ncx>"#
    );
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec().with_opf(&opf).with_extra("toc.ncx", ncx.into_bytes()));
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_css_font_face_strip() {
    // Line 712: CSS @font-face block removed when font not in files_to_include
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="css1" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let chapter = r#"<html><head><link href="style.css" rel="stylesheet"/></head><body><p>Hello</p></body></html>"#;
    let css = "@font-face { font-family: \"Missing\"; src: url(\"nothere.woff\"); }\nbody { color: red; }";
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    write_zip(
        &epub,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()),
            ("content.opf", opf.as_bytes()),
            ("ch1.xhtml", chapter.as_bytes()),
            ("style.css", css.as_bytes()),
        ],
    );
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_file_missing_in_write() {
    // Lines 717-718: file missing when writing output EPUB
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="img1" href="cover.png" media-type="image/png"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let chapter = r#"<html><body><img src="cover.png"/></body></html>"#;
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    // cover.png는 zip에 없다. manifest에 있고 HTML에서 참조한다.
    write_zip(&epub, &[("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()), ("content.opf", opf.as_bytes()), ("ch1.xhtml", chapter.as_bytes())]);
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_get_book_preview_epub_validation_fail() {
    // Lines 723-725: generated EPUB fails validation
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec());
    let es = FakeEsManager::new();
    // 가정한 seam: 미리보기 EPUB 검증 함수를 교체한다. 문서에 없는 seam이다(파일 머리말 참고).
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_preview_epub_validator(Arc::new(|_path| Err("bad epub".to_string()))));
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(result.status_code, 422);
    assert!(result.body_text().contains("bad epub"));
}

#[test]
fn test_get_book_preview_epub_bad_zip() {
    // Lines 730-732: corrupted ZIP
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    fs::write(&epub, b"not a zip file").unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(result.status_code, 422);
}

#[test]
fn test_get_book_preview_epub_general_exception() {
    // Lines 733-735: general exception during EPUB processing
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec());
    let es = FakeEsManager::new();
    // TODO(common): `EpubCoverHooks::with_find_opf_path`는 `Fn() -> String`이라 `RuntimeError("boom")`을 돌려줄 수 없다. 문서가 `Result`를 받도록 정해지면 고친다. 지금은 panic으로 대신한다.
    let hooks = EpubCoverHooks::default().with_find_opf_path(Arc::new(|| panic!("boom")));
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_epub_hooks(hooks));
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(result.status_code, 500);
    assert_eq!(result.body_text(), "EPUB preview failed");
}

#[test]
fn test_get_book_preview_doc_exception() {
    // Lines 750-752: .doc conversion exception
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let doc = tmp_path.join("A").join("test.doc");
    fs::create_dir_all(doc.parent().unwrap()).unwrap();
    fs::write(&doc, b"fake doc content").unwrap();
    let es = FakeEsManager::new();
    let runner = FakeRunner::returning(Err(CommandError::Failed("boom".to_string())));
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(runner));
    es.set_doc(Some(make_doc("A/test.doc", "doc")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(result.status_code, 500);
    assert_eq!(result.body_text(), ".DOC preview failed");
}

// ---- coverage: reload_category edge cases ----

#[test]
fn test_reload_category_empty_name() {
    // Line 1032: empty category name
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("", None)));
    let err = err.expect("err is not None");
    assert!(err.contains("비어있습니다"));
}

#[test]
fn test_reload_category_path_traversal() {
    // Line 1034: category with '..'
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("../etc", None)));
    assert!(err.is_some());
}

#[test]
fn test_reload_category_path_escape() {
    // Line 1038: resolved path escapes path_prefix
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let evil = tmp_path.join("evil");
    symlink("/tmp", &evil).unwrap();
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("evil", None)));
    assert!(err.is_some());
}

#[test]
fn test_reload_category_not_a_directory() {
    // Lines 1040-1041: path is not a directory
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::write(tmp_path.join("notadir"), "file").unwrap();
    let (_result, err) = outcome(asyncio_runner(manager.reload_category("notadir", None)));
    let err = err.expect("err is not None");
    assert!(err.contains("디렉토리를 찾을 수 없습니다"));
}

// ---- coverage: get_pdf_pages edge cases ----

#[test]
fn test_get_pdf_pages_not_found() {
    // Line 1080: book not in ES
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_doc(None);
    let result = asyncio_runner(manager.get_pdf_pages(999, 1, 1));
    assert_eq!(result.status_code, 404);
}

#[test]
fn test_get_pdf_pages_not_pdf() {
    // Line 1085: file is not a PDF
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let txt = tmp_path.join("A").join("test.txt");
    fs::create_dir_all(txt.parent().unwrap()).unwrap();
    fs::write(&txt, "hello").unwrap();
    es.set_doc(Some(make_doc("A/test.txt", "txt")));
    let result = asyncio_runner(manager.get_pdf_pages(1, 1, 1));
    assert_eq!(result.status_code, 400);
}

#[test]
fn test_get_pdf_pages_happy_path_and_cache() {
    // Lines 1091-1122: happy path + cache hit
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let pdf = tmp_path.join("A").join("test.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    fs::write(&pdf, blank_pdf_bytes(2, &[])).unwrap();

    es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

    // 첫 호출: 생성하고 캐시한다.
    let result = asyncio_runner(manager.get_pdf_pages(1, 1, 1));
    assert_eq!(result.status_code, 200);
    assert_eq!(result.header("x-total-pages"), Some("2"));

    // 두 번째 호출: 캐시 적중.
    let result2 = asyncio_runner(manager.get_pdf_pages(1, 1, 1));
    assert_eq!(result2.status_code, 200);
}

#[test]
fn test_get_pdf_pages_start_exceeds_total() {
    // Lines 1096-1097: start page exceeds total pages
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let pdf = tmp_path.join("A").join("test.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    fs::write(&pdf, blank_pdf_bytes(1, &[])).unwrap();

    es.set_doc(Some(make_doc("A/test.pdf", "pdf")));

    let result = asyncio_runner(manager.get_pdf_pages(1, 100, 200));
    assert_eq!(result.status_code, 400);
    assert!(result.body_text().contains("exceeds total pages"));
}

// ---- coverage: rename_category new_category path traversal ----

#[test]
fn test_rename_category_new_path_traversal() {
    // Line 1149: new_category resolved path escapes path_prefix
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_counts(&[("old", 1)]);
    let evil = tmp_path.join("evil");
    symlink("/tmp", &evil).unwrap();
    let (_result, err) = outcome(asyncio_runner(manager.rename_category("old", "evil")));
    assert!(err.is_some());
}

// ---- coverage: index_single_file unsupported format ----

#[test]
fn test_index_single_file_unsupported() {
    // Line 1011: Loader.read_file returns falsy
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // Loader::read_file이 빈 dict를 돌려주게 한다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_file_reader(file_reader(|_path| Ok(json!({})))));
    let f = tmp_path.join("test.xyz");
    fs::write(&f, "data").unwrap();

    let (result, err) = split(asyncio_runner(manager.index_single_file("test.xyz")));
    assert!(result.is_none());
    assert!(err.expect("err").contains("지원하지 않는"));
}

// ---- coverage: rename_category old/new count checks ----

#[test]
fn test_rename_category_old_count_zero() {
    // Line 1155: old_category has 0 documents
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 0), ("new", 0)]);
    let (_result, err) = outcome(asyncio_runner(manager.rename_category("old", "new")));
    assert!(err.expect("err").contains("문서가 없습니다"));
}

#[test]
fn test_rename_category_new_count_nonzero() {
    // Line 1159: new_category already has documents
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_counts(&[("old", 1), ("new", 5)]);
    let (_result, err) = outcome(asyncio_runner(manager.rename_category("old", "new")));
    let err = err.expect("err");
    assert!(err.contains("이미"));
    assert!(err.contains('5'));
}

// ---- coverage: update_book IOError and force overwrite ----

#[test]
fn test_update_book_ioerror_on_rename() {
    // Lines 829-830: IOError during file rename
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // 파일 이동(rename)이 항상 실패하게 한다.
    let faulty = FaultyFs::new();
    faulty.set_rename(|_from, _to| Some(other_err("disk error")));
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
    fs::create_dir(tmp_path.join("A")).unwrap();
    let src = tmp_path.join("A").join("src.txt");
    fs::write(&src, "content").unwrap();
    es.set_doc(Some(make_doc("A/src.txt", ".txt")));

    let (result, err) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "A", "dst", "U", &tmp_path.join("A").join("dst.txt"), "txt", false)));
    assert_eq!(result, "Error");
    assert!(err.expect("err").contains("can't move"));
}

#[test]
fn test_update_book_force_overwrite() {
    // Line 824: force overwrite when destination exists
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir(tmp_path.join("A")).unwrap();
    let src = tmp_path.join("A").join("src.txt");
    let dst = tmp_path.join("A").join("dst.txt");
    fs::write(&src, "source").unwrap();
    fs::write(&dst, "existing").unwrap();
    es.set_doc(Some(make_doc("A/src.txt", ".txt")));

    let (result, _err) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "A", "dst", "U", &dst, "txt", true)));
    assert_eq!(result, "Ok");
    assert!(!src.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "source");
}

// ---- coverage: get_book_preview epub with many spine items (line 648) ----

#[test]
fn test_get_book_preview_epub_spine_trimmed() {
    // Line 648: spine refs not in chapter_idrefs are removed
    let opf_ns = "http://www.idpf.org/2007/opf";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch2" href="ch2.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch3" href="ch3.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch4" href="ch4.xhtml" media-type="application/xhtml+xml"/>
    <item id="ch5" href="ch5.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="ch1"/>
    <itemref idref="ch2"/>
    <itemref idref="ch3"/>
    <itemref idref="ch4"/>
    <itemref idref="ch5"/>
  </spine>
</package>"#
    );
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    let chapters: Vec<(String, String)> = (1..6).map(|i| (format!("ch{i}.xhtml"), format!("<html><body><p>Chapter {i}</p></body></html>"))).collect();
    let mut entries: Vec<(&str, &[u8])> = vec![("mimetype", b"application/epub+zip"), ("META-INF/container.xml", CONTAINER_XML_CONTENT_OPF.as_bytes()), ("content.opf", opf.as_bytes())];
    for (name, body) in &chapters {
        entries.push((name.as_str(), body.as_bytes()));
    }
    write_zip(&epub, &entries);
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    // 2개 챕터만 요청한다. ch3-ch5는 spine에서 빠져야 한다.
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, Some(2), None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

// ---- coverage: get_categories and get_books_in_category (unit test) ----

#[test]
fn test_get_categories_unit() {
    // Lines 282-284: get_categories returns aggregation
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    es.set_aggregate(&[("Cat1", 3), ("Cat2", 5)]);
    let cats = asyncio_runner(BookManagerApi::get_categories(&manager)).unwrap();
    assert_eq!(cats, json!({"Cat1": 3, "Cat2": 5}));
}

#[test]
fn test_get_books_in_category_unit() {
    // Line 289: get_books_in_category returns books
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let doc = make_doc("A/test.txt", ".txt");
    es.set_category_docs(vec![(1, doc, 1.0)]);
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    fs::write(tmp_path.join("A").join("test.txt"), "x").unwrap();
    let books = asyncio_runner(manager.get_books_in_category("A")).unwrap();
    assert_eq!(books.len(), 1);
}

// ---- HWP3 네이티브 파서 fallback 프리뷰 테스트 ----

fn hwp_test_dir() -> PathBuf {
    common::project_root().join("tests").join("books").join("_hwp")
}

/// 원본 테스트 파일을 복사하고 경로를 돌려준다. 파일이 없으면 panic한다.
fn copy_hwp_sample(tmp_path: &Path, sample_name: &str, dest_name: &str) -> PathBuf {
    let hwp_src = hwp_test_dir().join(sample_name);
    assert!(hwp_src.exists(), "테스트 파일 tests/books/_hwp/{sample_name}이 필요하다");
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let hwp_file = tmp_path.join("A").join(dest_name);
    fs::copy(&hwp_src, &hwp_file).unwrap();
    hwp_file
}

#[test]
fn test_hwp_preview_fallback_when_libreoffice_empty() {
    // LibreOffice가 빈 문자열 반환 시 hwp3 파서로 fallback하여 HTML 생성
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    copy_hwp_sample(tmp_path, "v2.10_KYOKA.hwp", "kyoka.hwp");
    let doc = make_doc("A/kyoka.hwp", "hwp");
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
    es.set_doc(Some(doc));
    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(resp.status_code, 200);
    let body = resp.body_text();
    assert!(body.contains("영동") || body.contains("교가"));
    assert!(body.contains("<p>"));
}

#[test]
fn test_hwp_preview_fallback_v3_00() {
    // V3.00 파일의 fallback 프리뷰
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    copy_hwp_sample(tmp_path, "v3.00_현대시사전.hwp", "poem.hwp");
    let doc = make_doc("A/poem.hwp", "hwp");
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
    es.set_doc(Some(doc));
    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(resp.status_code, 200);
    let body = resp.body_text();
    assert!(body.chars().count() > 50);
}

#[test]
fn test_hwp_preview_fallback_caches_result() {
    // fallback 결과가 캐시 파일에 저장되는지 확인
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    copy_hwp_sample(tmp_path, "v2.10_KYOKA.hwp", "kyoka.hwp");
    let doc = make_doc("A/kyoka.hwp", "hwp");
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
    es.set_doc(Some(doc));
    asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    let cache_file = tmp_path.join(".preview_cache").join("1.html");
    assert!(cache_file.exists());
    assert!(fs::read_to_string(&cache_file).unwrap().chars().count() > 50);
}

#[test]
fn test_hwp_preview_no_fallback_for_doc() {
    // DOC 파일은 hwp3 fallback을 사용하지 않음
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let doc_file = tmp_path.join("A").join("test.doc");
    fs::write(&doc_file, b"fake doc content").unwrap();
    let doc = make_doc("A/test.doc", "doc");
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
    es.set_doc(Some(doc));
    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    // DOC는 빈 html_content → 캐시 안 되고 preview 없음
    assert!([200, 400, 500].contains(&resp.status_code) || resp.status_code == 755, "status={}", resp.status_code);
    // 빈 content면 status 755 안 나옴
}

#[test]
fn test_hwp_preview_fallback_both_fail() {
    // LibreOffice도 실패하고 hwp3 파서도 빈 결과인 경우 (V1.20)
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    copy_hwp_sample(tmp_path, "v1.20_부하의약혼녀.hwp", "old.hwp");
    let doc = make_doc("A/old.hwp", "hwp");
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
    es.set_doc(Some(doc));
    asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    // V1.20은 hwp3 파서도 빈 결과 → 캐시 미생성, 400/500 가능
    let cache_file = tmp_path.join(".preview_cache").join("1.html");
    assert!(!cache_file.exists());
}

#[test]
fn test_hwp_preview_libreoffice_success_no_fallback() {
    // LibreOffice가 정상 결과 반환 시 hwp3 fallback 미사용
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    copy_hwp_sample(tmp_path, "v2.10_KYOKA.hwp", "kyoka.hwp");
    let doc = make_doc("A/kyoka.hwp", "hwp");
    let es = FakeEsManager::new();
    // LibreOffice처럼 `--outdir`에 `<이름>.html`을 쓰는 변환기 대역이다(`office_output`은 stdout에만 담아 실제 변환기와 다르다).
    let runner = FakeRunner::new(|argv| {
        let outdir = PathBuf::from(&argv[argv.iter().position(|a| a == "--outdir").unwrap() + 1]);
        fs::write(outdir.join("kyoka.html"), "<p>LibreOffice 결과</p>").unwrap();
        Ok(sysdeps::CommandOutput { returncode: 0, stdout: vec![], stderr: vec![] })
    });
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(runner));
    es.set_doc(Some(doc));
    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert_eq!(resp.status_code, 200);
    let body = resp.body_text();
    assert!(body.contains("LibreOffice"));
    assert!(!body.contains("영동")); // fallback 미사용 확인
}

#[test]
fn test_hwp_preview_html_escaping() {
    // fallback HTML 출력에서 특수문자가 이스케이프되는지 확인
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    copy_hwp_sample(tmp_path, "v2.10_PAGER.hwp", "pager.hwp");
    let doc = make_doc("A/pager.hwp", "hwp");
    let es = FakeEsManager::new();
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_runner(FakeRunner::office_output("")));
    es.set_doc(Some(doc));
    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    if resp.status_code == 200 {
        let body = resp.body_text();
        // XSS 방지: <script> 같은 태그가 이스케이프되어야 함
        assert!(!body.contains("<script>"));
    }
}

#[test]
fn test_get_book_preview_html_sanitizes_active_content_and_rewrites_resources() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let html_file = tmp_path.join("A").join("test.html");
    fs::write(
        &html_file,
        r##"
        <html>
          <head>
            <meta http-equiv="refresh" content="0;url=https://evil.example.com" />
            <script>alert(1)</script>
            <link rel="stylesheet" href="style.css" />
            <link rel="preload" href="evil.js" />
          </head>
          <body onload="steal()">
            <iframe src="https://evil.example.com/embed"></iframe>
            <img src="cover.png" onclick="hack()" />
            <a href="chapter2.html">next</a>
            <a href="#section1">toc</a>
          </body>
        </html>
        "##,
    )
    .unwrap();
    fs::write(tmp_path.join("A").join("style.css"), "body { color: red; }").unwrap();
    fs::write(tmp_path.join("A").join("cover.png"), b"png").unwrap();
    let doc = make_doc("A/test.html", "html");
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(doc));

    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, Some("/html-resource/1")));
    let body = resp.body_text();

    assert_eq!(resp.status_code, 200);
    assert!(resp.header("content-security-policy").expect("csp").starts_with("sandbox;"));
    assert!(!body.contains("<script"));
    assert!(!body.contains("<iframe"));
    assert!(!body.contains("onload="));
    assert!(!body.contains("onclick="));
    assert!(!body.contains("http-equiv"));
    assert!(body.contains(r#"href="/html-resource/1?path=style.css""#));
    assert!(body.contains(r#"src="/html-resource/1?path=cover.png""#));
    assert!(body.contains(r##"href="#section1""##));
    assert!(!body.contains(r#"href="chapter2.html""#));
}

#[test]
fn test_get_html_resource_allows_local_whitelisted_files_only() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let html_file = tmp_path.join("A").join("test.html");
    fs::write(&html_file, "<html></html>").unwrap();
    let css_file = tmp_path.join("A").join("style.css");
    fs::write(&css_file, "body{}").unwrap();
    let doc = make_doc("A/test.html", "html");
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(doc));

    let resp = asyncio_runner(manager.get_html_resource(1, "style.css"));
    assert!(resp.is_file());
    assert_eq!(resp.status_code, 200);
    assert!(resp.header("content-security-policy").expect("csp").starts_with("sandbox;"));
    assert_eq!(resp.header("x-content-type-options"), Some("nosniff"));

    let bad = asyncio_runner(manager.get_html_resource(1, "../secret.txt"));
    assert_eq!(bad.status_code, 400);

    let unsupported = tmp_path.join("A").join("script.js");
    fs::write(&unsupported, "alert(1)").unwrap();
    let resp2 = asyncio_runner(manager.get_html_resource(1, "script.js"));
    assert_eq!(resp2.status_code, 400);
}

#[test]
fn test_get_book_content_html_forces_attachment() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir_all(tmp_path.join("A")).unwrap();
    let html_file = tmp_path.join("A").join("test.html");
    fs::write(&html_file, "<html><body>safe</body></html>").unwrap();
    let doc = make_doc("A/test.html", "html");
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(doc));

    // 응답 헤더는 라우트가 `file_response_meta`로 만든다(`BookContent::File`은 경로만 담는다).
    let path = match asyncio_runner(manager.get_book_content(1)) {
        BookContent::File(path) => path,
        other => panic!("FileResponse가 아님: {other:?}"),
    };
    assert!(path.is_file());
    let (media_type, headers) = BookManager::file_response_meta(&path);
    assert_eq!(media_type, "text/html");
    let content_disposition = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("content-disposition")).map(|(_, v)| v.as_str()).expect("content-disposition");
    assert!(content_disposition.starts_with("attachment;"));
}

// ── _build_html_resource_url 엣지 케이스 (lines 61, 64, 71, 73) ──────────────

mod test_build_html_resource_url {
    use super::*;

    #[test]
    fn test_empty_string_returns_none() {
        assert_eq!(BookManager::_build_html_resource_url("/base", ""), None);
    }

    #[test]
    fn test_whitespace_only_returns_none() {
        assert_eq!(BookManager::_build_html_resource_url("/base", "   "), None);
    }

    #[test]
    fn test_data_uri_returned_as_is() {
        let uri = "data:image/png;base64,abc123";
        assert_eq!(BookManager::_build_html_resource_url("/base", uri), Some(uri.to_string()));
    }

    #[test]
    fn test_blob_uri_returned_as_is() {
        let uri = "blob:http://example.com/uuid";
        assert_eq!(BookManager::_build_html_resource_url("/base", uri), Some(uri.to_string()));
    }

    #[test]
    fn test_external_http_url_returns_none() {
        assert_eq!(BookManager::_build_html_resource_url("/base", "http://external.com/img.png"), None);
    }

    #[test]
    fn test_protocol_relative_url_returns_none() {
        assert_eq!(BookManager::_build_html_resource_url("/base", "//cdn.example.com/img.png"), None);
    }
}

// ── _sanitize_html_for_viewer: external src 속성 제거 (line 116) ─────────────

#[test]
fn test_sanitize_html_external_src_attribute_removed() {
    // 외부 URL src 는 _build_html_resource_url 이 None 을 반환하므로 속성이 제거된다.
    let html = r#"<html><body><img src="http://external.com/img.png"></body></html>"#;
    let result = BookManager::_sanitize_html_for_viewer(html, "/base");
    let soup = scraper::Html::parse_document(&result);
    let sel = scraper::Selector::parse("img").unwrap();
    let img = soup.select(&sel).next();
    assert!(img.is_none() || img.unwrap().value().attr("src").is_none());
}

// ── get_book_preview: HTML read_text 예외 → 500 (lines 818-820) ─────────────

#[test]
fn test_get_book_preview_html_read_exception() {
    // HTML 파일 read_text 가 예외를 올리면 500 응답을 반환한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let html_file = tmp_path.join("A").join("book.html");
    fs::create_dir_all(html_file.parent().unwrap()).unwrap();
    fs::write(&html_file, "<html/>").unwrap();

    let doc = make_doc("A/book.html", "html");
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(doc));

    // 읽기 권한을 없애 PermissionError를 흉내 낸다.
    fs::set_permissions(&html_file, fs::Permissions::from_mode(0o000)).unwrap();
    let resp = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    fs::set_permissions(&html_file, fs::Permissions::from_mode(0o644)).unwrap();

    assert_eq!(resp.status_code, 500);
}

// ── get_html_resource 오류 경로 (lines 859, 862, 872-874, 876) ───────────────

mod test_get_html_resource {
    use super::*;

    #[test]
    fn test_missing_required_es_fields_returns_server_error() {
        let tmp = tmp_dir();
        let html = tmp.path().join("A/book.html");
        fs::create_dir_all(html.parent().unwrap()).unwrap();
        fs::write(&html, "<html/>").unwrap();
        fs::write(html.parent().unwrap().join("style.css"), "body{}").unwrap();

        let mut doc = make_doc("A/book.html", "html");
        doc.as_object_mut().unwrap().remove("category");
        let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
        let resp = asyncio_runner(manager.get_html_resource(1, "style.css"));

        assert_eq!(resp.status_code, 500);
    }

    #[test]
    fn test_book_not_found_returns_404() {
        // ES 에 책이 없으면 404 (line 859).
        let tmp = tmp_dir();
        let manager = make_manager(tmp.path(), &FakeEsManager::new());
        let resp = asyncio_runner(manager.get_html_resource(99, "image.jpg"));
        assert_eq!(resp.status_code, 404);
    }

    #[test]
    fn test_non_html_file_returns_400() {
        // HTML 이 아닌 파일(.epub)은 400 (line 862).
        let tmp = tmp_dir();
        let doc = make_doc("A/book.epub", "epub");
        let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
        let resp = asyncio_runner(manager.get_html_resource(1, "image.jpg"));
        assert_eq!(resp.status_code, 400);
    }

    #[test]
    fn test_path_traversal_via_symlink_returns_400() {
        // html_dir 밖을 가리키는 심볼릭 링크는 400 (line 872).
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let html_file = tmp_path.join("A").join("book.html");
        fs::create_dir_all(html_file.parent().unwrap()).unwrap();
        fs::write(&html_file, "<html/>").unwrap();

        // html_dir 밖 파일을 가리키는 심볼릭 링크 생성
        let outside = tmp_path.join("secret.jpg");
        fs::File::create(&outside).unwrap();
        let link = html_file.parent().unwrap().join("evil.jpg");
        symlink(&outside, &link).unwrap();

        let doc = make_doc("A/book.html", "html");
        let manager = make_manager(tmp_path, &FakeEsManager::with_doc(doc));
        let resp = asyncio_runner(manager.get_html_resource(1, "evil.jpg"));
        assert_eq!(resp.status_code, 400);
    }

    #[test]
    fn test_resource_file_not_found_returns_404() {
        // html_dir 내 존재하지 않는 리소스는 404 (line 876).
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let html_file = tmp_path.join("A").join("book.html");
        fs::create_dir_all(html_file.parent().unwrap()).unwrap();
        fs::write(&html_file, "<html/>").unwrap();

        let doc = make_doc("A/book.html", "html");
        let manager = make_manager(tmp_path, &FakeEsManager::with_doc(doc));
        let resp = asyncio_runner(manager.get_html_resource(1, "nonexistent.jpg"));
        assert_eq!(resp.status_code, 404);
    }

    #[test]
    fn test_is_relative_to_oserror_returns_400() {
        // is_relative_to 가 OSError 를 던지면 400 (lines 873-874).
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let html_file = tmp_path.join("A").join("book.html");
        fs::create_dir_all(html_file.parent().unwrap()).unwrap();
        fs::write(&html_file, "<html/>").unwrap();
        fs::File::create(html_file.parent().unwrap().join("image.jpg")).unwrap();

        // 가정한 seam: 경로 포함 관계 검사(is_relative_to)를 실패시키는 훅. 문서에 없는 seam이다(파일 머리말 참고).
        let doc = make_doc("A/book.html", "html");
        let es = FakeEsManager::with_doc(doc);
        let manager = BookManager::with_deps(deps(tmp_path, &es).with_path_is_relative_to(Arc::new(|_path, _other| Err(io::Error::other("path too long")))));
        let resp = asyncio_runner(manager.get_html_resource(1, "image.jpg"));
        assert_eq!(resp.status_code, 400);
    }
}

#[test]
fn test_book_preview_rejects_missing_required_es_fields() {
    let tmp = tmp_dir();
    let html = tmp.path().join("A/book.html");
    fs::create_dir_all(html.parent().unwrap()).unwrap();
    fs::write(&html, "<html><body>preview</body></html>").unwrap();

    let mut doc = make_doc("A/book.html", "html");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
    let resp = asyncio_runner(manager.get_book_preview(1, Some(5), Some(10)));

    assert_eq!(resp.status_code, 500);
}

#[test]
fn test_pdf_pages_rejects_missing_required_es_fields() {
    let tmp = tmp_dir();
    let mut doc = make_doc("A/book.pdf", "pdf");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));
    let resp = asyncio_runner(manager.get_pdf_pages(1, 1, 1));

    assert_eq!(resp.status_code, 500);
}

#[test]
fn test_get_cover_rejects_missing_required_es_fields() {
    let tmp = tmp_dir();
    let mut doc = make_doc("A/book.txt", "txt");
    doc.as_object_mut().unwrap().remove("category");
    let manager = make_manager(tmp.path(), &FakeEsManager::with_doc(doc));

    let resp = asyncio_runner(manager.get_cover(1, None));

    assert_eq!(resp.status_code, 500);
}

// ── update_book: 책이 없을 때 Error 반환 (line 978) ─────────────────────────

#[test]
fn test_update_book_no_such_book_returns_error() {
    // ES 에서 책을 찾지 못하면 ('Error', '…no such a book') 을 반환한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let new_path = tmp_path.join("A").join("new.txt");

    let (status, msg) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 999, "A", "Title", "Author", &new_path, "txt", false)));
    assert_eq!(status, "Error");
    assert!(msg.unwrap_or_default().contains("no such a book"));
}

// --- M1: lxml XXE 방지 (CWE-611) ---

#[test]
fn test_safe_xml_parser_blocks_external_entity() {
    // _safe_xml_parser 는 외부 엔티티(file://)를 해석하지 않아야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let secret = tmp_path.join("canary.txt");
    fs::write(&secret, "XXE_CANARY_VALUE").unwrap();
    let payload = format!(r#"<?xml version="1.0"?><!DOCTYPE r [<!ENTITY xxe SYSTEM "file://{}">]><r>&xxe;</r>"#, secret.display());

    // recover=True (book_manager의 4개 호출부 모드): 엔티티 미해석 → 파일 내용 미노출
    let tree = _safe_xml_parser(true).fromstring(payload.as_bytes()).expect("recover 모드는 파싱을 포기하지 않는다");
    assert!(!tree.tostring().contains("XXE_CANARY_VALUE"));

    // 정상 XML 은 그대로 파싱되어야 한다 (기능 회귀 방지)
    let ok = _safe_xml_parser(false).fromstring(b"<r><a>hi</a></r>").unwrap();
    assert_eq!(ok.find("a").unwrap().text().as_deref(), Some("hi"));
}

// --- M2: 카테고리 조회 결과 상한 (CWE-770) ---

#[test]
fn test_get_books_in_category_uses_bounded_result_count() {
    // get_books_in_category 는 sys.maxsize 가 아니라 MAX_CATEGORY_RESULT_COUNT 로 조회해야 한다.
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let captured: Arc<Mutex<Option<i64>>> = Arc::new(Mutex::new(None));
    let cap = captured.clone();
    es.on_search_by_category(move |_category, max_result_count| {
        *cap.lock().unwrap() = Some(max_result_count);
        Vec::new()
    });
    let manager = make_manager(tmp.path(), &es);

    let _ = asyncio_runner(manager.get_books_in_category("A"));

    let got = captured.lock().unwrap().expect("search_by_category가 호출되지 않음");
    assert_eq!(got, MAX_CATEGORY_RESULT_COUNT);
    assert_ne!(got, isize::MAX as i64);
    const _: () = assert!(MAX_CATEGORY_RESULT_COUNT <= 10000); // ES max_result_window 이내 → scroll 미사용 경로 유지
}

#[test]
fn test_get_books_in_category_warns_on_truncation() {
    // 결과가 MAX_CATEGORY_RESULT_COUNT 에 도달하면 잘림 경고를 남긴다 (book_manager.py:372).
    let tmp = tmp_dir();
    // 가정한 seam: 상한 값 설정자(문서에 없는 seam, 파일 머리말 참고)와 log 캡처.
    let es = FakeEsManager::new();
    es.set_category_docs(vec![(1, make_doc("A/x.txt", ".txt"), 1.0)]);
    let manager = BookManager::with_deps(deps(tmp.path(), &es).with_max_category_result_count(1));
    let logs = LogCapture::start(log::Level::Warn);

    let books = asyncio_runner(manager.get_books_in_category("A")).unwrap();
    let records = logs.messages();

    assert_eq!(books.len(), 1);
    assert!(records.iter().any(|r| r.contains("상한") && r.contains("get_books_in_category")), "records={records:?}");
}

#[test]
fn test_get_category_mismatch_details_reads_all_pages() {
    // 상한 크기의 첫 페이지에서 멈추지 않고 다음 페이지까지 비교한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let first = tmp_path.join("A").join("first.txt");
    let second = tmp_path.join("A").join("second.txt");
    fs::create_dir_all(first.parent().unwrap()).unwrap();
    fs::write(&first, "x").unwrap();
    fs::write(&second, "y").unwrap();
    es.set_category_docs(vec![
        (fs::metadata(&first).unwrap().ino() as i64, make_doc("A/first.txt", ".txt"), 1.0),
        (fs::metadata(&second).unwrap().ino() as i64, make_doc("A/second.txt", ".txt"), 1.0),
    ]);
    // 상한을 1로 줄인다(문서에 없는 seam, 파일 머리말 참고).
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_max_category_result_count(1));

    let details = manager.get_category_mismatch_details("A");

    assert_eq!(details["es_only"], json!([]));
    assert_eq!(details["fs_only"], json!([]));
}

// ── Additional edge case tests to reach >99% coverage on backend/book_manager.py ──

#[test]
fn test_empty_epub_spine_href_is_rejected() {
    let tmp = tmp_dir();
    let epub_path = tmp.path().join("empty_path.epub");
    write_zip(
        &epub_path,
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#),
            ("content.opf", br#"<?xml version="1.0"?><package version="2.0" xmlns="http://www.idpf.org/2007/opf" unique-identifier="BookId"><manifest><item id="item1" href="" media-type="text/html"/></manifest><spine toc="ncx"><itemref idref="item1"/></spine></package>"#),
        ],
    );
    let (valid, _msg) = verdict(BookManager::_validate_preview_epub(&epub_path));
    assert!(!valid);
}

#[test]
fn test_page_count_cache_eviction() {
    // Line 386: page_count_cache eviction when max exceeded
    // 캐시를 인스턴스가 갖는다(3.11). 상한과 캐시 접근자는 문서에 없는 seam이다(파일 머리말 참고).
    let tmp = tmp_dir();
    let manager = BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_page_count_cache_max(2));
    let cache = manager.page_count_cache();
    cache.clear();
    cache.insert(("k1".to_string(), 1), 10);
    cache.insert(("k2".to_string(), 1), 20);
    // 세 번째 항목을 넣어 제거 루프를 일으킨다 (386)
    let page_key = ("k3".to_string(), 1);
    cache.insert(page_key.clone(), 30);
    cache.touch(&page_key);
    cache.evict();
    assert_eq!(cache.len(), 2);
    assert!(!cache.contains(&("k1".to_string(), 1)));
}

#[test]
fn test_get_latest_books_delegation() {
    // Lines 538, 540-541: get_latest_books
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    es.set_latest_docs(vec![(10, make_doc("A/test.txt", ".txt"), 1.0)], 1);
    let manager = make_manager(tmp.path(), &es);
    let BookPage { books, total, .. } = asyncio_runner(manager.get_latest_books(5, Some(vec!["hidden".to_string()]))).unwrap();
    assert_eq!(total, 1);
    assert_eq!(books.len(), 1);
    assert_eq!(books[0].book_id, 10);
}

#[test]
fn test_backfill_created_time_thread_states() {
    // Lines 549, 561-563: thread is alive and start exception
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let _env = EnvGuard::single("TM_CREATED_TIME_BACKFILL", "1");

    // 549: thread is already alive
    let mock_thread = Arc::new(FakeBackfillThread::alive());
    manager.set_created_time_backfill_thread(Some(mock_thread.clone()));
    manager._backfill_created_time_if_enabled();
    assert_eq!(mock_thread.start_calls(), 0, "start가 호출되지 않아야 한다");

    // 561-563: thread.start() raises exception
    // 가정한 seam: 스레드 시작이 실패하는 실행기(TODO 주석 참고).
    let manager = BookManager::with_deps(deps(tmp.path(), &es).with_task_runner(Arc::new(FailingStartRunner { message: "thread pool exhausted" })));
    manager.set_created_time_backfill_thread(None);
    manager._backfill_created_time_if_enabled();
    assert!(manager.created_time_backfill_thread().is_none());
}

#[test]
fn test_normalize_stored_file_path_all_branches() {
    // Lines 1217, 1223-1224, 1228, 1230
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());

    // 1217: empty
    assert_eq!(manager._normalize_stored_file_path(""), "");

    // 1223-1224: absolute path outside prefix
    let res = manager._normalize_stored_file_path("/some/other/non_relative_root/file.txt");
    assert_eq!(res, "some/other/non_relative_root/file.txt");

    // 1228: normalized == "."
    assert_eq!(manager._normalize_stored_file_path("."), "");

    // 1230: normalized starts with ./
    assert_eq!(manager._normalize_stored_file_path("./A/file.txt"), "A/file.txt");
}

#[test]
fn test_is_safe_category_name_oserror() {
    // Lines 1244-1245: OSError in _is_safe_category_name
    let tmp = tmp_dir();
    // 경로 정규화(resolve = canonicalize) 실패 훅. 이름에 "bad_oserror"가 든 경로만 실패한다.
    let faulty = FaultyFs::new();
    faulty.set_canonicalize(|p| p.to_string_lossy().contains("bad_oserror").then(|| other_err("permission denied")));
    let manager = BookManager::with_deps(deps(tmp.path(), &FakeEsManager::new()).with_fs(faulty));
    assert!(!manager._is_safe_category_name("bad_oserror"));
}

#[test]
fn test_is_indexable_file_path_hidden() {
    // Line 1249: dot file
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    assert!(!manager._is_indexable_file_path(Path::new(".hidden.epub")));
}

#[test]
fn test_classification_haystack_relative_to_error() {
    // Lines 1275-1276: ValueError in _classification_haystack
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let outside_file = Path::new("/tmp/outside/nested/book.epub");
    let haystack = manager._classification_haystack(outside_file);
    assert!(haystack.contains("nested"));
    assert!(haystack.contains("book"));
}

#[test]
fn test_iter_category_indexable_files_branches_shadowed() {
    // 원본에는 같은 이름의 함수가 뒤에 하나 더 있어 Python에서는 이 정의가 가려진다(pytest는 뒤의 것만 실행한다).
    // Rust는 한 모듈에 같은 이름을 둘 수 없어 앞쪽 정의에 _shadowed를 붙였다.
    // Lines 1350-1353, 1359-1360
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let faulty = FaultyFs::new();
    faulty.set_canonicalize(|p| p.to_string_lossy().contains("valid.txt").then(|| other_err("resolve failure")));
    let manager = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_fs(faulty));
    let cat_dir = tmp_path.join("A");
    fs::create_dir_all(&cat_dir).unwrap();
    fs::create_dir_all(cat_dir.join(".hidden_dir")).unwrap();
    fs::write(cat_dir.join(".hidden_dir").join("ignored.txt"), "x").unwrap();
    let normal_file = cat_dir.join("valid.txt");
    fs::write(&normal_file, "y").unwrap();

    let files = manager._iter_category_indexable_files("A", true);
    assert_eq!(files.len(), 0); // .hidden_dir ignored (1353), valid.txt resolve OSError (1359-1360)
}

#[test]
fn test_rename_category_counts_subcategory_documents() {
    // 문서가 하위 카테고리에만 있어도 디렉토리는 존재하므로 이름을 바꿀 수 있어야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir(tmp_path.join("A")).unwrap();
    es.set_counts(&[("A/sub", 3)]);

    let result = asyncio_runner(manager.rename_category("A", "B")).unwrap();

    assert_eq!(result["fs_renamed"], json!(true));
    assert!(tmp_path.join("B").is_dir());
}

#[test]
fn test_rename_category_blocks_target_with_subcategory_documents() {
    // 대상 카테고리가 하위에만 문서를 가져도 충돌이다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir(tmp_path.join("A")).unwrap();
    es.set_counts(&[("A", 2), ("B/sub", 5)]);

    let (result, err) = outcome(asyncio_runner(manager.rename_category("A", "B")));

    assert_eq!(result, json!({}));
    assert!(err.expect("err").contains("이미"));
    assert!(tmp_path.join("A").is_dir());
}

#[test]
fn test_move_classified_file_keeps_both_when_content_differs() {
    // 이름만 같고 내용이 다르면 둘 다 남기고 새 파일에 괄호 번호를 붙인다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let src = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "새 파일 내용").unwrap();
    let existing = tmp_path.join("B").join("book.txt");
    fs::create_dir_all(existing.parent().unwrap()).unwrap();
    fs::write(&existing, "기존 파일 내용은 다르다").unwrap();

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&src, "B", "book", MoveOpts { clean_existing: true, ..Default::default() })));

    assert!(err.is_none());
    assert_eq!(result.unwrap()["to"], "B/book (1).txt");
    assert_eq!(fs::read_to_string(&existing).unwrap(), "기존 파일 내용은 다르다");
    assert_eq!(fs::read_to_string(tmp_path.join("B").join("book (1).txt")).unwrap(), "새 파일 내용");
    assert!(!src.exists());
}

#[test]
fn test_move_classified_file_numbers_up_when_bracket_name_taken() {
    // '(1)'도 이미 있으면 '(2)'로 넘어간다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let src = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "세 번째").unwrap();
    fs::create_dir(tmp_path.join("B")).unwrap();
    fs::write(tmp_path.join("B").join("book.txt"), "첫 번째").unwrap();
    fs::write(tmp_path.join("B").join("book (1).txt"), "두 번째").unwrap();

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&src, "B", "book", MoveOpts::default())));

    assert!(err.is_none());
    assert_eq!(result.unwrap()["to"], "B/book (2).txt");
    assert_eq!(fs::read_to_string(tmp_path.join("B").join("book (2).txt")).unwrap(), "세 번째");
}

#[test]
fn test_move_classified_file_same_size_different_content_keeps_both() {
    // 크기가 같아도 내용이 다르면 중복이 아니다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let src = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src.parent().unwrap()).unwrap();
    fs::write(&src, "AAAA").unwrap();
    fs::create_dir(tmp_path.join("B")).unwrap();
    fs::write(tmp_path.join("B").join("book.txt"), "BBBB").unwrap();

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&src, "B", "book", MoveOpts { clean_existing: true, ..Default::default() })));

    assert!(err.is_none());
    assert_eq!(result.unwrap()["to"], "B/book (1).txt");
    assert_eq!(fs::read_to_string(tmp_path.join("B").join("book.txt")).unwrap(), "BBBB");
}
#[test]
fn test_move_classified_file_edge_cases() {
    // Lines 1379-1380, 1385-1387, 1393-1394, 1398, 1420-1421, 1440-1441, 1450, 1473, 1490-1491, 1494-1499, 1503
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());

    // 1. 1379-1380: old_rel_path ValueError
    let outside_file = Path::new("/tmp/outside/file.txt");
    let (res, err) = split(asyncio_runner(manager._move_classified_file(outside_file, "B", "book", MoveOpts::default())));
    assert!(res.is_none());
    assert_eq!(err.as_deref(), Some("잘못된 파일 경로입니다"));

    // 2. 1385-1387: target_path resolve error
    let f1 = tmp_path.join("A").join("f1.txt");
    fs::create_dir_all(f1.parent().unwrap()).unwrap();
    fs::write(&f1, "f1").unwrap();
    // 단계마다 seam이 달라 `with_fs`로 매니저를 새로 만든다(구 전역 가드의 생명주기).
    let faulty_resolve = FaultyFs::new();
    faulty_resolve.set_canonicalize(|p| p.to_string_lossy().contains("bad_target").then(|| other_err("target path resolve error")));
    let manager_resolve = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_fs(faulty_resolve));
    let (res2, err2) = split(asyncio_runner(manager_resolve._move_classified_file(&f1, "bad_target", "book", MoveOpts::default())));
    assert!(res2.is_none());
    assert_eq!(err2.as_deref(), Some("잘못된 대상 경로입니다"));

    // 3. 1393-1394 & 1398: target exists, samefile OSError, clean_existing and dry_run
    let target_f = tmp_path.join("B").join("f1.txt");
    fs::create_dir_all(target_f.parent().unwrap()).unwrap();
    fs::write(&target_f, "f1").unwrap(); // 내용까지 같아야 중복 정리 대상이다

    let faulty_dup = FaultyFs::new();
    faulty_dup.set_same_file(|_a, _b| Some(other_err("samefile failed")));
    let manager_dup = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_fs(faulty_dup.clone()));
    let (res3, err3) = split(asyncio_runner(manager_dup._move_classified_file(&f1, "B", "book", MoveOpts { dry_run: true, clean_existing: true, ..Default::default() })));
    let res3 = res3.expect("res3");
    assert_eq!(res3["action"], "duplicate_clean");
    assert_eq!(res3["status"], "dry_run");
    assert!(err3.is_none());

    // 4. 1420-1421: clean_existing exception
    // samefile 실패는 그대로 두고 unlink(remove_file)만 더 실패시킨다.
    faulty_dup.set_remove_file(|_p| Some(other_err("delete duplicate failed")));
    let (res4, err4) = split(asyncio_runner(manager_dup._move_classified_file(&f1, "B", "book", MoveOpts { dry_run: false, clean_existing: true, ..Default::default() })));
    assert!(res4.is_none());
    assert!(err4.expect("err4").contains("중복 파일 정리 실패"));

    // 5. 1440-1441 & 1450: shutil.move OSError, search_by_id exception
    fs::remove_file(&target_f).unwrap();

    let err_es = FakeEsManager::new();
    err_es.fail("search_by_id", "es lookup error");
    // `shutil.move`는 `FsOps::rename`으로 합쳤다(파일 머리말 참고).
    let faulty_move = FaultyFs::new();
    faulty_move.set_rename(|_src, _dst| Some(other_err("cross-device link failed")));
    let manager_err = BookManager::with_deps(deps(tmp_path, &err_es).with_fs(faulty_move));
    let (res5, err5) = split(asyncio_runner(manager_err._move_classified_file(&f1, "B", "book", MoveOpts::default())));
    assert!(res5.is_none());
    assert!(err5.expect("err5").contains("파일 이동 실패"));

    // 6. 1473, 1490-1491, 1494-1499, 1503: add_book failed, file rollback failed, restore old doc failed
    let f2 = tmp_path.join("A").join("f2.txt");
    fs::write(&f2, "f2").unwrap();

    let add_err_es = FakeEsManager::new();
    add_err_es.set_doc(Some(json!({"file_path": "A/f2.txt", "title": "f2"})));
    // 앞으로 가는 이동은 성공시키고(첫 호출) 롤백 이동(두 번째 호출)만 실패시킨다.
    let faulty_rollback = FaultyFs::new();
    faulty_rollback.set_rename(fail_on_nth_call(2, "rollback move error"));
    let manager_add_err = BookManager::with_deps(deps(tmp_path, &add_err_es).with_book_adder(book_adder(|| Err(AddBookError::Rejected("ES add error".to_string())))).with_fs(faulty_rollback));

    let (res6, err6) = split(asyncio_runner(manager_add_err._move_classified_file(&f2, "B", "book", MoveOpts::default())));
    assert!(res6.is_none());
    let err6 = err6.expect("err6");
    assert!(err6.contains("ES add error"));
    assert!(err6.contains("파일 롤백 실패"));
}

#[test]
fn test_move_classified_file_is_a_noop_when_already_in_target_category() {
    // 지금 있는 카테고리를 목적지로 고르면 아무것도 하지 않고 성공으로 끝낸다.
    //
    // 관리자가 표의 드롭다운에서 현재 카테고리를 그대로 고르면 원본과 대상 경로가 같아진다.
    // 이 분기가 없으면 os.rename 이 같은 경로에 대해 조용히 성공한 뒤 ES 문서를 지웠다가
    // 같은 내용으로 다시 넣는다. 파일도 문서도 그대로여야 하고, ES 를 건드리지 않아야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    let category_dir = tmp_path.join("2_science");
    fs::create_dir_all(&category_dir).unwrap();
    let book = category_dir.join("제자리 도서.txt");
    fs::write(&book, "내용").unwrap();

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&book, "2_science", "book", MoveOpts::default())));

    assert!(err.is_none());
    let result = result.expect("result");
    assert_eq!(result["status"], "moved");
    assert_eq!(result["action"], "noop");
    assert_eq!(result["from"], result["to"]);
    assert_eq!(result["to"], "2_science/제자리 도서.txt");
    assert!(book.exists());
    let deleted_paths: Vec<String> = es.deleted_file_paths().into_iter().flat_map(|(paths, _exclude)| paths).collect();
    assert!(deleted_paths.is_empty(), "제자리인데 ES 문서를 지웠다");
}

#[test]
fn test_move_classified_file_does_not_orphan_a_hardlink() {
    // 대상에 같은 실체(하드링크)가 이미 있으면 옮기지 않고 거부한다.
    //
    // 하드링크는 경로가 달라도 samefile 이 참이다. 이동으로 처리하면 os.rename 이 원본
    // 링크를 지우지 않은 채 성공해(실측), 디스크에는 파일이 남았는데 ES 문서만 사라진
    // 고아가 만들어진다. 그 고아는 '이상 항목'으로만 드러나고 왜 생겼는지 알 수 없다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let source_dir = tmp_path.join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("링크 도서.txt");
    fs::write(&source, "내용").unwrap();
    let target_dir = tmp_path.join("2_science");
    fs::create_dir(&target_dir).unwrap();
    fs::hard_link(&source, target_dir.join(source.file_name().unwrap())).unwrap();

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&source, "2_science", "book", MoveOpts { clean_existing: false, ..Default::default() })));

    assert!(result.is_none());
    assert!(err.expect("err").contains("대상 경로에 파일이 이미 존재합니다"));
    assert!(source.exists(), "원본 링크가 남았는데 옮긴 것으로 처리했다");
}

#[test]
fn test_move_classified_file_rejects_existing_duplicate_without_clean() {
    // 내용까지 같은 파일이 이미 있고 clean_existing이 꺼져 있으면 옮기지 않고 실패로 남긴다.
    //
    // apply_category_changes가 clean_existing=False로 이 메서드를 부를 때 거치는 경로다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let source_dir = tmp_path.join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let conflict = source_dir.join("중복 도서.txt");
    fs::write(&conflict, "같은 내용").unwrap();
    let target_dir = tmp_path.join("2_science");
    fs::create_dir(&target_dir).unwrap();
    fs::write(target_dir.join(conflict.file_name().unwrap()), "같은 내용").unwrap();

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&conflict, "2_science", "book", MoveOpts { clean_existing: false, ..Default::default() })));

    assert!(result.is_none());
    assert!(err.expect("err").contains("대상 경로에 파일이 이미 존재합니다"));
    assert!(conflict.exists());
}

#[test]
fn test_move_classified_file_preserves_existing_es_metadata() {
    // 관리자가 ES에서 고친 제목/저자/요약은 재색인 때도 유지된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let source_dir = tmp_path.join("0_inbox");
    fs::create_dir_all(&source_dir).unwrap();
    let source_file = source_dir.join("수정 과학.txt");
    fs::write(&source_file, "hello").unwrap();
    let mut old_doc = make_doc("0_inbox/수정 과학.txt", "txt");
    old_doc["category"] = json!("0_inbox");
    old_doc["title"] = json!("관리자가 수정한 제목");
    old_doc["author"] = json!("관리자가 수정한 저자");
    old_doc["summary"] = json!("관리자가 유지하려는 요약");
    let es = FakeEsManager::with_doc(old_doc);
    // Loader::read_file 교체 훅. 호출되면 테스트를 실패시킨다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_file_reader(file_reader(|_path| panic!("existing ES metadata should be reused"))));

    let (result, err) = split(asyncio_runner(manager._move_classified_file(&source_file, "2_science", "book", MoveOpts::default())));

    assert!(err.is_none());
    assert_eq!(result.expect("result")["status"], "moved");
    let indexed_doc = es.inserted()[0].values().next().unwrap().clone();
    assert_eq!(indexed_doc["category"], "2_science");
    assert_eq!(indexed_doc["file_path"], "2_science/수정 과학.txt");
    assert_eq!(indexed_doc["title"], "관리자가 수정한 제목");
    assert_eq!(indexed_doc["author"], "관리자가 수정한 저자");
    assert_eq!(indexed_doc["summary"], "관리자가 유지하려는 요약");
}

#[test]
fn test_mismatch_and_reload_more_edge_cases() {
    // Lines 1780-1781, 1905-1906, 1909-1912, 1921, 1935, 1951-1952, 1964-1965, 1989, 1995, 2012, 2029-2030, 2042-2043, 2063, 2065, 2087-2088
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());

    // 1780-1781: unsafe category in get_category_mismatch_details
    let details = manager.get_category_mismatch_details("../unsafe");
    assert_eq!(details, json!({"es_only": [], "fs_only": [], "duplicates": [], "fs_count": 0}));

    // 1905-1906, 1909-1912, 1921, 1935: _bulk_index_files edge cases

    // non-existent file (1905-1906)
    let (_idx1, fail1) = asyncio_runner(manager._bulk_index_files(&["A/non_existent.txt"], true));
    assert_eq!(fail1.len(), 1);
    assert!(fail1[0]["error"].as_str().unwrap().contains("파일을 찾을 수 없습니다"));

    // ParserTimeout (1909-1912)
    let f_timeout = tmp_path.join("A").join("timeout.txt");
    fs::create_dir_all(f_timeout.parent().unwrap()).unwrap();
    fs::write(&f_timeout, "timeout").unwrap();

    // 단계마다 `with_file_reader`가 달라 매니저를 새로 만든다(구 전역 가드의 생명주기).
    let manager_timeout =
        BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_file_reader(file_reader(|_path| Err(LoaderReadError::Timeout(ParserTimeout { message: "read timed out".to_string() })))));
    let (_idx2, fail2) = asyncio_runner(manager_timeout._bulk_index_files(&["A/timeout.txt"], true));
    assert_eq!(fail2.len(), 1);
    assert!(fail2[0]["error"].as_str().unwrap().contains("파싱 시간 초과"));

    // merged is empty (1921)
    let manager_empty = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_file_reader(file_reader(|_path| Ok(json!({})))));
    let (idx3, _fail3) = asyncio_runner(manager_empty._bulk_index_files(&["A/timeout.txt"], true));
    assert!(idx3.is_empty());

    // ES insert failure (1935)
    let f_ok = tmp_path.join("A").join("ok.txt");
    fs::write(&f_ok, "ok").unwrap();

    let fail_insert_es = FakeEsManager::new();
    fail_insert_es.with(|s| s.insert_returns_empty = true); // 999 not in indexed_ids
    let manager_fail_ins = BookManager::with_deps(deps(tmp_path, &fail_insert_es).with_file_reader(file_reader(|_path| Ok(json!({"999": make_doc("A/ok.txt", ".txt")})))));
    let (_idx4, fail4) = asyncio_runner(manager_fail_ins._bulk_index_files(&["A/ok.txt"], true));
    assert_eq!(fail4.len(), 1);
    assert!(fail4[0]["error"].as_str().unwrap().contains("ES 적재 실패"));

    // 1951-1952, 1964-1965, 1989, 1995, 2012: _reload_category_mismatch_details branches
    let bad_details = json!({
        "fs_only": [{"file_path": ""}], // 1951-1952: empty file_path
        "duplicates": [
            {"file_path": "", "file_exists": true, "docs": []}, // 1964-1965: empty file_path
            {"file_path": "A/missing_idx.txt", "file_exists": true, "docs": [{"book_id": "not_an_int"}]}, // 1989: new_book_id is None, 1995: not int
        ],
        "es_only": [{"book_id": 888}],
    });

    let partial_del_es = FakeEsManager::new();
    partial_del_es.with(|s| s.delete_by_ids_returns = Some(0)); // deleted < len(ids_to_delete) -> 2012
    let manager_partial_del = make_manager(tmp_path, &partial_del_es);
    let res_bad = asyncio_runner(manager_partial_del._reload_category_mismatch_details("A", &bad_details));
    let failures = res_bad["failures"].as_array().unwrap();
    assert!(failures.iter().any(|f| f.get("error").and_then(Value::as_str).unwrap_or("").contains("file_path가 비어있습니다")));
    assert!(failures.iter().any(|f| f.get("error").and_then(Value::as_str).unwrap_or("").contains("삭제됨")));

    // 2029-2030, 2042-2043: reload_category_mismatches unsafe category and refresh exception
    let refresh_err_es = FakeEsManager::new();
    refresh_err_es.fail("refresh", "refresh error");
    // 가정한 seam: 인스턴스 메서드 바꿔치기(문서에 없는 seam, 파일 머리말 참고).
    let manager_ref_err = BookManager::with_deps(deps(tmp_path, &refresh_err_es).with_get_category_mismatches_override(Arc::new(|_force_refresh| json!({"mismatches": [{"category": "../unsafe"}]}))));
    let (res_mismatches, _) = outcome(asyncio_runner(manager_ref_err.reload_category_mismatches(None)));
    let failures = res_mismatches["failures"].as_array().unwrap();
    assert!(failures.iter().any(|f| f.to_string().contains("잘못된 카테고리 경로입니다")));
    assert!(failures.iter().any(|f| f.to_string().contains("ES refresh 실패")));

    // 2063, 2065, 2087-2088: reload_category_mismatch_files validation and refresh exception
    let (_, err_empty) = outcome(asyncio_runner(manager.reload_category_mismatch_files("", None)));
    assert_eq!(err_empty.as_deref(), Some("카테고리 이름이 비어있습니다"));
    let (_, err_unsafe) = outcome(asyncio_runner(manager.reload_category_mismatch_files("../unsafe", None)));
    assert_eq!(err_unsafe.as_deref(), Some("잘못된 카테고리 경로입니다"));

    let manager_ref_details =
        BookManager::with_deps(deps(tmp_path, &refresh_err_es).with_get_category_mismatch_details_override(Arc::new(|_cat| json!({"fs_only": [], "es_only": [], "duplicates": []}))));
    let (res_single_reload, _) = outcome(asyncio_runner(manager_ref_details.reload_category_mismatch_files("A", None)));
    assert!(res_single_reload["failures"].as_array().unwrap().iter().any(|f| f.to_string().contains("ES refresh 실패")));
}

#[test]
fn test_get_pdf_pages_oserror() {
    // Lines 2209-2210: OSError in get_pdf_pages
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let pdf_file = tmp_path.join("A").join("sample.pdf");
    fs::create_dir_all(pdf_file.parent().unwrap()).unwrap();
    fs::write(&pdf_file, b"%PDF-1.4 dummy").unwrap();

    let pdf_es = FakeEsManager::new();
    pdf_es.set_doc(Some(make_doc("A/sample.pdf", ".pdf")));
    // 쪽수 조회가 ENOSPC로 실패하게 한다(`PdfPageOps` 대역, TODO 주석 참고).
    let manager = BookManager::with_deps(deps(tmp_path, &pdf_es).with_pdf_pages(Arc::new(NoSpacePdfPages)));
    let resp = asyncio_runner(manager.get_pdf_pages(777, 1, 5));
    assert_eq!(resp.status_code, 503);
    assert!(resp.body_text().contains("Storage access error"));
}

#[test]
fn test_update_book_reindex_and_rollback_branches() {
    // Lines 1169, 1182-1184, 1188-1190, 1192-1193, 1204-1206, 1208-1211
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let src_file = tmp_path.join("A").join("book.txt");
    fs::create_dir_all(src_file.parent().unwrap()).unwrap();
    fs::write(&src_file, "content").unwrap();

    let old_doc = make_doc("A/book.txt", ".txt");

    let update_es = FakeEsManager::new();
    update_es.set_doc(Some(old_doc));
    // 단계마다 seam이 달라 `with_*`로 매니저를 새로 만든다(구 전역 가드와 `override_add_book`의 생명주기).
    let empty_reader = || file_reader(|_path| Ok(json!({})));
    let crash_reader = || file_reader(|_path| Err(LoaderReadError::Other("read_file crashed".to_string())));

    // 1. 1169 & 1182-1184: Loader.read_file returns empty -> reindex_error, rename raises OSError on rollback (second call)
    let faulty_rename = FaultyFs::new();
    faulty_rename.set_rename(fail_on_nth_call(2, "rename rollback failed"));
    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_fs(faulty_rename));

    let dst = tmp_path.join("B").join("book.txt");
    let (st1, msg1) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st1, "Error");
    assert!(msg1.expect("msg1").contains("파일 롤백도 실패"));

    // 2. 1188-1190: rollback success, restore old doc raises exception
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager =
        BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore old doc exception".to_string())))));

    let (st2, msg2) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st2, "Error");
    assert!(msg2.expect("msg2").contains("기존 ES 문서 복구 실패"));

    // 3. 1192-1193: restore old doc returns error
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(empty_reader()).with_book_adder(book_adder(|| Err(AddBookError::Rejected("restore error message".to_string())))));

    let (st3, msg3) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st3, "Error");
    assert!(msg3.expect("msg3").contains("restore error message"));

    // 4. 1204-1206: exception during reindex, rollback succeeds, restore old doc raises exception
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager =
        BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore old doc exception".to_string())))));

    let (st4, msg4) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st4, "Error");
    assert!(msg4.expect("msg4").contains("기존 ES 문서 복구 실패"));

    // 5. 1208-1210: exception during reindex, rollback succeeds, restore old doc returns error
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Err(AddBookError::Rejected("restore error message".to_string())))));

    let (st5, msg5) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st5, "Error");
    assert!(msg5.expect("msg5").contains("restore error message"));

    // 6. 1211: exception during reindex, rollback succeeds, restore old doc succeeds!
    if dst.exists() {
        fs::remove_file(&dst).unwrap();
    }
    fs::write(&src_file, "content").unwrap();

    let manager = BookManager::with_deps(deps(tmp_path, &update_es).with_file_reader(crash_reader()).with_book_adder(book_adder(|| Ok(1))));
    let (st6, msg6) = as_status(asyncio_runner(BookManagerApi::update_book(&manager, 1, "B", "T", "A", &dst, "txt", false)));
    assert_eq!(st6, "Error");
    assert!(msg6.expect("msg6").contains("기존 ES 문서 복구 완료"));
}

#[test]
fn test_encode_decode_category_cursor() {
    // Lines 55, 60-64
    let cur = encode_category_cursor(&json!([1, "val"]));
    assert_eq!(decode_category_cursor(&cur), Some(json!([1, "val"])));
    assert_eq!(decode_category_cursor("invalid_base64!!!"), None);
    let not_list = base64::engine::general_purpose::URL_SAFE.encode(br#"{"a": 1}"#);
    assert_eq!(decode_category_cursor(&not_list), None);
}

#[test]
fn test_pdf_reader_cache_eviction_and_branches() {
    // Lines 333-334, 375-376, 386, 389, 395
    // 캐시를 인스턴스가 갖는다(3.11). 한도는 `with_pdf_cache_limits`, 쪽수 캐시 상한은 문서에 없는 seam이다(파일 머리말 참고).
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let pdf_path = tmp.path().join("test.pdf");
    fs::write(&pdf_path, blank_pdf_bytes_sized(1, 100, 100)).unwrap();
    let manager = make_manager(tmp.path(), &es);

    // 1. First get: cache miss
    let (reader1, pages1) = manager._get_cached_pdf_reader(&pdf_path);
    assert_eq!(pages1, 1);

    // 2. Lines 375-376: Cache hit with matching mtime
    let (reader2, pages2) = manager._get_cached_pdf_reader(&pdf_path);
    assert!(Arc::ptr_eq(&reader2, &reader1));
    assert_eq!(pages2, 1);

    // 3. Line 389: Cache item exists, but mtime changed -> del cache[key]
    let new_mtime = fs::metadata(&pdf_path).unwrap().modified().unwrap() + Duration::from_secs(5);
    set_mtime(&pdf_path, new_mtime);
    let (_reader3, pages3) = manager._get_cached_pdf_reader(&pdf_path);
    assert_eq!(pages3, 1);

    // 4. Line 386: page_cache exceeds PAGE_COUNT_CACHE_MAX
    {
        let manager = BookManager::with_deps(deps(tmp.path(), &es).with_page_count_cache_max(1));
        manager.pdf_cache().clear();
        manager.page_count_cache().insert(("dummy1".to_string(), 0), 1);
        manager.page_count_cache().insert(("dummy2".to_string(), 0), 2);
        manager._get_cached_pdf_reader(&pdf_path);
        assert!(manager.page_count_cache().len() <= 2);
    }

    // 5. Lines 333-334: _evict_pdf_readers eviction loop
    {
        let limits = PdfCacheLimits { max_file_bytes: None, max_bytes: Some(10), max_entries: Some(1) };
        let manager = BookManager::with_deps(deps(tmp.path(), &es).with_pdf_cache_limits(limits));
        manager.pdf_cache().insert_raw("k1", 0, 1, 100);
        manager.pdf_cache().insert_raw("k2", 0, 1, 100);
        manager.pdf_cache().evict();
        assert!(manager.pdf_cache().len() <= 1);
    }

    // 6. Line 395: size > PDF_READER_CACHE_MAX_FILE_BYTES
    {
        let limits = PdfCacheLimits { max_file_bytes: Some(0), max_bytes: None, max_entries: None };
        let manager = BookManager::with_deps(deps(tmp.path(), &es).with_pdf_cache_limits(limits));
        let (_r, p) = manager._get_cached_pdf_reader(&pdf_path);
        assert_eq!(p, 1);
    }
}

#[test]
fn test_get_books_in_category_paged() {
    // Lines 520-528
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let doc = make_doc("A/test.txt", ".txt");
    es.set_category_docs(vec![(1, doc.clone(), 1.0), (2, doc, 1.0)]);

    // 1. Invalid cursor (line 524)
    let error = asyncio_runner(BookManagerApi::get_books_in_category_paged(&manager, "A", None, Some("bad_cursor!!!"))).unwrap_err();
    assert_eq!(error, "invalid cursor");

    // 2. Success first page with next cursor (lines 525-528)
    let BookPage { books, next_cursor: next_cur, .. } = asyncio_runner(BookManagerApi::get_books_in_category_paged(&manager, "A", Some(1), None)).unwrap();
    assert_eq!(books.len(), 1);
    assert!(next_cur.is_some());

    // 3. Next page using cursor
    let page2 = asyncio_runner(BookManagerApi::get_books_in_category_paged(&manager, "A", Some(1), next_cur.as_deref())).unwrap();
    assert_eq!(page2.books.len(), 1);
}

#[test]
fn test_backfill_created_time_branches() {
    // Lines 545, 549, 554-555, 561-563
    let tmp = tmp_dir();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);

    // 1. Line 549: thread is alive -> returns early
    manager.set_created_time_backfill_thread(Some(Arc::new(FakeBackfillThread::alive())));
    let env = EnvGuard::single(CREATED_TIME_BACKFILL_ENV, "1");
    manager._backfill_created_time_if_enabled();

    // 2. Lines 561-563: thread.start() raises exception
    // 가정한 seam: 스레드 시작이 실패하는 실행기(TODO 주석 참고).
    let manager = BookManager::with_deps(deps(tmp.path(), &es).with_task_runner(Arc::new(FailingStartRunner { message: "start failed" })));
    manager.set_created_time_backfill_thread(None);
    manager._backfill_created_time_if_enabled();
    assert!(manager.created_time_backfill_thread().is_none());

    // 3. Lines 554-555: run_backfill raises exception
    es.fail("backfill_created_time", "backfill error");
    drop(env); // monkeypatch.undo()
    let _env = EnvGuard::single(CREATED_TIME_BACKFILL_ENV, "1");

    // 스레드를 띄우지 않고 대상 함수를 그 자리에서 실행한다.
    let manager = BookManager::with_deps(deps(tmp.path(), &es).with_task_runner(Arc::new(InlineRunner)));
    manager.set_created_time_backfill_thread(None);
    manager._backfill_created_time_if_enabled();
}

#[test]
fn test_epub_preview_raw_zip_and_ncx_branches() {
    // Lines 799, 959
    let opf_ns = "http://www.idpf.org/2007/opf";
    let ncx_ns = "http://www.daisy.org/z3986/2005/ncx/";
    let opf = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<package xmlns="{opf_ns}" version="3.0">
  <manifest>
    <item id="ch1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="raw" href="ch%20raw.xhtml" media-type="application/xhtml+xml"/>
    <item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>
  </manifest>
  <spine toc="ncx">
    <itemref idref="ch1"/>
  </spine>
</package>"#
    );
    let ncx = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<ncx xmlns="{ncx_ns}">
  <navMap>
    <navPoint id="np1"><navLabel><text>Ch1</text></navLabel><content src="ch1.xhtml"/></navPoint>
    <navPoint id="np2"><navLabel><text>Empty</text></navLabel><content src=""/></navPoint>
  </navMap>
</ncx>"#
    );
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let epub = tmp_path.join("A").join("test.epub");
    fs::create_dir_all(epub.parent().unwrap()).unwrap();
    make_epub(&epub, &minimal_spec().with_opf(&opf).with_extra("toc.ncx", ncx.into_bytes()).with_extra("ch raw.xhtml", b"<html><body>raw</body></html>".to_vec()));
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_doc(Some(make_doc("A/test.epub", "epub")));
    let result = asyncio_runner(BookManagerApi::get_book_preview(&manager, 1, None, None));
    assert!([200, 422].contains(&result.status_code), "status={}", result.status_code);
}

#[test]
fn test_normalize_stored_file_path_dot_slash() {
    // Line 1230
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    assert_eq!(manager._normalize_stored_file_path("./foo/bar.txt"), "foo/bar.txt");
}

#[test]
fn test_match_category_by_keywords_skips_invalid_mapping_entries() {
    // _classify_file_to_top_category가 사라진 뒤에도 이 검증 분기들은 _match_category_by_keywords에
    // 그대로 남아 있다: 문자열이 아닌 카테고리/키워드, 안전하지 않은 카테고리 이름, 빈/중복 키워드.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let test_file = tmp_path.join("A").join("sample.txt");
    fs::create_dir_all(test_file.parent().unwrap()).unwrap();
    fs::write(&test_file, "content").unwrap();

    let mappings: Mappings = vec![
        (json!(123), vec![json!("kw")]),                                                                    // 카테고리가 문자열이 아니면 건너뛴다
        (json!("safe_target"), vec![json!(999), json!("  "), json!("dup"), json!("dup"), json!("sample")]), // 키워드가 문자열이 아니거나 비었거나 중복이면 건너뛴다
        (json!("../unsafe"), vec![json!("kw")]),                                                            // 안전하지 않은 카테고리 이름은 건너뛴다
    ];
    let (category, matched, reason, _ranked) = manager._match_category_by_keywords(&test_file, "A", &mappings);
    assert_eq!(category.as_deref(), Some("safe_target"));
    assert!(matched.iter().any(|k| k == "sample"));
    assert!(reason.is_none());
}

#[test]
fn test_match_category_by_keywords_rejects_control_char_category() {
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let file_path = tmp_path.join("test.txt");
    fs::write(&file_path, "hello").unwrap();

    let (category, matched, reason, _ranked) = manager._match_category_by_keywords(&file_path, "0_inbox", &mappings(&[("unsafe\x00cat", &["test"])]));

    assert!(category.is_none());
    assert!(matched.is_empty());
    assert!(reason.is_none());
}

#[test]
fn test_match_category_by_keywords_reports_tie() {
    // 동점이면 목적지를 고르지 않고 사유를 돌려준다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = tmp_path.join("A").join("SF음악.epub");
    fs::create_dir(target.parent().unwrap()).unwrap();
    fs::write(&target, "x").unwrap();
    let mappings = mappings(&[("3_SF", &["SF"]), ("5_음악", &["음악"])]);

    let (category, keywords, tie_reason, ranked) = manager._match_category_by_keywords(&target, "A", &mappings);

    assert!(category.is_none());
    assert!(tie_reason.is_some());
    assert!(!keywords.is_empty());
    // 동점 상위 2개가 candidates 조립의 재료다(Task A) — 여기서도 같이 확인한다.
    assert_eq!(ranked.len(), 2);
    let cats: BTreeSet<&str> = ranked.iter().map(|(cat, _kw)| cat.as_str()).collect();
    assert_eq!(cats, BTreeSet::from(["3_SF", "5_음악"]));
}

#[test]
fn test_match_category_by_keywords_picks_single_best() {
    // 단독 최고점이면 그 카테고리를 고른다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = tmp_path.join("A").join("과학소설 모음.epub");
    fs::create_dir(target.parent().unwrap()).unwrap();
    fs::write(&target, "x").unwrap();
    let mappings = mappings(&[("3_SF", &["과학소설"]), ("5_음악", &["음악"])]);

    let (category, keywords, tie_reason, ranked) = manager._match_category_by_keywords(&target, "A", &mappings);

    assert_eq!(category.as_deref(), Some("3_SF"));
    assert_eq!(keywords, vec!["과학소설".to_string()]);
    assert!(tie_reason.is_none());
    // 순위 1위는 채택된 카테고리와 같아야 한다(Task A candidates 조립의 전제).
    assert_eq!(ranked[0], ("3_SF".to_string(), vec!["과학소설".to_string()]));
}

#[test]
fn test_match_category_by_keywords_uses_python_full_casefold() {
    let tmp = TempDir::new().unwrap();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp.path(), &es);
    let directory = tmp.path().join("A");
    fs::create_dir_all(&directory).unwrap();
    let target = directory.join("Straße\u{1c}.TXT");
    fs::write(&target, "x").unwrap();

    let (category, matched, tie_reason, _) = manager._match_category_by_keywords(&target, "A", &mappings(&[("3_SF", &["STRASSE .TXT"])]));

    assert_eq!(category.as_deref(), Some("3_SF"));
    assert_eq!(matched, ["STRASSE .TXT"]);
    assert!(tie_reason.is_none());
}

#[test]
fn test_iter_category_indexable_files_branches() {
    // Lines 1350-1351, 1353, 1358, 1359-1360
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let cat_dir = tmp_path.join("cat");
    fs::create_dir_all(&cat_dir).unwrap();

    // Line 1353: hidden file
    fs::write(cat_dir.join(".hidden.txt"), "x").unwrap();

    // 1. Lines 1350-1351: relative_to raises ValueError
    // 2. Line 1358: resolve not relative to root
    let outside_file = PathBuf::from("/tmp/outside_dummy.txt");
    let hidden = cat_dir.join(".hidden.txt");
    // iterdir 결과를 고정한 `FsOps`(구 `fs_iterdir_override`). 단계마다 다른 목록이라 매니저를 새로 만든다.
    let manager = BookManager::with_deps(deps(tmp_path, &es).with_fs(ListDirFs::new(move |_dir| vec![outside_file.clone(), hidden.clone()])));
    let files = manager._iter_category_indexable_files("cat", false);
    assert_eq!(files, Vec::<PathBuf>::new());

    // Lines 1359-1360: resolve raises OSError
    let normal_file = cat_dir.join("normal.txt");
    fs::write(&normal_file, "x").unwrap();
    let nf = normal_file.clone();
    let nf2 = normal_file.clone();
    let faulty = FaultyFs::wrapping(ListDirFs::new(move |_dir| vec![nf2.clone()]));
    faulty.set_canonicalize(move |p| (p == nf).then(|| other_err("resolve failed")));
    let manager2 = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
    let files2 = manager2._iter_category_indexable_files("cat", false);
    assert_eq!(files2, Vec::<PathBuf>::new());
}
#[test]
fn test_move_classified_file_target_traversal_and_restore_err() {
    // Lines 1385, 1498-1499
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir_all(tmp_path.join("cat")).unwrap();
    let f = tmp_path.join("cat").join("a.txt");
    fs::write(&f, "data").unwrap();

    // 1. Line 1385: target_path not relative to root
    let (_res1, err1) = split(asyncio_runner(manager._move_classified_file(&f, "../outside", "txt", MoveOpts::default())));
    assert_eq!(err1.as_deref(), Some("잘못된 대상 경로입니다"));

    // 2. Lines 1498-1499: ES reindex fails, rollback add_book raises exception
    let doc = make_doc("cat/a.txt", ".txt");
    es.set_doc(Some(doc));
    es.fail("insert", "insert boom");

    let manager = BookManager::with_deps(deps(tmp_path, &es).with_book_adder(book_adder(|| Err(AddBookError::Exception("restore add_book boom".to_string())))));
    fs::create_dir_all(tmp_path.join("target")).unwrap();
    let (_res2, err2) = split(asyncio_runner(manager._move_classified_file(&f, "target", "txt", MoveOpts::default())));
    let err2 = err2.expect("err2 is not None");
    assert!(err2.contains("기존 ES 문서 복구 실패"));
}

#[test]
fn test_get_category_mismatches_root_file() {
    // Line 1722
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    es.set_aggregate(&[]);
    fs::write(tmp_path.join("root_file.txt"), "sample").unwrap();
    // 3.2 `BookManagerApi`가 `Result<Value, String>`을 돌려주므로 풀어서 읽는다.
    let result = BookManagerApi::get_category_mismatches(&manager, false).expect("get_category_mismatches");
    assert!(result["fs_only"].as_array().unwrap().iter().any(|m| m["category"] == "_root"));
}

#[test]
fn test_reload_category_mismatch_files_non_int_book_id() {
    // Line 1995
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    // 가정한 seam: 인스턴스 메서드 바꿔치기(문서에 없는 seam, 파일 머리말 참고).
    let manager = BookManager::with_deps(
        deps(tmp_path, &es)
            .with_get_category_mismatch_details_override(Arc::new(|_cat| json!({"duplicates": [{"file_path": "cat/dup.txt", "docs": [{"book_id": "bad_id"}]}], "fs_only": [], "es_only": []}))),
    );
    fs::create_dir_all(tmp_path.join("cat")).unwrap();
    asyncio_runner(manager.reload_category_mismatch_files("cat", None)).unwrap();
}

#[test]
fn test_get_pdf_pages_storage_oserror_and_cached_reader_none() {
    // Lines 2159-2161, 2192
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let pdf_file = tmp_path.join("A").join("sample.pdf");
    fs::create_dir_all(pdf_file.parent().unwrap()).unwrap();
    fs::write(&pdf_file, blank_pdf_bytes_sized(2, 100, 100)).unwrap();

    let es = FakeEsManager::new();
    es.set_doc(Some(make_doc("A/sample.pdf", ".pdf")));

    // 1. Lines 2159-2161: book.file_path.is_file() raises OSError
    let faulty = FaultyFs::new();
    faulty.set_is_file(|_p| Some(io::Error::from_raw_os_error(5)));
    let manager_fault = BookManager::with_deps(deps(tmp_path, &es).with_fs(faulty));
    let resp1 = asyncio_runner(manager_fault.get_pdf_pages(1, 1, 2));
    assert_eq!(resp1.status_code, 503);
    assert!(resp1.body_text().contains("Storage access error"));

    // 2. Line 2192: reader is None (page count cached, but reader cache miss)
    // 쪽수 캐시 접근자는 문서에 없는 seam이다(파일 머리말 참고). 새 매니저의 reader 캐시는 비어 있어 `_pdf_reader_cache_remove`가 필요 없다.
    let manager = make_manager(tmp_path, &es);
    manager.page_count_cache().insert((pdf_file.to_string_lossy().into_owned(), mtime_key(&pdf_file)), 2);

    let resp2 = asyncio_runner(manager.get_pdf_pages(1, 1, 2));
    assert_eq!(resp2.status_code, 200);
}

#[test]
fn test_delete_category_success() {
    // Lines 2326-2328
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let es = FakeEsManager::new();
    let manager = make_manager(tmp_path, &es);
    fs::create_dir_all(tmp_path.join("test_cat")).unwrap();
    es.set_counts(&[("test_cat", 5)]);
    es.with(|s| s.deleted_by_category = json!({"deleted": 5, "failures": []}));
    let res = asyncio_runner(manager.delete_category("test_cat", false)).unwrap();
    assert_eq!(res["deleted_count"], 5);
    assert_eq!(res["category"], "test_cat");
}

// ---- Task 3: 등급 판정과 제안 생성 ----

/// classify_file 결과를 고정해 등급 규칙만 검증한다.
///
/// bookstore_candidates는 기본값 None이면 entry에 아예 안 실어서, 이 필드를
/// 모르는 기존 9(10)-케이스를 그대로 둔다(호출자가 .get() 기본값으로 방어한다는
/// 전제를 같이 검증하는 셈이다).
///
/// `FakeClassifierWithFloor`/`FakeClassifierWithCalibration`은 `with_floor`/`with_calibration`으로 옮겼다.
/// 하위 클래스만 `classifier` 속성(min_confidence, expected_accuracy)을 가지므로, 기본형은 `classifier`가 None이다.
struct FakeClassifier {
    result: (Option<String>, String, String, Value),
    override_below: f64,
    classifier: Option<FakeModel>,
}

struct FakeModel {
    min_confidence: f64,
    expected_accuracy: Option<f64>,
    calibrated: bool,
}

impl FakeClassifier {
    fn new(target: Option<&str>, method: &str, reason: &str, model_category: Option<&str>, confidence: Option<f64>) -> Self {
        let entry = json!({"confidence": confidence, "model_category": model_category});
        Self { result: (target.map(str::to_string), method.to_string(), reason.to_string(), entry), override_below: 0.5, classifier: None }
    }

    fn override_below(mut self, boundary: f64) -> Self {
        self.override_below = boundary;
        self
    }

    fn bookstore_candidates(mut self, candidates: &[(&str, i64)]) -> Self {
        self.result.3["bookstore_candidates"] = json!(candidates);
        self
    }

    fn model_candidates(mut self, candidates: &[(&str, f64)]) -> Self {
        self.result.3["model_candidates"] = json!(candidates);
        self
    }

    /// FakeClassifierWithFloor: 판정 임계값을 가진 모델 대역 (기본 0.056).
    fn with_floor(mut self) -> Self {
        self.classifier = Some(FakeModel { min_confidence: 0.056, expected_accuracy: None, calibrated: false });
        self
    }

    /// FakeClassifierWithCalibration: 확신도를 예상 정답률로 옮길 줄 아는 모델 대역.
    fn with_calibration(mut self, accuracy: f64) -> Self {
        self = self.with_floor();
        let model = self.classifier.as_mut().unwrap();
        model.expected_accuracy = Some(accuracy);
        model.calibrated = true;
        self
    }
}

impl BookClassifierApi for FakeClassifier {
    fn classify_file(&self, _path: &Path) -> (Option<String>, String, String, Value) {
        self.result.clone()
    }

    fn override_below(&self) -> f64 {
        self.override_below
    }

    fn prefers_bookstore(&self, value: f64) -> bool {
        self.override_below > 0.0 && value < self.override_below
    }

    fn min_confidence(&self) -> Option<f64> {
        self.classifier.as_ref().map(|m| m.min_confidence)
    }

    fn expected_accuracy(&self, confidence: Option<f64>) -> Option<Option<f64>> {
        // 바깥 Option은 보정 함수 유무(classifier.expected_accuracy 속성), 안쪽은 함수의 반환값이다.
        let model = self.classifier.as_ref()?;
        if !model.calibrated {
            return None;
        }
        Some(confidence.and(model.expected_accuracy))
    }
}

/// 등급 판정 9(10)-케이스: candidates 추가가 target_category/grade를 바꾸지 않는지도
/// 이 목록을 그대로 재사용해서(test_propose_category_candidates_first_matches_target_when_decided)
/// 같이 검증한다. 두 값을 따로 유지하면 표류할 수 있어 리스트를 공유한다.
fn propose_category_grade_cases() -> Vec<(Mappings, FakeClassifier, Option<&'static str>, &'static str)> {
    vec![
        (mappings(&[("3_SF", &["과학소설"])]), FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.9)), Some("3_SF"), "certain"),
        (mappings(&[("3_SF", &["과학소설"])]), FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.1)), Some("3_SF"), "unsure"),
        (mappings(&[("3_SF", &["과학소설"])]), FakeClassifier::new(Some("5_음악"), "model", "r", Some("5_음악"), Some(0.9)), Some("3_SF"), "unsure"),
        (mappings(&[("3_SF", &["과학소설"]), ("5_음악", &["과학소설"])]), FakeClassifier::new(None, "not_found", "r", None, Some(0.1)), None, "unknown"),
        (mappings(&[]), FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.9)), Some("3_SF"), "certain"),
        (mappings(&[]), FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.1)), None, "unknown"),
        (mappings(&[]), FakeClassifier::new(Some("3_SF"), "bookstore_majority", "r", None, Some(0.1)), Some("3_SF"), "certain"),
        (mappings(&[]), FakeClassifier::new(Some("3_SF"), "bookstore_single", "r", None, Some(0.1)), Some("3_SF"), "unsure"),
        (mappings(&[]), FakeClassifier::new(None, "conflict", "r", Some("3_SF"), Some(0.1)), None, "unknown"),
        (mappings(&[]), FakeClassifier::new(None, "not_found", "r", None, Some(0.1)), None, "unknown"),
    ]
}

/// 임시 디렉토리 `A/<name>`을 만들어 내용 "x"인 파일 경로를 돌려준다.
fn make_source_file(tmp_path: &Path, category: &str, name: &str) -> PathBuf {
    let source = tmp_path.join(category);
    fs::create_dir_all(&source).unwrap();
    let target = source.join(name);
    fs::write(&target, "x").unwrap();
    target
}

#[test]
fn test_propose_category_grades() {
    for (i, (mappings, fake, expected_target, expected_grade)) in propose_category_grade_cases().into_iter().enumerate() {
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let manager = make_manager(tmp_path, &FakeEsManager::new());
        let target = make_source_file(tmp_path, "A", "과학소설 모음.epub");

        let proposal = manager._propose_category_for_file(&target, "A", &mappings, &fake, true, true);

        assert_eq!(proposal["target_category"], json!(expected_target), "case {i}: target_category");
        assert_eq!(proposal["grade"], json!(expected_grade), "case {i}: grade");
    }
}

#[test]
fn test_propose_category_candidates_first_matches_target_when_decided() {
    // target_category가 None이 아닌 모든 등급에서 candidates[0]이 그 값과 같아야 한다.
    //
    // 후보를 더 보여주는 것이 등급 판정 결과(target_category)를 바꾸면 안 된다는
    // 불변식을, 등급 판정 자체를 검증하는 목록을 그대로 재사용해서 확인한다.
    for (i, (mappings, fake, expected_target, _expected_grade)) in propose_category_grade_cases().into_iter().enumerate() {
        let tmp = tmp_dir();
        let tmp_path = tmp.path();
        let manager = make_manager(tmp_path, &FakeEsManager::new());
        let target = make_source_file(tmp_path, "A", "과학소설 모음.epub");

        let proposal = manager._propose_category_for_file(&target, "A", &mappings, &fake, true, true);

        assert_eq!(proposal["target_category"], json!(expected_target), "case {i}: target_category");
        if let Some(expected) = expected_target {
            let candidates = proposal["candidates"].as_array().unwrap_or_else(|| panic!("case {i}: candidates 없음"));
            assert!(!candidates.is_empty(), "case {i}: candidates 비어 있음");
            assert_eq!(candidates[0]["category"], expected, "case {i}: candidates[0]");
        }
    }
}

/// proposal["candidates"]의 category만 뽑는다.
fn candidate_categories(proposal: &Value) -> Vec<String> {
    proposal["candidates"].as_array().unwrap().iter().map(|c| c["category"].as_str().unwrap().to_string()).collect()
}

#[test]
fn test_propose_category_candidates_keyword_top_with_runner_up() {
    // 키워드 단독 최고점이면 1위와 2위를 후보로 낸다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "과학소설 클래식 모음.epub");
    let mappings = mappings(&[("3_SF", &["과학소설"]), ("5_음악", &["클래식"])]);
    let fake = FakeClassifier::new(None, "not_found", "r", None, None);

    let proposal = manager._propose_category_for_file(&target, "A", &mappings, &fake, true, true);

    assert_eq!(proposal["target_category"], "3_SF");
    assert_eq!(candidate_categories(&proposal), ["3_SF", "5_음악"]);
    assert_eq!(proposal["candidates"][0]["category"], proposal["target_category"]);
}

#[test]
fn test_propose_category_item_always_carries_matched_keywords() {
    // 제안 항목은 어느 경로로 나오든 matched_keywords 키를 달고 나온다.
    //
    // 화면이 '왜 이 카테고리인가'를 설명할 때 쓰는 근거다. 이 키가 조용히 빠져도
    // 다른 테스트는 전부 통과하므로(실제로 한 번 그렇게 빠뜨렸다) 여기서 직접 잠근다.
    // 키워드가 맞은 경로는 맞은 키워드가 담기고, 아무것도 못 맞힌 경로는 빈 배열이다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let matched = make_source_file(tmp_path, "A", "과학소설 모음.epub");
    let unmatched = make_source_file(tmp_path, "A", "제목 없음.epub");
    let fake = FakeClassifier::new(None, "not_found", "r", None, None);

    let hit = manager._propose_category_for_file(&matched, "A", &mappings(&[("3_SF", &["과학소설"])]), &fake, true, true);
    let miss = manager._propose_category_for_file(&unmatched, "A", &mappings(&[("3_SF", &["과학소설"])]), &fake, true, true);

    assert_eq!(hit["matched_keywords"], json!(["과학소설"]));
    assert_eq!(miss["matched_keywords"], json!([]));
}

#[test]
fn test_propose_category_candidates_keyword_tie() {
    // 키워드가 동점으로 갈리면 후보 2개를 보여주되 target은 정하지 않는다.
    //
    // 체계가 못 정했다는 사실은 후보가 보여도 바뀌지 않는다 — 사람이 셀렉트박스로
    // 골라야 체크박스가 켜진다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "SF음악.epub");
    let mappings = mappings(&[("3_SF", &["SF"]), ("5_음악", &["음악"])]);
    let fake = FakeClassifier::new(None, "not_found", "r", None, None);

    let proposal = manager._propose_category_for_file(&target, "A", &mappings, &fake, true, true);

    assert!(proposal["target_category"].is_null());
    assert_eq!(proposal["grade"], "unknown");
    assert_eq!(proposal["candidates"].as_array().unwrap().len(), 2);
    let cats: BTreeSet<String> = candidate_categories(&proposal).into_iter().collect();
    assert_eq!(cats, BTreeSet::from(["3_SF".to_string(), "5_음악".to_string()]));
}

#[test]
fn test_propose_category_candidates_bookstore_conflict() {
    // 서점 판정이 갈리면 득표 상위 2개를 후보로 보여주되 target은 unknown으로 남긴다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "달빛조각사.epub");
    let fake = FakeClassifier::new(None, "conflict", "r", None, None).bookstore_candidates(&[("3_무협", 1), ("3_판타지", 1)]);

    let proposal = manager._propose_category_for_file(&target, "A", &mappings(&[]), &fake, true, true);

    assert!(proposal["target_category"].is_null());
    assert_eq!(proposal["grade"], "unknown");
    assert_eq!(candidate_categories(&proposal), ["3_무협", "3_판타지"]);
}

#[test]
fn test_propose_category_candidates_model_only_has_length_one() {
    // 모델 단독 판정은 대안이 없으니 후보가 1개다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "book.epub");
    let fake = FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.9));

    let proposal = manager._propose_category_for_file(&target, "A", &mappings(&[]), &fake, true, true);

    assert_eq!(proposal["target_category"], "3_SF");
    assert_eq!(proposal["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(proposal["candidates"][0]["category"], "3_SF");
}

#[test]
fn test_propose_category_candidates_capped_at_two_even_with_more_contenders() {
    // 경합이 3개 이상이어도 후보는 최대 2개만 보여준다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "과학 음악 역사.epub");
    let mappings = mappings(&[("3_SF", &["과학"]), ("5_음악", &["음악"]), ("7_역사", &["역사"])]);
    let fake = FakeClassifier::new(None, "not_found", "r", None, None);

    let proposal = manager._propose_category_for_file(&target, "A", &mappings, &fake, true, true);

    assert!(proposal["candidates"].as_array().unwrap().len() <= 2);
}

#[test]
fn test_propose_category_candidates_bookstore_majority_without_vote_data_does_not_understate_the_count() {
    // bookstore_candidates가 비어 있어도(오래된 캐시 등) certain 등급에 "서점 1곳"이라는
    // 자기모순적인 근거를 달면 안 된다. MIN_STORE_VOTES(book_classifier.py)는 2라서
    // bookstore_majority는 실제로 1표일 수 없는데, 근거 문구가 1곳이라고 하면 관리자가
    // "겨우 한 곳"이라는 약한 근거로 착각한 채 사전 체크된 행을 그대로 승인할 수 있다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let target = make_source_file(tmp_path, "A", "book.epub");
    let fake = FakeClassifier::new(Some("3_SF"), "bookstore_majority", "r", None, Some(0.1));

    let proposal = manager._propose_category_for_file(&target, "A", &mappings(&[]), &fake, true, true);

    assert_eq!(proposal["target_category"], "3_SF");
    assert_eq!(proposal["grade"], "certain");
    assert!(!proposal["candidates"][0]["detail"].as_str().unwrap().contains("1곳"));
}

#[test]
fn test_high_confidence_needs_a_calibrated_boundary() {
    // 정책 파일이 없으면 경계가 0이라 모든 점수가 높음이 된다. 그러면 전부 자동 체크된다.
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let fake = FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.9)).override_below(0.0);

    assert!(!manager._is_high_confidence(&fake, 0.9));
}

#[test]
fn test_high_confidence_needs_more_than_the_bookstore_boundary() {
    // 서점 경계만 넘은 낮은 점수는 '확실'이 아니다.
    //
    // 서점 경계(override_below)는 "모델과 서점 중 누구를 믿을까"를 가르려고 잰 값이지
    // "확실한가"를 가르려고 잰 값이 아니다. 이 코퍼스에서 그 값은 0.056인데, 752건을
    // 실측하니 0.10 아래 구간의 정답률은 약 53%(동전 던지기)이고 0.10 위는 약 97.7%다.
    // 경계를 그대로 쓰던 시절에는 제안의 88%가 그 상태로 미리 체크됐다.
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    let fake = FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.07)).override_below(0.056);

    // 서점 경계(0.056)는 넘지만 확실 경계(0.10)는 못 넘는다.
    assert!(!manager._is_high_confidence(&fake, 0.07));
    assert!(manager._is_high_confidence(&fake, 0.12));
}

#[test]
fn test_low_confidence_model_answer_is_not_pre_checked() {
    // 0.10 아래 모델 판정은 '애매'로 내려가 체크가 꺼진 채 사람에게 간다.
    //
    // 실제로 겪은 사례다. 성인 웹소설이 확신도 0.069~0.088로 2_수필서간일기로 판정돼
    // '확실'로 미리 체크됐다. 그 점수대의 정답률은 절반 수준이라 사람이 봐야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "9_성인", "어떤 웹소설.epub");
    let fake = FakeClassifier::new(Some("2_수필서간일기"), "model", "모델 판정", Some("2_수필서간일기"), Some(0.076)).override_below(0.056);

    let proposal = manager._propose_category_for_file(&book, "9_성인", &mappings(&[]), &fake, true, true);

    assert_eq!(proposal["grade"], BookManager::GRADE_UNKNOWN, "동전 던지기 점수가 미리 체크됐다");
    assert!(proposal["target_category"].is_null(), "점수가 낮은 답을 목적지로 썼다");
    // 모델이 뭐라고 했는지는 후보로 남는다 — 사람이 보고 그걸 고를 수도 있어야 한다.
    assert_eq!(candidate_categories(&proposal), ["2_수필서간일기"]);
}

#[test]
fn test_single_keyword_match_still_offers_a_second_choice() {
    // 키워드가 하나만 맞아도 추천 2 칸이 비지 않는다.
    //
    // 실제로 겪은 사례다. 파일명의 '에세이' 하나가 2_수필서간일기를 정하자 모델이 낸
    // 답은 통째로 버려지고 추천 2가 비었다. 키워드가 틀렸다고 느낀 관리자에게는
    // 2,000개짜리 드롭다운 말고는 길이 없었다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "9_어린이육아", "고생을 위한 철학에세이.txt");
    let fake = FakeClassifier::new(Some("9_어린이육아"), "model", "r", Some("9_어린이육아"), Some(0.116)).model_candidates(&[("9_어린이육아", 0.116), ("4_철학윤리", 0.04)]);

    let proposal = manager._propose_category_for_file(&book, "9_어린이육아", &mappings(&[("2_수필서간일기", &["에세이"])]), &fake, true, true);

    assert_eq!(proposal["target_category"], "2_수필서간일기");
    let categories = candidate_categories(&proposal);
    assert_eq!(categories, ["2_수필서간일기", "9_어린이육아"]);
    assert_eq!(proposal["candidates"][0]["category"], proposal["target_category"]);
}

#[test]
fn test_model_path_offers_its_runner_up_as_the_second_choice() {
    // 모델 판정 경로는 모델이 이미 매긴 2순위를 추천 2로 보여준다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "제목.txt");
    let fake = FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.3)).model_candidates(&[("3_SF", 0.3), ("5_음악", 0.08)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[]), &fake, true, true);

    assert_eq!(candidate_categories(&proposal), ["3_SF", "5_음악"]);
}

#[test]
fn test_model_alternative_is_not_added_when_two_candidates_already_exist() {
    // 이미 후보가 2개면 모델 대안을 덧붙이지 않는다. 2개가 상한이다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "과학소설 클래식 모음.txt");
    let fake = FakeClassifier::new(None, "not_found", "r", None, None).model_candidates(&[("9_성인", 0.2)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[("3_SF", &["과학소설"]), ("5_음악", &["클래식"])]), &fake, true, true);

    let categories = candidate_categories(&proposal);
    assert_eq!(categories.len(), 2);
    assert!(!categories.iter().any(|c| c == "9_성인"));
}

#[test]
fn test_model_alternative_never_repeats_the_destination() {
    // 모델 1순위가 이미 목적지면 같은 카테고리를 두 번 보여주지 않는다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "과학소설 모음.txt");
    let fake = FakeClassifier::new(Some("3_SF"), "model", "r", Some("3_SF"), Some(0.3)).model_candidates(&[("3_SF", 0.3)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[("3_SF", &["과학소설"])]), &fake, true, true);

    assert_eq!(candidate_categories(&proposal), ["3_SF"]);
}

fn manager_propose(manager: &BookManager, category: &str, mappings: &Mappings) -> (Value, Option<String>) {
    asyncio_runner(propose_opts(manager, category, mappings, ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: None }))
}

#[test]
fn test_propose_category_changes_lists_items_without_moving() {
    // 제안만 만들고 파일은 그대로 둔다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "과학소설 모음.epub");

    let (result, error) = manager_propose(&manager, "A", &mappings(&[("3_SF", &["과학소설"])]));

    assert!(error.is_none());
    assert_eq!(result["total_count"], 1);
    assert_eq!(result["items"][0]["file_path"], "A/과학소설 모음.epub");
    assert!(book.is_file());
}

#[test]
fn test_propose_category_changes_preserves_requested_content_type() {
    let tmp = tmp_dir();
    let manager = make_manager(tmp.path(), &FakeEsManager::new());
    make_source_file(tmp.path(), "A", "과학소설 모음.epub");
    let (result, error) = outcome(asyncio_runner(manager.propose_category_changes(
        "A",
        Some(&json!({})),
        ProposeOptions { content_type: Some("comic"), delay: Some(0.0), use_bookstore: Some(false), use_content_meta: Some(false), on_progress: None },
    )));
    assert!(error.is_none());
    assert_eq!(result["content_type"], "comic");
}

#[test]
fn test_propose_category_changes_progress_reports_new_items_only() {
    // 진행 보고의 new_items는 이번 틱에서 새로 생긴 항목만 담는다(누적이 아니다).
    //
    // 누적 목록(items)을 매번 통째로 실으면 보고 크기가 책 수의 제곱으로 커진다(과거
    // JSON 상태 파일 방식에서 실측된 문제 — 79,589권 카테고리에서 기록만 약 33시간).
    // new_items로 이번 틱의 델타만 보내야 한 틱의 비용이 책 수와 무관해진다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "가.epub");
    make_source_file(tmp_path, "A", "나.epub");

    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    let (result, error) = asyncio_runner(propose_opts(
        &manager,
        "A",
        &mappings(&[("3_SF", &["과학소설"])]),
        ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: Some(Box::new(move |call| sink.lock().unwrap().push(call))) },
    ));

    assert!(error.is_none());
    let progress = progress.lock().unwrap();
    // 첫 보고는 대상 파일 전체의 자리(이름만)를 한 번에 깔고, 이후 보고는 분류가 끝난
    // 항목만 하나씩 채운다. 어느 쪽도 누적 목록을 통째로 싣지 않는다.
    assert!(progress.iter().all(|call| call.get("items").is_none()));
    let updates: Vec<&Value> = progress.iter().filter(|call| call.get("updated_items").is_some()).collect();
    assert_eq!(updates.len(), 2);
    assert!(updates.iter().all(|call| call["updated_items"].as_array().unwrap().len() == 1));
    let updated_paths: Vec<&Value> = updates.iter().map(|call| &call["updated_items"][0]["file_path"]).collect();
    let result_paths: Vec<&Value> = result["items"].as_array().unwrap().iter().map(|item| &item["file_path"]).collect();
    assert_eq!(updated_paths, result_paths);
}

#[test]
fn test_propose_category_changes_lists_every_target_before_classifying() {
    // 분류를 시작하기 전에 대상 파일 전체가 이름만이라도 먼저 보고된다.
    //
    // 한 권에 서점 조회까지 하면 수 초가 든다. 다 끝난 뒤에 목록을 주면 관리자는 그동안
    // 무엇이 대상인지조차 알 수 없다. 첫 보고에 전체 목록이 실려야 화면이 이름부터
    // 채우고 분류 결과를 뒤이어 메울 수 있다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "가.epub");
    make_source_file(tmp_path, "A", "나.epub");

    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = progress.clone();
    asyncio_runner(propose_opts(
        &manager,
        "A",
        &mappings(&[]),
        ProposeOpts { use_bookstore: false, use_content_meta: false, on_progress: Some(Box::new(move |call| sink.lock().unwrap().push(call))) },
    ));

    let progress = progress.lock().unwrap();
    let first = &progress[0];
    assert_eq!(first["processed_count"], 0, "분류가 시작되기 전에 나와야 한다");
    assert_eq!(first["total_count"], 2);
    let mut paths: Vec<&str> = first["new_items"].as_array().unwrap().iter().map(|item| item["file_path"].as_str().unwrap()).collect();
    paths.sort();
    assert_eq!(paths, ["A/가.epub", "A/나.epub"]);
    // 아직 분류 전이라 등급도 목적지도 없다. 화면은 이것으로 "분류 중"을 구분한다.
    assert!(first["new_items"].as_array().unwrap().iter().all(|item| item["grade"].is_null() && item["target_category"].is_null()));
    assert!(first["new_items"].as_array().unwrap().iter().all(|item| item["title"].as_str().is_some_and(|t| !t.is_empty())));
}

#[test]
fn test_propose_category_changes_result_items_still_has_everything() {
    // 함수 반환값 result["items"]는 진행 보고와 무관하게 여전히 전체 항목을 담는다.
    //
    // 진행 보고는 델타(new_items)만 실어도, 최종 저장은 호출자가 반환값을 보고 하므로
    // 반환값 자체는 지금처럼 전체 목록이어야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    for i in 0..3 {
        make_source_file(tmp_path, "A", &format!("책{i}.epub"));
    }

    let (result, error) = manager_propose(&manager, "A", &mappings(&[("3_SF", &["과학소설"])]));

    assert!(error.is_none());
    assert_eq!(result["items"].as_array().unwrap().len(), 3);
}

#[test]
fn test_apply_category_changes_rejects_unknown_file() {
    // 제안에 없던 파일은 옮기지 않는다. 화면이 보낸 경로를 그대로 믿으면 안 된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "book.epub");

    let (result, error) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "3_SF"})], ApplyOpts::new(BTreeSet::new())));

    assert!(error.is_none());
    assert_eq!(result["applied_count"], 0);
    assert_eq!(result["results"][0]["apply_error"], "제안 목록에 없는 파일입니다");
}

#[test]
fn test_apply_category_changes_rejects_unsafe_target() {
    // 사용자가 고른 목적지도 검증한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "book.epub");
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "../밖"})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 0);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("카테고리"));
}

#[test]
fn test_apply_category_changes_uses_user_chosen_target() {
    // 사용자가 제안과 다른 목적지를 고르면 그쪽으로 옮긴다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "book.epub");
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 1);
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

#[test]
fn test_apply_category_changes_records_missing_file() {
    // 제안을 만든 뒤 파일이 사라졌으면 실패로 남기고 나머지를 계속 처리한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("A")).unwrap();
    let allowed = allowed_set(&["A/사라진책.epub"]);

    let (result, _error) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/사라진책.epub", "target_category": "3_SF"})], ApplyOpts::new(allowed)));

    assert_eq!(result["failed_count"], 1);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("파일"));
}

#[test]
fn test_apply_category_changes_rejects_top_level_but_unsafe_target() {
    // ".."는 슬래시가 없어 최상위 검사는 통과하지만 안전한 이름 검사에서 걸러야 한다.
    //
    // 기존 "../밖" 케이스는 슬래시가 있어 _is_top_level_target_category에서 이미
    // 걸러지므로 _is_safe_category_name이 실제로 호출되는지 증명하지 못한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    make_source_file(tmp_path, "A", "book.epub");
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": ".."})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 0);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("카테고리"));
}

#[test]
fn test_apply_category_changes_rejects_symlinked_source() {
    // 제안 시점 이후 파일이 심볼릭 링크로 바뀌면 corpus 밖 파일 유출을 막는다.
    //
    // allowed_file_paths에 있던 경로라도 실제 파일이 corpus 밖을 가리키는 링크로
    // 치환됐을 수 있다. is_file()은 링크를 따라가 True를 주므로 별도로 걸러야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("A")).unwrap();
    let outside = tmp_path.parent().unwrap().join("outside_secret.epub");
    fs::write(&outside, "secret").unwrap();
    let link_path = tmp_path.join("A").join("book.epub");
    symlink(&outside, &link_path).unwrap();
    let allowed = allowed_set(&["A/book.epub"]);

    let (result, _error) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "3_SF"})], ApplyOpts::new(allowed)));

    assert_eq!(result["applied_count"], 0);
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("링크"));
    assert!(outside.is_file());
    assert_eq!(fs::read_to_string(&outside).unwrap(), "secret");
    // 원본 테스트는 공유 임시 디렉토리에 파일을 남기므로 여기서 치운다.
    fs::remove_file(&outside).unwrap();
}

#[test]
fn test_apply_category_changes_calls_on_item_done_per_item() {
    // 항목마다 on_item_done이 file_path/apply_status/apply_error와 함께 불린다.
    //
    // 성공(moved)/실패(파일 없음) 양쪽 다 확인한다. book_manager는 DB를 모르므로
    // 이 콜백을 통해서만 호출자(main.py)가 건별로 기록할 수 있다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "ok.epub");
    let allowed = allowed_set(&["A/ok.epub", "A/missing.epub"]);
    let done_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = done_calls.clone();

    let (result, _error) = asyncio_runner(apply_opts(
        &manager,
        &[json!({"file_path": "A/ok.epub", "target_category": "5_음악"}), json!({"file_path": "A/missing.epub", "target_category": "5_음악"})],
        ApplyOpts::new(allowed).on_item_done(Box::new(move |call| sink.lock().unwrap().push(call))),
    ));

    assert_eq!(result["applied_count"], 1);
    assert_eq!(result["failed_count"], 1);
    // ok.epub은 실제 이동 직전에 "moving"으로 한 번 더 불린 뒤 "moved"로 마무리된다.
    // missing.epub은 파일이 없어 이동을 시도조차 못 하므로 moving 없이 failed 한 번뿐이다.
    let done_calls = done_calls.lock().unwrap();
    assert_eq!(done_calls.len(), 3);
    assert_eq!(done_calls[0], json!({"file_path": "A/ok.epub", "apply_status": "moving", "apply_error": null}));
    assert_eq!(done_calls[1], json!({"file_path": "A/ok.epub", "apply_status": "moved", "apply_error": null}));
    assert_eq!(done_calls[2]["file_path"], "A/missing.epub");
    assert_eq!(done_calls[2]["apply_status"], "failed");
    assert!(done_calls[2]["apply_error"].as_str().unwrap().contains("파일"));
}

#[test]
fn test_apply_category_changes_stops_before_next_item_when_should_continue_is_false() {
    // should_continue가 False를 돌려주는 순간 그 이후 항목은 아예 손대지 않는다.
    //
    // 다른 승인 작업이 이 작업을 토큰으로 대체했을 때, 진 쪽이 계속 파일을 옮기며
    // 행을 덮어쓰지 못하게 막는 장치다. 첫 항목은 정상 처리되고, 두 번째 항목
    // "앞"에서 중단되어 파일도 옮겨지지 않고 결과에도 없어야 한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "first.epub");
    make_source_file(tmp_path, "A", "second.epub");
    let allowed = allowed_set(&["A/first.epub", "A/second.epub"]);
    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();

    let (result, _error) = asyncio_runner(apply_opts(
        &manager,
        &[json!({"file_path": "A/first.epub", "target_category": "5_음악"}), json!({"file_path": "A/second.epub", "target_category": "5_음악"})],
        ApplyOpts::new(allowed).should_continue(Box::new(move || {
            let mut n = counter.lock().unwrap();
            *n += 1;
            *n <= 1 // 첫 항목 앞에서만 True, 두 번째 항목 앞에서 False
        })),
    ));

    assert_eq!(result["total_count"], 2);
    assert_eq!(result["results"].as_array().unwrap().len(), 1);
    assert_eq!(result["results"][0]["file_path"], "A/first.epub");
    assert_eq!(result["applied_count"], 1);
    assert!(tmp_path.join("5_음악").join("first.epub").is_file());
    // 두 번째는 손대지 않았다 - 여전히 원래 자리에 그대로 있다
    assert!(tmp_path.join("A").join("second.epub").is_file());
    assert_eq!(*calls.lock().unwrap(), 2);
}

#[test]
fn test_apply_category_changes_on_item_done_already_called_for_items_before_exception() {
    // 한 항목에서 예외가 나도, 그 항목까지의 on_item_done 호출은 이미 일어난 채로
    // 다음 항목 처리를 계속한다 — 중단되면 그때까지 처리한 결과가 이미 기록된
    // 것과 같은 상태를 재현한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "boom.epub");
    make_source_file(tmp_path, "A", "ok.epub");
    let allowed = allowed_set(&["A/boom.epub", "A/ok.epub"]);

    // 가정한 seam: 두 번째 인자는 교체 전 원래 구현이다.
    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();
    let manager = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_category_name_check(Arc::new(move |category, original| {
        let mut n = counter.lock().unwrap();
        *n += 1;
        if *n == 1 { Err(io::Error::other("예상치 못한 파일시스템 오류")) } else { Ok(original(category)) }
    })));
    let done_calls: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = done_calls.clone();

    let (result, _error) = asyncio_runner(apply_opts(
        &manager,
        &[json!({"file_path": "A/boom.epub", "target_category": "5_음악"}), json!({"file_path": "A/ok.epub", "target_category": "5_음악"})],
        ApplyOpts::new(allowed).on_item_done(Box::new(move |call| sink.lock().unwrap().push(call))),
    ));

    assert_eq!(result["applied_count"], 1);
    assert_eq!(result["failed_count"], 1);
    // boom.epub은 카테고리 검증 단계에서 예외가 나 이동을 시도조차 못 하므로 failed
    // 한 번뿐이다. ok.epub은 이동 직전 "moving"이 한 번 더 불린 뒤 "moved"로 끝난다.
    let done_calls = done_calls.lock().unwrap();
    assert_eq!(done_calls.len(), 3);
    assert_eq!(done_calls[0]["file_path"], "A/boom.epub");
    assert_eq!(done_calls[0]["apply_status"], "failed");
    assert_eq!(done_calls[1], json!({"file_path": "A/ok.epub", "apply_status": "moving", "apply_error": null}));
    assert_eq!(done_calls[2], json!({"file_path": "A/ok.epub", "apply_status": "moved", "apply_error": null}));
}

#[test]
fn test_apply_category_changes_continues_after_one_item_raises() {
    // 한 항목 처리 중 예외가 나도 배치 전체가 죽지 않고 나머지를 계속 처리한다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "boom.epub");
    make_source_file(tmp_path, "A", "ok.epub");
    let allowed = allowed_set(&["A/boom.epub", "A/ok.epub"]);

    let calls = Arc::new(Mutex::new(0usize));
    let counter = calls.clone();
    let manager = BookManager::with_deps(deps(tmp_path, &FakeEsManager::new()).with_category_name_check(Arc::new(move |category, original| {
        let mut n = counter.lock().unwrap();
        *n += 1;
        if *n == 1 { Err(io::Error::other("예상치 못한 파일시스템 오류")) } else { Ok(original(category)) }
    })));

    let (result, _error) = asyncio_runner(apply_opts(
        &manager,
        &[json!({"file_path": "A/boom.epub", "target_category": "5_음악"}), json!({"file_path": "A/ok.epub", "target_category": "5_음악"})],
        ApplyOpts::new(allowed),
    ));

    assert_eq!(result["total_count"], 2);
    assert_eq!(result["results"].as_array().unwrap().len(), 2);
    assert_eq!(result["results"][0]["apply_status"], "failed");
    assert_eq!(result["results"][1]["apply_status"], "moved");
    assert_eq!(result["applied_count"], 1);
    assert_eq!(result["failed_count"], 1);
}

#[test]
fn test_apply_category_changes_retry_of_failed_row_succeeds() {
    // C1: 첫 시도에서 allowed 밖이라 거부된 행도, 재시도 때 allowed에 다시 담기면
    // 실제로 이동에 성공한다.
    //
    // main.py의 허용 집합 계산이 apply_status == "pending"만 담던 버그를 재현한다 —
    // 거부됐던(failed) 행을 재시도해도 허용 집합에 안 들어가면 여기(book_manager)까지
    // 오지도 못하고 "제안 목록에 없는 파일입니다"만 영원히 반복된다. 이 테스트는 그
    // 행이 다시 allowed에 담기기만 하면 재시도가 실제로 동작함을 보인다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "book.epub");

    // 1차 시도: allowed에 없어 거부된다(버그가 있던 상태를 재현).
    let (first, _) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(BTreeSet::new())));
    assert_eq!(first["results"][0]["apply_status"], "failed");
    assert_eq!(first["results"][0]["apply_error"], "제안 목록에 없는 파일입니다");
    assert!(tmp_path.join("A").join("book.epub").is_file());

    // 재시도: failed 행이 allowed에 다시 담기면(고친 뒤 동작) 실제로 이동한다.
    let (second, _) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed_set(&["A/book.epub"]))));
    assert_eq!(second["results"][0]["apply_status"], "moved");
    assert!(!tmp_path.join("A").join("book.epub").exists());
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

#[test]
fn test_apply_category_changes_retrying_moving_row_when_file_already_moved() {
    // I4 outcome 1: moving으로 남았지만 실제로는 이미 옮겨져 있던 행을 재시도하면,
    // 원본을 못 찾아 이중 이동 없이 안전하게 failed로 기록된다.
    //
    // apply_category_changes가 이동 직전 반드시 source 파일 존재를 다시 확인하므로,
    // moving을 pending과 똑같이 취급해도 이미 끝난 이동을 또 옮기는 사고는 안 난다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("A")).unwrap();
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    // 원본은 이미 없고(실제로는 이동이 끝남), 목적지에만 파일이 있다.
    fs::write(tmp_path.join("5_음악").join("book.epub"), "x").unwrap();

    let (result, _) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed_set(&["A/book.epub"]))));

    assert_eq!(result["results"][0]["apply_status"], "failed");
    assert!(result["results"][0]["apply_error"].as_str().unwrap().contains("파일을 찾을 수 없습니다"));
    // 이미 옮겨져 있던 목적지 파일은 건드리지 않는다 — 덮어쓰거나 지우지 않는다.
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

#[test]
fn test_apply_category_changes_retrying_moving_row_when_move_never_happened() {
    // I4 outcome 2: moving으로 남았지만 실제로는 이동이 시작되지 않았던(원본이
    // 그대로인) 행을 재시도하면, 이번에는 정상적으로 이동이 완료된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    fs::create_dir(tmp_path.join("5_음악")).unwrap();
    make_source_file(tmp_path, "A", "book.epub");

    let (result, _) = asyncio_runner(apply_opts(&manager, &[json!({"file_path": "A/book.epub", "target_category": "5_음악"})], ApplyOpts::new(allowed_set(&["A/book.epub"]))));

    assert_eq!(result["results"][0]["apply_status"], "moved");
    assert!(!tmp_path.join("A").join("book.epub").exists());
    assert!(tmp_path.join("5_음악").join("book.epub").is_file());
}

mod test_es_reads_do_not_block_event_loop {
    //! 동기 elasticsearch 클라이언트를 이벤트 루프에서 직접 부르면 그 왕복 동안
    //! 같은 워커의 다른 요청이 전부 멈춘다. 읽기 경로는 워커 스레드로 넘겨야 한다.
    use super::*;

    #[tokio::test]
    async fn test_read_paths_run_off_the_event_loop_thread() {
        let tmp = tmp_dir();
        // ThreadRecordingES: ES 호출이 어느 스레드에서 실행됐는지 기록한다.
        let es = FakeEsManager::new();
        es.record_call_threads();
        let manager = make_manager(tmp.path(), &es);
        let loop_thread = std::thread::current().id();

        let _ = BookManagerApi::get_categories(&manager).await;
        let _ = manager.get_book(1).await;
        let _ = BookManagerApi::get_books_in_category_paged(&manager, "A", Some(10), None).await;
        let _ = manager.get_latest_books(10, None).await;
        let _ = manager.search_by_keyword_paged("k", 10, 0, None, None).await;

        let call_threads = es.call_threads();
        let names: BTreeSet<&str> = call_threads.keys().map(String::as_str).collect();
        assert_eq!(names, BTreeSet::from(["search_and_aggregate_by_category", "search_by_id", "search_by_category_paged", "search_latest_docs", "search_by_keyword_paged"]));
        let blocking: Vec<&String> = call_threads.iter().filter(|(_, tid)| **tid == loop_thread).map(|(name, _)| name).collect();
        assert!(blocking.is_empty(), "이벤트 루프 스레드에서 실행된 ES 호출: {blocking:?}");
    }
}

// ---------------------------------------------------------------------------
// 추천 2 — 모델이 스스로 버린 답을 대안으로 내밀지 않는다
// ---------------------------------------------------------------------------

#[test]
fn test_model_alternative_below_the_decision_floor_is_not_offered() {
    // 임계값 미만 점수는 모델이 같은 턴에 '판정하지 않음'으로 버린 값이다.
    //
    // 그것을 추천 칸에 올리면, 시스템이 근거 없다고 판단한 카테고리를 관리자가
    // 근거 있는 추천으로 읽는다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "제목.txt");
    let fake = FakeClassifier::new(Some("2_소설외국"), "bookstore_majority", "r", None, Some(0.076))
        .with_floor()
        .bookstore_candidates(&[("2_소설외국", 3)])
        .model_candidates(&[("2_소설외국", 0.076), ("2_수필서간일기", 0.046)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[]), &fake, true, true);

    assert_eq!(candidate_categories(&proposal), ["2_소설외국"]);
}

#[test]
fn test_model_alternative_at_or_above_the_decision_floor_is_offered() {
    // 임계값을 넘긴 대안은 그대로 보여준다. 게이트가 2순위를 통째로 죽이면 안 된다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "제목.txt");
    let fake = FakeClassifier::new(Some("2_소설외국"), "bookstore_majority", "r", None, Some(0.2))
        .with_floor()
        .bookstore_candidates(&[("2_소설외국", 3)])
        .model_candidates(&[("2_소설외국", 0.2), ("3_스릴러", 0.09)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[]), &fake, true, true);

    assert_eq!(candidate_categories(&proposal), ["2_소설외국", "3_스릴러"]);
}

#[test]
fn test_bookstore_votes_beat_a_model_alternative_when_no_destination_was_chosen() {
    // 목적지를 못 정했어도 서점 득표가 있으면 그것을 추천 2로 보여준다.
    //
    // 모델 확신도가 '확실' 기준에 못 미치면 목적지가 빈다. 그때 화면이 모델의
    // 저점수 대안만 늘어놓으면, 정작 서점이 남긴 근거가 안 보인다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "제목.txt");
    let fake = FakeClassifier::new(None, "model", "r", Some("2_소설외국"), Some(0.076))
        .with_floor()
        .bookstore_candidates(&[("8_건강일반", 1)])
        .model_candidates(&[("2_소설외국", 0.076), ("2_수필서간일기", 0.046)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[]), &fake, true, true);

    assert!(proposal["target_category"].is_null());
    assert_eq!(candidate_categories(&proposal), ["2_소설외국", "8_건강일반"]);
    assert_eq!(proposal["candidates"][1]["source"], "bookstore");
}

// ---------------------------------------------------------------------------
// 점수 표기 — 확신도 대신 예상 정답률
// ---------------------------------------------------------------------------

#[test]
fn test_proposal_carries_expected_accuracy() {
    // 0.076 은 좋은 점수인지 화면에서 읽을 수 없다. 보정한 정답률을 함께 보낸다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "제목.txt");
    let fake = FakeClassifier::new(Some("2_소설외국"), "bookstore_majority", "r", None, Some(0.076)).with_calibration(0.93).bookstore_candidates(&[("2_소설외국", 3)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[]), &fake, true, true);

    assert_eq!(proposal["confidence"], 0.076);
    assert_eq!(proposal["expected_accuracy"], 0.93);
}

#[test]
fn test_proposal_expected_accuracy_is_none_without_calibration() {
    // 보정이 없는 옛 모델에서는 아무 값도 주장하지 않는다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    let manager = make_manager(tmp_path, &FakeEsManager::new());
    let book = make_source_file(tmp_path, "A", "제목.txt");
    let fake = FakeClassifier::new(Some("2_소설외국"), "bookstore_majority", "r", None, Some(0.076)).with_floor().bookstore_candidates(&[("2_소설외국", 3)]);

    let proposal = manager._propose_category_for_file(&book, "A", &mappings(&[]), &fake, true, true);

    assert!(proposal["expected_accuracy"].is_null());
}

// --- 미선언 네임스페이스 접두사 보정 ---

const OPF_NS_STR: &str = "http://www.idpf.org/2007/opf";

fn opf(sep: &str, attr: &str, extra_decl: &str) -> Vec<u8> {
    format!(r#"<?xml version="1.0"?>{}<package{sep}xmlns="{OPF_NS_STR}"{extra_decl} version="3.0"><metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:creator {attr}>x</dc:creator></metadata><manifest/><spine/></package>"#, "\n").into_bytes()
}

fn strict_parses(data: &[u8]) -> bool {
    match std::str::from_utf8(data) {
        Ok(text) => roxmltree::Document::parse(text).is_ok(),
        Err(_) => false,
    }
}

#[test]
fn test_declare_unbound_prefixes_handles_any_whitespace_after_package() {
    for sep in [" ", "\n  ", "\t"] {
        let data = opf(sep, r#"opf:role="aut""#, "");
        let fixed = _declare_unbound_prefixes(&data);
        assert!(strict_parses(&fixed), "sep={sep:?}");
        assert!(String::from_utf8_lossy(&fixed).contains(&format!(r#"xmlns:opf="{OPF_NS_STR}""#)), "sep={sep:?}");
    }
}

#[test]
fn test_declare_unbound_prefixes_handles_prefixes_other_than_opf() {
    for (attr, prefix) in [(r#"xsi:type="x""#, "xsi"), (r#"calibre:series="s""#, "calibre")] {
        let data = opf(" ", attr, "");
        let fixed = _declare_unbound_prefixes(&data);
        assert!(strict_parses(&fixed), "attr={attr}");
        assert!(String::from_utf8_lossy(&fixed).contains(&format!("xmlns:{prefix}=")), "attr={attr}, prefix={prefix}");
    }
}

#[test]
fn test_declare_unbound_prefixes_returns_input_unchanged_when_nothing_is_missing() {
    let clean = opf(" ", r#"id="a""#, "");
    let out = _declare_unbound_prefixes(&clean);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert!(std::ptr::eq(out.as_ptr(), clean.as_ptr()));
    let declared = opf(" ", r#"opf:role="aut""#, &format!(r#" xmlns:opf="{OPF_NS_STR}""#));
    let out = _declare_unbound_prefixes(&declared);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert!(std::ptr::eq(out.as_ptr(), declared.as_ptr()));
}

#[test]
fn test_declare_unbound_prefixes_ignores_prefixes_that_only_appear_in_values_and_text() {
    let data = br#"<?xml version="1.0"?><!-- <fake:tag a:b="1"/> --><package xmlns="http://www.idpf.org/2007/opf"><meta property="dcterms:modified">urn:uuid:x</meta></package>"#;
    let out = _declare_unbound_prefixes(data);
    assert!(matches!(out, Cow::Borrowed(_)));
    assert!(std::ptr::eq(out.as_ptr(), data.as_ptr()));
}

#[test]
fn test_declare_unbound_prefixes_keeps_xml_prefix_and_declares_each_prefix_once() {
    let data = br#"<package xmlns="http://www.idpf.org/2007/opf" xml:lang="ko"><a opf:x="1" opf:y="2"/><b xsi:z="1"/></package>"#;
    let fixed = String::from_utf8(_declare_unbound_prefixes(data).into_owned()).unwrap();
    assert!(fixed.matches("xmlns:opf=").count() == 1 && !fixed.contains("xmlns:xml="));
    assert!(strict_parses(fixed.as_bytes()));
}

#[test]
fn test_declare_unbound_prefixes_without_root_element_is_unchanged() {
    assert_eq!(_declare_unbound_prefixes(b"not xml").as_ref(), b"not xml");
}

// --- 누락된 spine 문서: 총 챕터 수와 NCX 목차 ---

const NCX_NS_STR: &str = "http://www.daisy.org/z3986/2005/ncx/";

fn epub_with_missing_page(path: &Path, include_page: bool) -> PathBuf {
    let container = r#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    let opf = r#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="2.0"><manifest><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/><item id="a" href="Text/a.xhtml" media-type="application/xhtml+xml"/><item id="colophon" href="Text/colophon%20page.xhtml" media-type="application/xhtml+xml"/></manifest><spine toc="ncx"><itemref idref="a"/><itemref idref="colophon"/><itemref idref="ghost"/></spine></package>"#;
    let ncx = format!(
        r#"<?xml version="1.0"?><ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="n1"><navLabel><text>A</text></navLabel><content src="Text/a.xhtml#top"/></navPoint><navPoint id="n2"><navLabel><text>판권</text></navLabel><content src="Text/colophon%20page.xhtml"/><navPoint id="n3"><navLabel><text>자식</text></navLabel><content src="Text/a.xhtml#kid"/></navPoint></navPoint></navMap></ncx>"#
    );
    let a = "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>본문</p></body></html>";
    let colophon = "<html xmlns='http://www.w3.org/1999/xhtml'><body><p>판권</p></body></html>";
    let mut entries: Vec<(&str, &[u8])> = vec![
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/toc.ncx", ncx.as_bytes()),
        ("OEBPS/Text/a.xhtml", a.as_bytes()),
    ];
    if include_page {
        entries.push(("OEBPS/Text/colophon page.xhtml", colophon.as_bytes()));
    }
    write_zip(path, &entries);
    path.to_path_buf()
}

#[test]
fn test_total_chapters_counts_only_spine_items_that_exist_in_the_zip() {
    // manifest 에 없는 idref 와 ZIP 에 없는 문서는 뷰어가 보여 줄 수 없으므로 세지 않는다.
    let tmp = tmp_dir();
    let tmp_path = tmp.path();
    assert_eq!(BookManager::_get_epub_total_chapters(&epub_with_missing_page(&tmp_path.join("missing.epub"), false)), 1);
    assert_eq!(BookManager::_get_epub_total_chapters(&epub_with_missing_page(&tmp_path.join("full.epub"), true)), 2);
}

fn ncx_srcs(data: &[u8]) -> Vec<String> {
    let text = std::str::from_utf8(data).unwrap();
    let doc = roxmltree::Document::parse(text).unwrap();
    doc.descendants().filter(|n| n.has_tag_name((NCX_NS_STR, "content"))).filter_map(|c| c.attribute("src").map(str::to_string)).collect()
}

#[test]
fn test_prune_dead_ncx_navpoints_drops_missing_targets_and_keeps_children() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("m.epub"), false);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml"]);
    let pruned = _prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).expect("pruned");
    // 판권 항목만 사라지고, 그 아래 자식은 부모 자리로 올라온다
    assert_eq!(ncx_srcs(&pruned), ["Text/a.xhtml#top", "Text/a.xhtml#kid"]);
}

#[test]
fn test_prune_dead_ncx_navpoints_returns_none_when_nothing_is_dead() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("f.epub"), true);
    let ncx = read_zip_entry(&epub, "OEBPS/toc.ncx");
    let names: BTreeSet<&str> = BTreeSet::from(["OEBPS/Text/a.xhtml", "OEBPS/Text/colophon page.xhtml"]);
    assert!(_prune_dead_ncx_navpoints(&ncx, "OEBPS", &|path| names.contains(path)).is_none());
}

#[test]
fn test_prune_dead_ncx_navpoints_ignores_unparsable_ncx_and_empty_src() {
    assert!(_prune_dead_ncx_navpoints(b"<ncx", "OEBPS", &|_path| false).is_none());
    let no_src = format!(r#"<ncx xmlns="{NCX_NS_STR}"><navMap><navPoint id="x"><content src=""/></navPoint><navPoint id="y"/></navMap></ncx>"#).into_bytes();
    assert!(_prune_dead_ncx_navpoints(&no_src, "", &|_path| false).is_none());
}

#[test]
fn test_total_chapters_counts_percent_encoded_zip_entry_names() {
    // ZIP 항목 이름이 퍼센트 인코딩된 채 저장돼 있어도(href 는 풀어서 비교) 세어야 한다.
    let tmp = tmp_dir();
    let epub = tmp.path().join("pct.epub");
    write_zip(
        &epub,
        &[
            ("META-INF/container.xml", br#"<?xml version="1.0"?><container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="content.opf"/></rootfiles></container>"#),
            ("content.opf", br#"<package xmlns="http://www.idpf.org/2007/opf"><manifest><item id="a" href="a%20b.xhtml" media-type="application/xhtml+xml"/></manifest><spine><itemref idref="a"/></spine></package>"#),
            ("a%20b.xhtml", b"<html/>"),
        ],
    );
    assert_eq!(BookManager::_get_epub_total_chapters(&epub), 1);
}

#[test]
fn test_total_chapters_does_not_count_chapter_with_broken_zip_entry() {
    let tmp = tmp_dir();
    let epub = epub_with_missing_page(&tmp.path().join("broken.epub"), true);
    let offset = zip_header_offset(&epub, "OEBPS/Text/colophon page.xhtml") as usize;
    let mut data = fs::read(&epub).unwrap();
    data[offset..offset + 4].copy_from_slice(&[0, 0, 0, 0]);
    fs::write(&epub, &data).unwrap();
    assert_eq!(BookManager::_get_epub_total_chapters(&epub), 1);
}
