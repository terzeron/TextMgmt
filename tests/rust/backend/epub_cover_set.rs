//! 원본: tests/test_epub_cover_set.py
//!
//! EPUB 뷰어의 표지 이미지 추가: `BookManager::set_epub_cover`와 `POST /epub-cover/{id}`.
//! 운영 파일 대신 임시 EPUB를 쓰고, 이미지 다운로드·ES·책 조회만 통일 seam(`BookManagerDeps`)으로 바꾼다. 설계: docs/rust-test-seams.md 3.2, 3.5, 3.7, 3.12.
//!
//! 가정한 API (아직 구현 전이다). 함수명과 시그니처는 Python과 같다. 튜플 반환 `(result, error)`는 그대로 튜플이다.
//! - `backend::epub_cover::{CoverError, EpubCoverError}`
//!   `CoverError::new(msg: &str) -> CoverError` (`Display`), `enum EpubCoverError { Cover(CoverError), Io(std::io::Error) }` (`Debug`).
//!   Python의 `CoverError`와 `OSError`/`FileExistsError`를 한 `Result` 에러 타입으로 합친 것이다. `From<FetchError> for EpubCoverError`가 있다.
//! - `backend::epub_cover::inspect_epub(path: &Path) -> Result<Value, CoverError>` (`has_cover: bool`, `revision: String`)
//! - `backend::epub_cover::add_cover(path: &Path, image: &[u8], revision: &str, backup_dir: &Path, replace_existing: bool, backup_name: Option<&str>) -> Result<Value, CoverError>`
//! - `backend::epub_cover::remove_cover(path: &Path, revision: &str, backup_dir: &Path) -> Result<Value, CoverError>`
//! - `backend::epub_cover::add_illustration(path: &Path, image: &[u8], revision: &str, backup_dir: &Path, before_idref: &str) -> Result<Value, CoverError>`
//! - `backend::epub_cover::member_path(base: &str, href: &str) -> String`
//! - `backend::epub_cover::_stage_illustration(source: &mut File, staged: &Path, image: &[u8], before_idref: &str, existing: &Value) -> Result<_, EpubCoverError>`
//! - `backend::epub_cover::_stage_cover_removal(source: &mut File, staged: &Path) -> Result<_, EpubCoverError>`
//! - `backend::epub_cover::EpubCoverHooks` (Default + Clone. 구 `epub_cover::hooks::set_*` 5종): `.with_fs(Arc<dyn FsOps>)`(fsync는 `FsOps::sync_file`. `common::faulty_fs::FaultyFs`),
//!   `.with_limits(Limits)`(`Limits { max_xml_bytes, .. }`, `Default`는 운영 값), `.with_testzip(Arc<dyn Fn(&Path) -> Option<String>>)`, `.with_inspect_archive(Arc<dyn Fn(Value) -> Value>)`, `.with_uuid_hex(Arc<dyn Fn() -> String>)`.
//!   말단 함수는 `*_with(.., &EpubCoverHooks)`: `add_cover_with(path, image, revision, backup_dir, replace_existing, backup_name, &hooks)`는 문서 3.12에 있고,
//!   `add_illustration_with(path, image, revision, backup_dir, before_idref, &hooks)`, `remove_cover_with(path, revision, backup_dir, &hooks)`는 같은 모양으로 가정했다(문서에 없다).
//! - `backend::book_manager::{BookManager, BookManagerDeps, BookManagerApi, BookRef, BookLookup}`
//!   `BookRef { book_id: i64, file_path: PathBuf, file_type: String }` (`Clone`)
//!   `BookManager::with_deps(BookManagerDeps::new(path_prefix, es).with_trash_dirname(&str).with_book_lookup(Arc<dyn BookLookup>).with_image_fetcher(Arc<dyn ImageFetcher>))` (구 `BookManagerDeps { .. }` 리터럴)
//!   `BookManagerApi::set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> (Option<Value>, Option<String>)` (async)
//!   `BookManagerApi::remove_epub_cover(&self, book_id: i64) -> (Option<Value>, Option<String>)` (async)
//!   `BookManagerApi::insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> (Option<Value>, Option<String>)` (async)
//!   (구 `EpubCoverApi`를 `BookManagerApi`에 합쳤다. 모든 메서드에 기본 구현이 있어 fake는 쓰는 메서드만 구현한다)
//!   `BookManager::find_epub_cover_member(archive: &mut ZipArchive<File>) -> Option<String>`
//!   `trait BookLookup { async fn get_book(&self, book_id: i64) -> Result<BookRef, BookLookupError> }` (`async_trait`)
//! - `backend::direct_cover_url::{ImageFetcher, FetchError}`: `trait ImageFetcher: Send + Sync { fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError>; }`, `enum FetchError { Value(String), Os(String), Other(String) }`
//! - `backend::es_manager::EsManagerApi::update(&self, doc_id: i64, fields: &Value) -> Result<bool, EsError>` (`common::es_fake::FakeEsManager`가 구현한다)
//! - `backend::main::{app_with, AppDeps, AuthMode}`: `AppDeps::new(AuthMode::Fixed(Value)).with_book_manager(Arc<dyn BookManagerApi>) -> AppDeps`, `app_with(deps) -> axum::Router`
//!   (구 `app_with_overrides(AppOverrides { book_manager, admin })`. Python `dependency_overrides[require_admin]`와 `book_manager._instance` 교체의 대응이다)
//!
//! 가정한 dev-dependency: zip, image, base64, sha2, roxmltree, chrono, async-trait, tempfile, tokio, serde_json.

mod common;

use std::fs::{self, File};
use std::io::{Cursor, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use backend::book::BookLookupError;
use backend::book_manager::{BookLookup, BookManager, BookManagerApi, BookManagerDeps, BookRef};
use backend::direct_cover_url::{FetchError, ImageFetcher};
use backend::epub_cover::{self, CoverError, EpubCoverHooks, Limits};
use backend::main::{AppDeps, AuthMode, app_with};
use common::{
    TestClient,
    epub::{EpubSpec, make_epub},
    es_fake::FakeEsManager,
    faulty_fs::{FaultyFs, other_err},
    fsx::{dir_names, mtime_key, sha256_hex},
    image::{b64, color_jpeg, red_jpeg},
    zipx::{Members, bytes_replace, rewrite_epub_members},
};
use serde_json::{Value, json};

const URL: &str = "https://img.example.com/cover.jpg";

// ── 도구 ───────────────────────────────────────────────────

// TODO(common): 메모리 바이트열에서 `Members`를 읽는 함수가 zipx에 없다(`Members::read(path)`만 있다).
fn members_of(data: &[u8]) -> Members {
    let mut archive = zip::ZipArchive::new(Cursor::new(data)).unwrap();
    let mut items = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).unwrap();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        items.push((entry.name().unwrap().to_string(), bytes));
    }
    Members::new(items)
}

/// `Members`의 (이름, 내용) 쌍. 멤버 필드가 비공개라 이름 목록으로 만든다.
fn entries(members: &Members) -> Vec<(String, Vec<u8>)> {
    members.names().into_iter().map(|name| (members.get(&name).to_vec(), name)).map(|(data, name)| (name, data)).collect()
}

fn has_cover(path: &Path) -> bool {
    epub_cover::inspect_epub(path).unwrap()["has_cover"].as_bool().unwrap()
}

fn revision(path: &Path) -> String {
    epub_cover::inspect_epub(path).unwrap()["revision"].as_str().unwrap().to_string()
}

/// `datetime.strptime(value, "%Y-%m-%dT%H:%M:%S.%f")`: 소수부가 없거나 6자리를 넘으면 실패한다.
fn parse_updated_time(value: &str) -> chrono::NaiveDateTime {
    let (head, fraction) = value.split_once('.').unwrap_or_else(|| panic!("updated_time 형식 오류: {value}"));
    assert!((1..=6).contains(&fraction.len()) && fraction.bytes().all(|b| b.is_ascii_digit()), "updated_time 소수부 오류: {value}");
    let stamp = chrono::NaiveDateTime::parse_from_str(head, "%Y-%m-%dT%H:%M:%S").unwrap_or_else(|e| panic!("updated_time 형식 오류: {value}: {e}"));
    let micros: u32 = format!("{fraction:0<6}").parse().unwrap();
    stamp + chrono::Duration::microseconds(micros as i64)
}

/// `path.parent.glob(".cover-*.tmp")`
fn cover_tmp_files(dir: &Path) -> Vec<String> {
    (if dir.exists() { dir_names(dir) } else { Vec::new() }).into_iter().filter(|n| n.starts_with(".cover-") && n.ends_with(".tmp")).collect()
}

// ── XML 확인 도구 (lxml xpath 대응) ────────────────────────

fn xml_text(data: &[u8]) -> String {
    String::from_utf8(data.to_vec()).unwrap()
}

/// local-name이 `name`인 모든 요소를 문서 순서로 돌려준다.
fn elements<'a, 'i>(doc: &'a roxmltree::Document<'i>, name: &str) -> Vec<roxmltree::Node<'a, 'i>> {
    doc.descendants().filter(|n| n.is_element() && n.tag_name().name() == name).collect()
}

fn spine_idrefs(path: &Path) -> Vec<String> {
    let text = xml_text(Members::read(path).get("OPS/book.opf"));
    let doc = roxmltree::Document::parse(&text).unwrap();
    elements(&doc, "spine").iter().flat_map(|s| s.children().filter(|c| c.is_element() && c.tag_name().name() == "itemref")).filter_map(|c| c.attribute("idref").map(String::from)).collect()
}

fn ncx_doc(nav_map: &str, extra: &str) -> String {
    format!(r#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap>{nav_map}</navMap>{extra}</ncx>"#)
}

fn nav_doc(body: &str) -> String {
    format!(r#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc">{body}</nav></body></html>"#)
}

/// 형님 EPUB과 같은 SVG 표지, guide, NCX 구조를 만든다.
fn attach_publisher_cover_page(path: &Path, shared: bool) {
    rewrite_epub_members(path, |files| {
        let mut opf = xml_text(files.get("OPS/book.opf"));
        let version3 = opf.contains(r#"version="3.0""#);
        let mut items = String::from(r#"<item id="titlepage" href="Text/front.xhtml" media-type="application/xhtml+xml"/><item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/>"#);
        if version3 {
            items.push_str(r#"<item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav"/>"#);
        }
        opf = opf.replace("</manifest>", &format!("{items}</manifest>"));
        opf = opf.replace("<spine>", r#"<spine toc="ncx"><itemref idref="titlepage"/>"#);
        opf = opf.replace("</package>", r#"<guide><reference type="cover" href="Text/front.xhtml#cover"/></guide></package>"#);
        files.set("OPS/book.opf", opf.into_bytes());
        files.set(
            "OPS/Text/front.xhtml",
            br#"<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Cover</title></head><body><svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><image xlink:href="../cover.jpg"/></svg></body></html>"#.to_vec(),
        );
        files.set(
            "OPS/toc.ncx",
            br#"<ncx xmlns="http://www.daisy.org/z3986/2005/ncx/"><navMap><navPoint id="cover" playOrder="1"><navLabel><text>Cover</text></navLabel><content src="Text/front.xhtml#cover"/></navPoint><navPoint id="chapter" playOrder="2"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml"/></navPoint></navMap></ncx>"#.to_vec(),
        );
        if version3 {
            files.set(
                "OPS/nav.xhtml",
                br#"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><nav epub:type="toc"><ol><li><a href="Text/front.xhtml#cover">Cover</a></li><li><a href="chapter.xhtml">Chapter</a></li></ol></nav></body></html>"#.to_vec(),
            );
        }
        if shared {
            files.set("OPS/chapter.xhtml", br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Keep this chapter.</p><img src="cover.jpg"/></body></html>"#.to_vec());
        }
    });
}

// ── 대역 (env fixture) ─────────────────────────────────────

type Fetch = Box<dyn Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync>;

/// `monkeypatch.setattr("backend.cover_management.fetch_url_image", ...)`의 대응. 테스트 중간에 바꿀 수 있다.
/// `BookManagerDeps::with_image_fetcher`로 주입한다. 오류 타입은 `FetchError`(구 `EpubCoverError`).
struct FetchSlot(Mutex<Fetch>);

impl FetchSlot {
    fn set(&self, f: impl Fn(&str) -> Result<Vec<u8>, FetchError> + Send + Sync + 'static) {
        *self.0.lock().unwrap() = Box::new(f);
    }
}

impl ImageFetcher for FetchSlot {
    fn fetch_url_image(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        (self.0.lock().unwrap())(url)
    }
}

/// `AsyncMock(return_value=(book, None))` get_book.
struct FakeBooks(Mutex<(Option<BookRef>, Option<String>)>);

impl FakeBooks {
    fn set_file_type(&self, file_type: &str) {
        self.0.lock().unwrap().0.as_mut().unwrap().file_type = file_type.to_string();
    }

    fn set_missing(&self, message: &str) {
        *self.0.lock().unwrap() = (None, Some(message.to_string()));
    }
}

#[async_trait::async_trait]
impl BookLookup for FakeBooks {
    async fn get_book(&self, book_id: i64) -> Result<BookRef, BookLookupError> {
        match self.0.lock().unwrap().clone() {
            (Some(book), _) => Ok(book),
            (None, Some(error)) => Err(BookLookupError::Failed(error)),
            (None, None) => Err(BookLookupError::NotFound(book_id)),
        }
    }
}

/// (manager 대역, EPUB 경로, 책). get_book은 이 EPUB를 가리키는 책을 돌려준다.
struct Env {
    tmp: tempfile::TempDir,
    path: PathBuf,
    manager: BookManager,
    es: Arc<FakeEsManager>,
    books: Arc<FakeBooks>,
    fetcher: Arc<FetchSlot>,
}

fn env() -> Env {
    common::setup_env();
    let tmp = tempfile::tempdir().unwrap();
    let fetcher = Arc::new(FetchSlot(Mutex::new(Box::new(|_| Ok(red_jpeg())))));
    let path = make_epub(&tmp.path().join("분류").join("book.epub"), &EpubSpec::cover("3.0", false));
    let es = FakeEsManager::new();
    let books = Arc::new(FakeBooks(Mutex::new((Some(BookRef { book_id: 1, file_path: path.clone(), file_type: "epub".into() }), None))));
    let manager = BookManager::with_deps(BookManagerDeps::new(tmp.path().to_path_buf(), es.clone()).with_trash_dirname("0_trash").with_book_lookup(books.clone()).with_image_fetcher(fetcher.clone()));
    Env { tmp, path, manager, es, books, fetcher }
}

impl Env {
    fn dir(&self) -> &Path {
        self.tmp.path()
    }

    async fn call(&self, url: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.set_epub_cover(1, Some(url), None).await)
    }

    async fn upload(&self, image_data: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.set_epub_cover(1, None, Some(image_data)).await)
    }

    async fn remove(&self) -> (Option<Value>, Option<String>) {
        split(self.manager.remove_epub_cover(1).await)
    }

    async fn illustrate(&self, before: &str) -> (Option<Value>, Option<String>) {
        split(self.manager.insert_epub_illustration(1, before, Some(URL), None).await)
    }

    fn size(&self) -> u64 {
        fs::metadata(&self.path).unwrap().len()
    }
}

/// `"x" in error`: error가 None이면 Python은 TypeError로 실패한다.
/// `Result`를 Python 원본 테스트의 `(결과, 오류)` 모양으로 펼친다. 단언을 원본과 나란히 읽게 한다.
fn split(result: Result<Value, String>) -> (Option<Value>, Option<String>) {
    match result {
        Ok(value) => (Some(value), None),
        Err(error) => (None, Some(error)),
    }
}

fn err_has(error: &Option<String>, needle: &str) -> bool {
    error.as_deref().unwrap_or_else(|| panic!("error가 None이다 (기대: {needle:?} 포함)")).contains(needle)
}

/// `assert error` (None이나 빈 문자열이면 실패)
fn err_truthy(error: &Option<String>) -> bool {
    error.as_deref().is_some_and(|e| !e.is_empty())
}

/// `pytest.raises(CoverError, match=needle)`
fn assert_cover_error<T: std::fmt::Debug>(result: Result<T, CoverError>, needle: &str) {
    match result {
        Err(e) if !e.is_io() => assert!(e.to_string().contains(needle), "CoverError 메시지에 {needle:?}가 없다: {e}"),
        other => panic!("CoverError({needle:?})를 기대했지만 {other:?}"),
    }
}

#[tokio::test]
async fn test_adds_a_cover_to_an_epub_without_one() {
    let env = env();
    assert!(!has_cover(&env.path));

    let (result, error) = env.call(URL).await;

    assert!(error.is_none());
    assert!(has_cover(&env.path));
    let result = result.unwrap();
    assert_eq!(result["file_size"].as_u64(), Some(env.size()));
    let calls = env.es.updates();
    assert_eq!(calls.len(), 1, "update는 한 번만 불려야 한다");
    let (doc_id, kwargs) = &calls[0];
    assert!(*doc_id == 1 && kwargs["file_size"].as_u64() == Some(env.size()));
    // 최신 책 목록(updated_time 순)에 오르도록 방금 시각을 색인한다. 만화 인덱스의 strptime 형식과도 맞아야 한다.
    let stamp = parse_updated_time(kwargs["updated_time"].as_str().unwrap());
    assert!((chrono::Local::now().naive_local() - stamp).num_milliseconds().abs() < 60_000);
    assert!(result.get("warning").is_none());
}

#[tokio::test]
async fn test_resolves_the_book_from_elasticsearch_without_a_lookup_override() {
    let env = env();
    let relative_path = env.path.strip_prefix(env.dir()).unwrap();
    env.es.set_doc(Some(json!({
        "category": "분류",
        "title": "book",
        "author": "",
        "file_path": relative_path.to_string_lossy(),
        "file_type": "epub",
        "file_size": env.size(),
        "updated_time": "2024-01-01T00:00:00.000000"
    })));
    let manager = BookManager::with_deps(BookManagerDeps::new(env.dir().to_path_buf(), env.es.clone()).with_trash_dirname("0_trash").with_image_fetcher(env.fetcher.clone()));

    let (result, error) = split(manager.set_epub_cover(1, Some(URL), None).await);

    assert!(error.is_none());
    assert!(result.is_some());
    assert!(has_cover(&env.path));
    assert_eq!(env.es.updates().len(), 1);
}

#[tokio::test]
async fn test_replaces_an_existing_cover_and_keeps_the_original_in_0_trash() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let original = fs::read(&env.path).unwrap();

    let (_result, error) = env.call(URL).await;

    assert!(error.is_none());
    assert_ne!(fs::read(&env.path).unwrap(), original);
    // PDF 편집과 같이 0_trash 에 원래 이름으로 둔다. 숨김 폴더 .cover_backups 는 쓰지 않는다.
    assert_eq!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap(), original);
    assert!(!env.dir().join(".cover_backups").exists());
}

#[tokio::test]
async fn test_second_edit_keeps_the_first_original() {
    let env = env();
    let first_original = fs::read(&env.path).unwrap();

    env.call(URL).await;
    let after_first = fs::read(&env.path).unwrap();
    env.fetcher.set(|_| Ok(color_jpeg("blue")));
    let (_result, error) = env.call(URL).await;

    assert!(error.is_none());
    assert_ne!(fs::read(&env.path).unwrap(), after_first);
    // 처음 원본을 지킨다(덮어쓰지 않는다). 임시 롤백 파일도 남기지 않는다.
    assert_eq!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap(), first_original);
    assert_eq!(dir_names(&env.dir().join("0_trash")), ["book.epub"]);
    assert!(cover_tmp_files(env.path.parent().unwrap()).is_empty());
}

/// 백업이 이미 있으면 그것은 처음 원본이다. 쓰다가 실패하면 지금 원본으로 되돌려야 한다.
#[test]
fn test_failed_write_restores_the_current_file_not_the_older_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let tmp_path = tmp.path();

    let path = make_epub(&tmp_path.join("book.epub"), &EpubSpec::cover("3.0", false));
    let trash = tmp_path.join("0_trash");
    epub_cover::add_cover_backup_with(&path, &red_jpeg(), &revision(&path), &trash, false, Some("book.epub"), &EpubCoverHooks::default()).unwrap();
    let current = fs::read(&path).unwrap();
    let older = fs::read(trash.join("book.epub")).unwrap();
    assert_ne!(current, older);

    let counter = AtomicUsize::new(0);
    let faulty = FaultyFs::new();
    faulty.set_sync_file(move |_file| {
        let n = counter.fetch_add(1, Ordering::SeqCst) + 1;
        // 롤백 사본(1), 편집본(2), 본 파일에 쓴 뒤(3)
        if n == 3 { Some(other_err("write failed")) } else { None }
    });
    let hooks = EpubCoverHooks::default().with_fs(faulty);
    let result = epub_cover::add_cover_backup_with(&path, &color_jpeg("green"), &revision(&path), &trash, true, Some("book.epub"), &hooks);
    assert!(matches!(&result, Err(e) if e.is_io()), "OSError를 기대했지만 {result:?}");

    assert_eq!(fs::read(&path).unwrap(), current);
    assert_eq!(fs::read(trash.join("book.epub")).unwrap(), older);
    assert!(cover_tmp_files(tmp_path).is_empty());
}

#[tokio::test]
async fn test_rejects_a_book_that_is_not_an_epub() {
    let env = env();
    env.books.set_file_type("pdf");
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.call(URL).await;

    assert!(result.is_none() && err_has(&error, "EPUB"));
    assert_eq!(fs::read(&env.path).unwrap(), before);
}

#[tokio::test]
async fn test_reports_a_missing_book() {
    let env = env();
    env.books.set_missing("No book found by '1'");

    let (result, error) = env.call(URL).await;

    assert!(result.is_none() && err_has(&error, "No book found"));
}

#[tokio::test]
async fn test_blank_url_is_rejected_before_touching_the_file() {
    let env = env();
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.call("   ").await;

    assert!(result.is_none() && err_has(&error, "URL"));
    assert_eq!(fs::read(&env.path).unwrap(), before);
}

#[tokio::test]
async fn test_download_failure_returns_the_reason_and_keeps_the_file() {
    let env = env();
    env.fetcher.set(|_| Err(FetchError::Value("내부 주소의 이미지는 사용할 수 없습니다".to_string())));
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.call(URL).await;

    assert!(result.is_none() && err_has(&error, "내부 주소"));
    assert_eq!(fs::read(&env.path).unwrap(), before);
    assert!(env.es.updates().is_empty(), "update는 불리면 안 된다");
}

#[tokio::test]
async fn test_unexpected_io_error_hides_internal_details() {
    let env = env();
    env.fetcher.set(|_| Err(FetchError::Os("/secret/disk/path failed".to_string())));

    let (result, error) = env.call(URL).await;

    assert!(result.is_none());
    assert!(!err_has(&error, "secret"));
}

#[tokio::test]
async fn test_index_update_failure_keeps_the_saved_cover_and_warns() {
    let env = env();
    env.es.set_update_result(false);

    let (result, error) = env.call(URL).await;

    assert!(error.is_none());
    assert!(has_cover(&env.path));
    assert!(result.unwrap()["warning"].as_str().unwrap().contains("색인"));
}

#[tokio::test]
async fn test_removes_a_cover_added_by_this_tool() {
    let env = env();
    let original = fs::read(&env.path).unwrap();
    env.call(URL).await;
    let with_cover = fs::read(&env.path).unwrap();
    env.es.with(|s| s.updates.clear());

    let (result, error) = env.remove().await;

    assert!(error.is_none());
    assert!(!has_cover(&env.path));
    let names = Members::read(&env.path);
    assert!(!names.names().iter().any(|name| name.contains("tm-cover")));
    assert!(!String::from_utf8_lossy(names.get("OPS/book.opf")).contains("tm-cover"));
    // 본문은 그대로다.
    assert_eq!(Members::read(&env.path).get("OPS/chapter.xhtml"), &"<html><body>본문을 보존한다.</body></html>".as_bytes().to_vec());
    let result = result.unwrap();
    assert!(result["cover_image"].is_null() && result["file_size"].as_u64() == Some(env.size()));
    // 처음 원본은 0_trash 에 그대로 있다(표지를 넣을 때 보존한 것).
    assert!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap() == original && with_cover != original);
    let calls = env.es.updates();
    let (_, kwargs) = calls.last().expect("update 호출");
    assert_eq!(kwargs["file_size"].as_u64(), Some(env.size()));
    parse_updated_time(kwargs["updated_time"].as_str().unwrap());
}

#[tokio::test]
async fn test_remove_clears_the_stale_thumbnail_and_preview_caches() {
    let env = env();
    env.call(URL).await;
    let book_id = fs::metadata(&env.path).unwrap().ino();
    let mut stale = Vec::new();
    for (directory, name) in [(".cover_cache", format!("{book_id}.v1.jpg")), (".preview_cache", format!("{book_id}_10.epub"))] {
        fs::create_dir_all(env.dir().join(directory)).unwrap();
        fs::write(env.dir().join(directory).join(&name), b"stale").unwrap();
        stale.push(env.dir().join(directory).join(&name));
    }

    env.remove().await;

    assert!(!stale.iter().any(|item| item.exists()));
}

/// ES의 책 ID와 파일 inode가 달라도 편집 화면의 캐시를 지워야 한다.
#[tokio::test]
async fn test_cover_edit_clears_caches_for_index_id_and_current_inode() {
    for action in ["remove", "replace"] {
        let env = env();
        env.call(URL).await;
        let indexed_id: u64 = 1; // get_book(1)이 현재 파일을 가리킨다.
        let inode = fs::metadata(&env.path).unwrap().ino();
        assert_ne!(inode, indexed_id, "action={action}");
        let mut stale = Vec::new();
        let mut keep = Vec::new();
        for (directory, suffix) in [(".cover_cache", ".v1.jpg"), (".preview_cache", "_ch10_v3.epub")] {
            let cache_dir = env.dir().join(directory);
            fs::create_dir_all(&cache_dir).unwrap();
            for book_id in [indexed_id, inode] {
                let cached = cache_dir.join(format!("{book_id}{suffix}"));
                fs::write(&cached, b"stale").unwrap();
                stale.push(cached);
            }
            let unrelated = cache_dir.join(format!("{indexed_id}0{suffix}"));
            fs::write(&unrelated, b"keep").unwrap();
            keep.push(unrelated);
        }

        let (_, error) = if action == "remove" { env.remove().await } else { env.call(URL).await };

        assert!(error.is_none(), "action={action}");
        assert!(!stale.iter().any(|cached| cached.exists()), "action={action}");
        assert!(keep.iter().all(|cached| fs::read(cached).unwrap() == b"keep"), "action={action}");
    }
}

/// 기존 표지는 백업에 보존하고, 새 표지를 지운 뒤 다시 나타나지 않아야 한다.
#[tokio::test]
async fn test_replaced_publisher_cover_stays_in_the_backup() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let original = fs::read(&env.path).unwrap();
    env.call(URL).await;
    assert!(!Members::read(&env.path).contains("OPS/cover.jpg"));
    assert_eq!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap(), original);

    let (_result, error) = env.remove().await;

    assert!(error.is_none());
    assert!(!has_cover(&env.path));
    assert!(!Members::read(&env.path).names().iter().any(|name| name.contains("tm-cover")));
}

/// 교체로 빠지는 기존 표지는 새 표지 바로 뒤, 본문 앞의 도판으로 남는다.
#[tokio::test]
async fn test_replaced_cover_stays_as_an_illustration_after_the_new_cover() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let old_cover = Members::read(&env.path).get("OPS/cover.jpg").to_vec();

    let (_, error) = env.call(URL).await;

    assert!(error.is_none());
    let after = Members::read(&env.path);
    assert!(!after.contains("OPS/cover.jpg"));
    let plates: Vec<String> = after.names().into_iter().filter(|name| name.starts_with("OPS/tm-plate-") && name.ends_with(".jpg")).collect();
    assert!(plates.len() == 1 && after.get(&plates[0]) == old_cover);
    let ids = spine_idrefs(&env.path);
    assert!(ids[0].starts_with("tm-cover-page-") && ids[1].starts_with("tm-plate-page-") && ids[2] == "chapter");
    let page = xml_text(after.get(&format!("OPS/{}.xhtml", ids[1])));
    assert!(page.contains(plates[0].strip_prefix("OPS/").unwrap()));
    // 도판은 표지로 오인되지 않는다. 표지는 새 이미지다.
    let mut archive = zip::ZipArchive::new(File::open(&env.path).unwrap()).unwrap();
    assert!(BookManager::find_epub_cover_member(&mut archive).unwrap().contains("tm-cover-"));
}

#[tokio::test]
async fn test_illustration_survives_cover_removal_and_is_not_taken_for_a_cover() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    env.call(URL).await;

    let (_, error) = env.remove().await;

    assert!(error.is_none());
    assert!(!has_cover(&env.path));
    assert!(Members::read(&env.path).names().iter().any(|name| name.contains("tm-plate-")));
    assert!(spine_idrefs(&env.path)[0].starts_with("tm-plate-page-"));
}

#[tokio::test]
async fn test_each_replacement_adds_the_previous_cover_newest_first() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    env.call(URL).await;
    let first_cover = entries(&Members::read(&env.path)).iter().find(|(name, _)| name.starts_with("OPS/tm-cover-") && name.ends_with(".jpg")).map(|(_, data)| data.clone()).unwrap();
    env.fetcher.set(|_| Ok(color_jpeg("blue")));

    let (_, error) = env.call(URL).await;

    assert!(error.is_none());
    let after = Members::read(&env.path);
    assert_eq!(after.names().iter().filter(|name| name.starts_with("OPS/tm-cover-") && name.ends_with(".jpg")).count(), 1);
    let plate_images: Vec<Vec<u8>> = entries(&after).into_iter().filter(|(name, _)| name.starts_with("OPS/tm-plate-") && name.ends_with(".jpg")).map(|(_, data)| data).collect();
    assert!(plate_images.len() == 2 && plate_images.contains(&first_cover));
    let ids = spine_idrefs(&env.path);
    let kinds: Vec<&str> = ids[..3].iter().map(|i| i.split('-').nth(1).unwrap()).collect();
    assert!(kinds == ["cover", "plate", "plate"] && ids[3] == "chapter");
}

#[tokio::test]
async fn test_same_image_replacement_adds_no_duplicate_illustration() {
    let env = env();
    env.call(URL).await;
    let before = spine_idrefs(&env.path).len();

    let (_, error) = env.call(URL).await;

    assert!(error.is_none());
    assert_eq!(spine_idrefs(&env.path).len(), before);
    assert!(!Members::read(&env.path).names().iter().any(|name| name.contains("tm-plate-")));
}

#[tokio::test]
async fn test_cover_image_already_used_in_the_body_is_not_duplicated_as_an_illustration() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    attach_publisher_cover_page(&env.path, true);

    let (_, error) = env.call(URL).await;

    // 본문이 표지 이미지를 계속 쓰므로 파일은 제자리에 남는다. 이미 본문 도판이라 따로 복제하지 않는다.
    assert!(error.is_none());
    assert!(Members::read(&env.path).contains("OPS/cover.jpg"));
    assert!(!Members::read(&env.path).names().iter().any(|name| name.contains("tm-plate-")));
}

#[tokio::test]
async fn test_illustration_is_inserted_before_the_chosen_page_without_touching_the_cover() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    env.call(URL).await;
    let cover_before: Vec<(String, Vec<u8>)> = entries(&Members::read(&env.path)).into_iter().filter(|(n, _)| n.starts_with("OPS/tm-cover-")).collect();
    let original = fs::read(&env.path).unwrap();

    let (result, error) = env.illustrate("chapter").await;

    assert!(error.is_none());
    let idref = result.unwrap()["idref"].as_str().unwrap().to_string();
    assert!(idref.starts_with("tm-plate-page-"));
    let ids = spine_idrefs(&env.path);
    assert!(ids[0].starts_with("tm-cover-page-") && ids[1].starts_with("tm-plate-page-") && ids.last().unwrap() == "chapter");
    let position = |name: &str| ids.iter().position(|i| i == name).unwrap();
    assert_eq!(position(&idref), position("chapter") - 1);
    let after = Members::read(&env.path);
    assert!(cover_before.iter().all(|(name, data)| after.get(name) == data));
    assert!(has_cover(&env.path));
    assert_eq!(after.get("OPS/chapter.xhtml"), members_of(&original).get("OPS/chapter.xhtml"));
    assert!(env.dir().join("0_trash").join("book.epub").exists());
}

#[tokio::test]
async fn test_illustration_goes_in_front_of_a_book_without_a_cover() {
    let env = env();

    let (_, error) = env.illustrate("chapter").await;

    assert!(error.is_none());
    let ids = spine_idrefs(&env.path);
    assert!(ids[0].starts_with("tm-plate-page-") && ids[1] == "chapter");
    assert!(!has_cover(&env.path));
}

#[tokio::test]
async fn test_illustration_before_the_cover_page_is_refused() {
    let env = env();
    env.call(URL).await;
    let before = fs::read(&env.path).unwrap();
    let cover_page = spine_idrefs(&env.path)[0].clone();

    let (result, error) = env.illustrate(&cover_page).await;

    assert!(result.is_none() && err_has(&error, "표지 앞"));
    assert_eq!(fs::read(&env.path).unwrap(), before);
}

#[tokio::test]
async fn test_illustration_at_an_unknown_position_is_refused_and_leaves_the_file_alone() {
    let env = env();
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.illustrate("no-such-page").await;

    assert!(result.is_none() && err_has(&error, "위치"));
    assert_eq!(fs::read(&env.path).unwrap(), before);
}

#[tokio::test]
async fn test_the_same_image_can_be_inserted_at_several_places() {
    let env = env();
    let (first, _) = env.illustrate("chapter").await;
    let (second, error) = env.illustrate("chapter").await;

    assert!(error.is_none());
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_ne!(first["idref"], second["idref"]);
    let ids = spine_idrefs(&env.path);
    assert_eq!(ids[..3], [first["idref"].as_str().unwrap().to_string(), second["idref"].as_str().unwrap().to_string(), "chapter".to_string()]);
    let plates: std::collections::HashSet<String> = Members::read(&env.path).names().into_iter().filter(|n| n.starts_with("OPS/tm-plate-") && n.ends_with(".jpg")).collect();
    assert_eq!(plates.len(), 2);
}

#[tokio::test]
async fn test_illustration_from_an_uploaded_image_needs_no_download() {
    let env = env();
    env.fetcher.set(|_| panic!("다운로드 금지"));

    let (_, error) = split(env.manager.insert_epub_illustration(1, "chapter", None, Some(&b64(&color_jpeg("blue")))).await);

    assert!(error.is_none());
    assert!(spine_idrefs(&env.path)[0].starts_with("tm-plate-page-"));
}

#[tokio::test]
async fn test_illustration_requires_an_image_source() {
    let env = env();
    let before = fs::read(&env.path).unwrap();

    let (result, error) = split(env.manager.insert_epub_illustration(1, "chapter", Some("  "), None).await);

    assert!(result.is_none() && err_truthy(&error));
    assert_eq!(fs::read(&env.path).unwrap(), before);
}

#[tokio::test]
async fn test_removes_a_publisher_cover_and_preserves_the_original_backup() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.remove().await;

    assert!(error.is_none());
    assert!(result.unwrap()["cover_image"].is_null());
    assert!(!has_cover(&env.path));
    assert_eq!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap(), before);
    assert_eq!(Members::read(&env.path).get("OPS/chapter.xhtml"), &"<html><body>본문을 보존한다.</body></html>".as_bytes().to_vec());
    assert!(cover_tmp_files(env.path.parent().unwrap()).is_empty());
    assert_eq!(env.es.updates().len(), 1, "update는 한 번만 불려야 한다");
}

#[tokio::test]
async fn test_remove_refuses_a_book_without_a_cover() {
    let env = env();
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.remove().await;

    assert!(result.is_none() && err_truthy(&error));
    assert_eq!(fs::read(&env.path).unwrap(), before);
    assert!(!env.dir().join("0_trash").exists());
}

#[tokio::test]
async fn test_publisher_svg_cover_is_detached_from_spine_guide_and_toc() {
    for action in ["remove", "replace"] {
        for version in ["2.0", "3.0"] {
            let case = format!("action={action}, version={version}");
            let env = env();
            make_epub(&env.path, &EpubSpec::cover(version, true));
            attach_publisher_cover_page(&env.path, false);
            let original = fs::read(&env.path).unwrap();
            let before = Members::read(&env.path);
            let stat = fs::metadata(&env.path).unwrap();

            let (_result, error) = if action == "remove" { env.remove().await } else { env.call(URL).await };

            assert!(error.is_none(), "{case}");
            let after = Members::read(&env.path);
            let package_text = xml_text(after.get("OPS/book.opf"));
            let package = roxmltree::Document::parse(&package_text).unwrap();
            assert!(!elements(&package, "itemref").iter().any(|n| n.attribute("idref") == Some("titlepage")), "{case}");
            assert!(!elements(&package, "reference").iter().any(|n| n.attribute("type") == Some("cover") && n.attribute("href").is_some_and(|h| h.starts_with("Text/front.xhtml"))), "{case}");
            let toc_text = xml_text(after.get("OPS/toc.ncx"));
            let toc = roxmltree::Document::parse(&toc_text).unwrap();
            assert!(!elements(&toc, "content").iter().any(|n| n.attribute("src").is_some_and(|s| s.starts_with("Text/front.xhtml"))), "{case}");
            assert!(elements(&toc, "content").iter().any(|n| n.attribute("src") == Some("chapter.xhtml")), "{case}");
            // string(//navPoint/@playOrder): 문서 순서 첫 속성 값
            let first_play_order = elements(&toc, "navPoint").iter().find_map(|n| n.attribute("playOrder")).unwrap_or("");
            assert_eq!(first_play_order, "1", "{case}");
            if version == "3.0" {
                let nav_text = xml_text(after.get("OPS/nav.xhtml"));
                let nav = roxmltree::Document::parse(&nav_text).unwrap();
                let hrefs: Vec<&str> = elements(&nav, "a").iter().filter_map(|n| n.attribute("href")).collect();
                assert_eq!(hrefs, ["chapter.xhtml"], "{case}");
            }
            assert_eq!(after.get("OPS/chapter.xhtml"), before.get("OPS/chapter.xhtml"), "{case}");
            assert_eq!(after.get("OPS/style.css"), before.get("OPS/style.css"), "{case}");
            assert_eq!(fs::metadata(&env.path).unwrap().ino(), stat.ino(), "{case}");
            assert_eq!(mtime_key(&env.path), stat.mtime() * 1_000_000_000 + stat.mtime_nsec(), "{case}");
            assert_eq!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap(), original, "{case}");
            assert_eq!(has_cover(&env.path), action == "replace", "{case}");
            if action == "replace" {
                let (_, error) = env.remove().await;
                assert!(error.is_none(), "{case}");
                assert!(!has_cover(&env.path), "{case}");
            }
        }
    }
}

#[tokio::test]
async fn test_removed_cover_does_not_promote_the_next_image_only_page() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    attach_publisher_cover_page(&env.path, false);
    let chapter = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="illustration.jpg"/></body></html>"#.to_vec();

    rewrite_epub_members(&env.path, |files| {
        files.set("OPS/chapter.xhtml", chapter.clone());
        files.set("OPS/illustration.jpg", color_jpeg("green"));
        let opf = bytes_replace(files.get("OPS/book.opf"), b"</manifest>", br#"<item id="illustration" href="illustration.jpg" media-type="image/jpeg"/></manifest>"#);
        files.set("OPS/book.opf", opf);
    });

    let (_, error) = env.remove().await;

    assert!(error.is_none());
    assert!(!has_cover(&env.path));
    assert_eq!(Members::read(&env.path).get("OPS/chapter.xhtml"), &chapter);
    assert!(Members::read(&env.path).contains("OPS/illustration.jpg"));
}

/// 본문 안의 목차 페이지가 표지 페이지로 링크해도 편집을 막지 않는다. 링크만 걷어 낸다.
#[tokio::test]
async fn test_body_toc_link_to_the_cover_page_is_unlinked_instead_of_blocking_the_edit() {
    for action in ["remove", "replace"] {
        let env = env();
        make_epub(&env.path, &EpubSpec::cover("3.0", true));
        attach_publisher_cover_page(&env.path, false);
        let page =
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p><a class=\"toc-link\" href=\"Text/front.xhtml\">표지</a></p><a href=\"chapter2.xhtml\">Next</a></body></html>".as_bytes().to_vec();
        rewrite_epub_members(&env.path, |files| files.set("OPS/chapter.xhtml", page.clone()));

        let (_, error) = if action == "remove" { env.remove().await } else { env.call(URL).await };

        assert!(error.is_none(), "action={action}");
        let body = xml_text(Members::read(&env.path).get("OPS/chapter.xhtml"));
        assert!(!body.contains("href=\"Text/front.xhtml\""), "action={action}");
        assert!(body.contains("<span class=\"toc-link\">"), "action={action}");
        assert!(body.contains("href=\"chapter2.xhtml\""), "action={action}"); // 다른 링크는 그대로
        assert!(!Members::read(&env.path).contains("OPS/Text/front.xhtml"), "action={action}");
        assert_eq!(has_cover(&env.path), action == "replace", "action={action}");
    }
}

/// 링크가 아닌 참조(iframe 등)는 걷어 낼 수 없어 편집을 거부하고 파일을 그대로 둔다.
#[tokio::test]
async fn test_non_link_reference_to_the_cover_page_still_blocks_the_edit() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    attach_publisher_cover_page(&env.path, false);
    let page = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><iframe src="Text/front.xhtml"></iframe></body></html>"#.to_vec();
    rewrite_epub_members(&env.path, |files| files.set("OPS/chapter.xhtml", page.clone()));
    let before = fs::read(&env.path).unwrap();

    let (result, error) = env.call(URL).await;

    assert!(result.is_none() && err_has(&error, "본문"));
    assert_eq!(fs::read(&env.path).unwrap(), before);
}

#[tokio::test]
async fn test_removal_preserves_a_cover_image_used_in_a_chapter() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    attach_publisher_cover_page(&env.path, true);
    let original = fs::read(&env.path).unwrap();

    let (result, error) = env.remove().await;

    assert!(result.is_none() && err_has(&error, "본문"));
    assert_eq!(fs::read(&env.path).unwrap(), original);
    assert!(!env.dir().join("0_trash").exists());
}

#[tokio::test]
async fn test_remove_rejects_a_non_epub_and_a_missing_book() {
    let env = env();
    env.books.set_file_type("pdf");
    let (result, error) = env.remove().await;
    assert!(result.is_none() && err_has(&error, "EPUB"));

    env.books.set_missing("No book found by '1'");
    let (result, error) = env.remove().await;
    assert!(result.is_none() && err_has(&error, "No book found"));
}

#[tokio::test]
async fn test_remove_second_time_is_refused_because_nothing_is_left() {
    let env = env();
    env.call(URL).await;
    env.remove().await;
    let after = fs::read(&env.path).unwrap();

    let (result, error) = env.remove().await;

    assert!(result.is_none() && err_truthy(&error));
    assert_eq!(fs::read(&env.path).unwrap(), after);
}

#[tokio::test]
async fn test_remove_index_failure_keeps_the_change_and_warns() {
    let env = env();
    env.call(URL).await;
    env.es.set_update_result(false);

    let (result, error) = env.remove().await;

    assert!(error.is_none());
    assert!(result.unwrap()["warning"].as_str().unwrap().contains("색인"));
}

// ── 엔드포인트: fake manager를 끼운 앱 ──────────────────────

#[derive(Debug, Clone, PartialEq)]
enum FakeCall {
    SetCover(i64, Option<String>, Option<String>),
    RemoveCover(i64),
    Illustrate(i64, String, Option<String>, Option<String>),
}

type Outcome = (Option<Value>, Option<String>);

/// `MagicMock` book_manager. 호출 인자를 기록하고 정해 둔 `(result, error)`를 돌려준다.
struct FakeBookManager {
    calls: Mutex<Vec<FakeCall>>,
    set_epub_cover: Mutex<Outcome>,
    remove_epub_cover: Mutex<Outcome>,
    insert_epub_illustration: Mutex<Outcome>,
}

impl FakeBookManager {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
            set_epub_cover: Mutex::new((Some(json!({"file_size": 10})), None)),
            remove_epub_cover: Mutex::new((Some(json!({"file_size": 9})), None)),
            insert_epub_illustration: Mutex::new((Some(json!({"idref": "tm-plate-page-x"})), None)),
        })
    }

    fn calls(&self) -> Vec<FakeCall> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl BookManagerApi for FakeBookManager {
    async fn set_epub_cover(&self, book_id: i64, url: Option<&str>, image_data: Option<&str>) -> Result<Value, String> {
        self.calls.lock().unwrap().push(FakeCall::SetCover(book_id, url.map(String::from), image_data.map(String::from)));
        let (value, error) = self.set_epub_cover.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }

    async fn remove_epub_cover(&self, book_id: i64) -> Result<Value, String> {
        self.calls.lock().unwrap().push(FakeCall::RemoveCover(book_id));
        let (value, error) = self.remove_epub_cover.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }

    async fn insert_epub_illustration(&self, book_id: i64, before_idref: &str, url: Option<&str>, image_data: Option<&str>) -> Result<Value, String> {
        self.calls.lock().unwrap().push(FakeCall::Illustrate(book_id, before_idref.to_string(), url.map(String::from), image_data.map(String::from)));
        let (value, error) = self.insert_epub_illustration.lock().unwrap().clone();
        error.map_or(Ok(value.unwrap_or_default()), Err)
    }
}

/// require_admin을 통과시키고 book_manager를 fake로 바꾼 앱의 클라이언트.
fn endpoint_client() -> (TestClient, Arc<FakeBookManager>) {
    common::setup_env();
    let fake = FakeBookManager::new();
    let app = app_with(AppDeps::new(AuthMode::Fixed(json!({"role": "admin"}))).with_book_manager(fake.clone()));
    (TestClient::with_app(app), fake)
}

mod test_epub_cover_endpoint {
    use super::*;

    #[tokio::test]
    async fn test_success() {
        let (http, fake) = endpoint_client();
        let response = http.post_json("/epub-cover/1", &json!({"url": URL})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "success", "result": {"file_size": 10}}));
        assert_eq!(fake.calls(), [FakeCall::SetCover(1, Some(URL.to_string()), None)]);
    }

    #[tokio::test]
    async fn test_failure_is_reported_in_the_body() {
        let (http, fake) = endpoint_client();
        *fake.set_epub_cover.lock().unwrap() = (None, Some("EPUB이 변경되었습니다. 목록을 새로 불러오세요".to_string()));
        let body = http.post_json("/epub-cover/1", &json!({"url": URL})).await.json();
        assert_eq!(body, json!({"status": "failure", "error": "EPUB이 변경되었습니다. 목록을 새로 불러오세요"}));
    }

    #[tokio::test]
    async fn test_empty_or_oversized_url_is_a_validation_error() {
        let (http, _) = endpoint_client();
        assert_eq!(http.post_json("/epub-cover/1", &json!({"url": ""})).await.status, 422);
        assert_eq!(http.post_json("/epub-cover/1", &json!({"url": "x".repeat(5000)})).await.status, 422);
    }

    #[tokio::test]
    async fn test_requires_admin() {
        let response = TestClient::new().post_json("/epub-cover/1", &json!({"url": URL})).await;
        assert!([401, 403].contains(&response.status), "status={}", response.status);
    }

    #[tokio::test]
    async fn test_comics_router_exposes_the_same_endpoint() {
        let response = TestClient::new().post_json("/comics/epub-cover/1", &json!({"url": URL})).await;
        assert!([401, 403].contains(&response.status), "status={}", response.status);
    }

    #[tokio::test]
    async fn test_illustration_success() {
        let (http, fake) = endpoint_client();
        let response = http.post_json("/epub-illustration/1", &json!({"url": URL, "before_idref": "chapter"})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "success", "result": {"idref": "tm-plate-page-x"}}));
        assert_eq!(fake.calls(), [FakeCall::Illustrate(1, "chapter".to_string(), Some(URL.to_string()), None)]);
    }

    #[tokio::test]
    async fn test_illustration_failure_is_reported_in_the_body() {
        let (http, fake) = endpoint_client();
        *fake.insert_epub_illustration.lock().unwrap() = (None, Some("삽화를 넣을 위치를 찾을 수 없습니다".to_string()));
        let body = http.post_json("/epub-illustration/1", &json!({"url": URL, "before_idref": "nope"})).await.json();
        assert_eq!(body, json!({"status": "failure", "error": "삽화를 넣을 위치를 찾을 수 없습니다"}));
    }

    #[tokio::test]
    async fn test_illustration_needs_a_position_and_exactly_one_image_source() {
        let (http, _) = endpoint_client();
        assert_eq!(http.post_json("/epub-illustration/1", &json!({"url": URL})).await.status, 422);
        assert_eq!(http.post_json("/epub-illustration/1", &json!({"url": URL, "before_idref": ""})).await.status, 422);
        assert_eq!(http.post_json("/epub-illustration/1", &json!({"before_idref": "chapter"})).await.status, 422);
        assert_eq!(http.post_json("/epub-illustration/1", &json!({"url": URL, "image_data": "QQ==", "before_idref": "chapter"})).await.status, 422);
    }

    #[tokio::test]
    async fn test_illustration_requires_admin() {
        let response = TestClient::new().post_json("/epub-illustration/1", &json!({"url": URL, "before_idref": "chapter"})).await;
        assert!([401, 403].contains(&response.status), "status={}", response.status);
    }

    #[tokio::test]
    async fn test_delete_success() {
        let (http, fake) = endpoint_client();
        let response = http.delete("/epub-cover/1").await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "success", "result": {"file_size": 9}}));
        assert_eq!(fake.calls(), [FakeCall::RemoveCover(1)]);
    }

    #[tokio::test]
    async fn test_delete_failure_is_reported_in_the_body() {
        let (http, fake) = endpoint_client();
        *fake.remove_epub_cover.lock().unwrap() = (None, Some("이 EPUB의 표지는 이 도구가 넣은 것이 아니라 삭제할 수 없습니다".to_string()));
        let body = http.delete("/epub-cover/1").await.json();
        assert_eq!(body, json!({"status": "failure", "error": "이 EPUB의 표지는 이 도구가 넣은 것이 아니라 삭제할 수 없습니다"}));
    }

    #[tokio::test]
    async fn test_delete_requires_admin_on_books_and_comics() {
        let http = TestClient::new();
        assert!([401, 403].contains(&http.delete("/epub-cover/1").await.status));
        assert!([401, 403].contains(&http.delete("/comics/epub-cover/1").await.status));
    }
}

// ── 올린 이미지(image_data)로 표지 넣기 ──

#[tokio::test]
async fn test_adds_a_cover_from_an_uploaded_image_without_fetching() {
    let env = env();
    env.fetcher.set(|_| panic!("올린 이미지는 URL 을 내려받지 않는다"));

    let (result, error) = split(env.manager.set_epub_cover(1, None, Some(&b64(&color_jpeg("red")))).await);

    assert!(error.is_none());
    assert!(has_cover(&env.path));
    assert_eq!(result.unwrap()["file_size"].as_u64(), Some(env.size()));
    assert_eq!(env.es.updates().len(), 1, "update는 한 번만 불려야 한다");
}

#[tokio::test]
async fn test_uploaded_cover_replaces_an_existing_one_and_keeps_the_original_in_0_trash() {
    let env = env();
    make_epub(&env.path, &EpubSpec::cover("3.0", true));
    let original = fs::read(&env.path).unwrap();

    let (_, error) = env.upload(&b64(&color_jpeg("blue"))).await;

    assert!(error.is_none());
    assert_ne!(fs::read(&env.path).unwrap(), original);
    assert_eq!(fs::read(env.dir().join("0_trash").join("book.epub")).unwrap(), original);
    assert!(!env.dir().join(".cover_backups").exists());
}

/// 두 번째는 base64 이지만 이미지가 아니다
#[tokio::test]
async fn test_unreadable_uploaded_cover_is_rejected_without_touching_the_epub() {
    for data in ["***not-base64***", "bm90IGFuIGltYWdl"] {
        let env = env();
        let original = fs::read(&env.path).unwrap();

        let (result, error) = env.upload(data).await;

        assert!(result.is_none() && err_truthy(&error), "data={data}");
        assert_eq!(fs::read(&env.path).unwrap(), original, "data={data}");
        assert!(!env.dir().join("0_trash").exists(), "data={data}");
        assert!(env.es.updates().is_empty(), "data={data}: update는 불리면 안 된다");
    }
}

#[tokio::test]
async fn test_without_url_or_image_data_nothing_is_changed() {
    let env = env();
    let original = fs::read(&env.path).unwrap();

    let (result, error) = split(env.manager.set_epub_cover(1, None, None).await);

    assert!(result.is_none() && err_has(&error, "비어"));
    assert_eq!(fs::read(&env.path).unwrap(), original);
}

mod test_epub_cover_upload_endpoint {
    use super::*;

    fn upload_client() -> (TestClient, Arc<FakeBookManager>) {
        endpoint_client()
    }

    #[tokio::test]
    async fn test_image_data_is_passed_to_the_manager() {
        let (http, fake) = upload_client();
        let response = http.post_json("/epub-cover/1", &json!({"image_data": "QUJD"})).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status": "success", "result": {"file_size": 10}}));
        assert_eq!(fake.calls(), [FakeCall::SetCover(1, None, Some("QUJD".to_string()))]);
    }

    #[tokio::test]
    async fn test_requires_exactly_one_source() {
        for body in [json!({}), json!({"url": URL, "image_data": "QUJD"}), json!({"image_data": ""})] {
            let (http, fake) = upload_client();
            assert_eq!(http.post_json("/epub-cover/1", &body).await.status, 422, "body={body}");
            assert!(fake.calls().is_empty(), "body={body}: set_epub_cover가 호출되면 안 된다");
        }
    }
}

// ── epub_cover 방어 경로: 표지 분리, 교체 전 표지 보존, 검증 실패 ──

/// SVG 표지 페이지(Text/front.xhtml)를 가진 EPUB 3. `files`로 멤버를 덮어쓴다.
fn publisher_cover_epub(tmp_path: &Path, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let path = make_epub(&tmp_path.join("book.epub"), &EpubSpec::cover("3.0", true));
    attach_publisher_cover_page(&path, false);
    if !files.is_empty() {
        rewrite_epub_members(&path, |members_| {
            for (name, data) in files {
                members_.set(name, data.clone());
            }
        });
    }
    path
}

fn remove_directly(path: &Path, tmp_path: &Path) -> Result<Value, CoverError> {
    epub_cover::remove_cover(path, &revision(path), &tmp_path.join(".cover_backups"))
}

fn remove_directly_with(path: &Path, tmp_path: &Path, hooks: &EpubCoverHooks) -> Result<Value, CoverError> {
    epub_cover::remove_cover_with(path, &revision(path), &tmp_path.join(".cover_backups"), hooks)
}

#[test]
fn test_member_path_rejects_external_and_fragment_only_references() {
    assert_eq!(epub_cover::member_path("OPS", "https://example.com/a.jpg"), "");
    assert_eq!(epub_cover::member_path("OPS", "#top"), "");
    assert_eq!(epub_cover::member_path("OPS", ""), "");
    assert_eq!(epub_cover::member_path("OPS", "../../x.jpg"), "");
    assert_eq!(epub_cover::member_path("OPS/Text", "../a%20b.jpg"), "OPS/a b.jpg");
}

#[test]
fn test_manifest_page_missing_from_the_archive_is_skipped_while_detaching_the_cover() {
    let tmp = tempfile::tempdir().unwrap();
    let path = publisher_cover_epub(tmp.path(), &[]);
    rewrite_epub_members(&path, |files| {
        let opf = bytes_replace(files.get("OPS/book.opf"), b"</manifest>", br#"<item id="ghost" href="ghost.xhtml" media-type="application/xhtml+xml"/></manifest>"#);
        files.set("OPS/book.opf", opf);
    });

    remove_directly(&path, tmp.path()).unwrap();

    assert!(!has_cover(&path));
    assert!(!Members::read(&path).contains("OPS/Text/front.xhtml"));
}

#[test]
fn test_oversized_body_page_stops_cover_removal_without_touching_the_book() {
    let tmp = tempfile::tempdir().unwrap();
    let mut chapter = b"<html><body>".to_vec();
    chapter.extend(std::iter::repeat_n(b'x', 6000));
    chapter.extend_from_slice(b"</body></html>");
    let path = publisher_cover_epub(tmp.path(), &[("OPS/chapter.xhtml", chapter)]);
    let original = fs::read(&path).unwrap();
    let hooks = EpubCoverHooks::default().with_limits(Limits { max_xml_bytes: 5000, ..Limits::default() });

    assert_cover_error(remove_directly_with(&path, tmp.path(), &hooks), "너무 큽니다");

    assert_eq!(fs::read(&path).unwrap(), original);
    assert!(!tmp.path().join(".cover_backups").exists());
}

fn add_stylesheet_to_manifest(files: &mut Members) {
    let opf = bytes_replace(files.get("OPS/book.opf"), b"</manifest>", br#"<item id="css" href="style.css" media-type="text/css"/></manifest>"#);
    files.set("OPS/book.opf", opf);
}

#[test]
fn test_stylesheet_that_does_not_use_the_cover_page_is_kept_as_is() {
    let tmp = tempfile::tempdir().unwrap();
    let path = publisher_cover_epub(tmp.path(), &[]);
    rewrite_epub_members(&path, add_stylesheet_to_manifest);
    let css = Members::read(&path).get("OPS/style.css").to_vec();

    remove_directly(&path, tmp.path()).unwrap();

    assert!(!has_cover(&path));
    assert_eq!(Members::read(&path).get("OPS/style.css"), &css);
}

#[test]
fn test_stylesheet_that_uses_the_cover_page_blocks_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = publisher_cover_epub(tmp.path(), &[]);
    rewrite_epub_members(&path, |files| {
        add_stylesheet_to_manifest(files);
        files.set("OPS/style.css", br#"body { background: url("Text/front.xhtml"); }"#.to_vec());
    });
    let original = fs::read(&path).unwrap();

    assert_cover_error(remove_directly(&path, tmp.path()), "표지 페이지가 본문에서도 쓰여");

    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn test_xml_comment_in_a_body_page_does_not_break_unlinking_the_cover_page() {
    let tmp = tempfile::tempdir().unwrap();
    let page = br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><!-- note --><p><a href="Text/front.xhtml">cover</a></p></body></html>"#.to_vec();
    let path = publisher_cover_epub(tmp.path(), &[("OPS/chapter.xhtml", page)]);

    remove_directly(&path, tmp.path()).unwrap();

    let body = xml_text(Members::read(&path).get("OPS/chapter.xhtml"));
    assert!(body.contains("<!-- note -->"));
    assert!(!body.contains("front.xhtml"));
    assert!(body.contains("<span>cover</span>"));
}

#[test]
fn test_ncx_child_entries_move_up_when_their_cover_parent_is_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let nav_map = concat!(
        r#"<navPoint id="cover" playOrder="1"><navLabel><text>Cover</text></navLabel><content src="Text/front.xhtml#cover"/>"#,
        r#"<navPoint id="child" playOrder="2"><navLabel><text>Child</text></navLabel><content src="chapter.xhtml"/></navPoint></navPoint>"#
    );
    let path = publisher_cover_epub(tmp.path(), &[("OPS/toc.ncx", ncx_doc(nav_map, "").into_bytes())]);

    remove_directly(&path, tmp.path()).unwrap();

    let text = xml_text(Members::read(&path).get("OPS/toc.ncx"));
    let toc = roxmltree::Document::parse(&text).unwrap();
    // /*/navMap/navPoint: 루트 바로 아래 navMap의 직계 navPoint
    let points: Vec<_> = toc
        .root_element()
        .children()
        .filter(|n| n.is_element() && n.tag_name().name() == "navMap")
        .flat_map(|m| m.children().filter(|n| n.is_element() && n.tag_name().name() == "navPoint").collect::<Vec<_>>())
        .collect();
    assert_eq!(points.iter().map(|p| p.attribute("id").unwrap()).collect::<Vec<_>>(), ["child"]);
    assert_eq!(points[0].attribute("playOrder"), Some("1"));
    assert!(!toc.descendants().any(|n| n.is_element() && n.attribute("src").is_some_and(|s| s.starts_with("Text/front.xhtml"))));
}

#[test]
fn test_nav_entry_with_sub_entries_keeps_its_place_as_plain_text() {
    let tmp = tempfile::tempdir().unwrap();
    let body = r#"<ol><li><a href="Text/front.xhtml#cover">Cover</a><ol><li><a href="chapter.xhtml">Chapter</a></li></ol></li></ol>"#;
    let path = publisher_cover_epub(tmp.path(), &[("OPS/nav.xhtml", nav_doc(body).into_bytes())]);

    remove_directly(&path, tmp.path()).unwrap();

    let text = xml_text(Members::read(&path).get("OPS/nav.xhtml"));
    let nav = roxmltree::Document::parse(&text).unwrap();
    let hrefs: Vec<&str> = elements(&nav, "a").iter().filter_map(|n| n.attribute("href")).collect();
    assert_eq!(hrefs, ["chapter.xhtml"]);
    // string(//li/span): 문서 순서 첫 li의 자식 span의 텍스트
    let span_text: String = elements(&nav, "li")
        .iter()
        .flat_map(|li| li.children().filter(|c| c.is_element() && c.tag_name().name() == "span").collect::<Vec<_>>())
        .next()
        .map(|span| span.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect())
        .unwrap_or_default();
    assert_eq!(span_text, "Cover");
}

#[test]
fn test_ncx_page_list_reference_to_the_cover_cannot_be_cleaned_up_safely() {
    let tmp = tempfile::tempdir().unwrap();
    let nav_map = r#"<navPoint id="chapter" playOrder="1"><navLabel><text>Chapter</text></navLabel><content src="chapter.xhtml"/></navPoint>"#;
    let page_list = r#"<pageList><pageTarget id="p1" type="normal" value="1"><navLabel><text>1</text></navLabel><content src="Text/front.xhtml"/></pageTarget></pageList>"#;
    let path = publisher_cover_epub(tmp.path(), &[("OPS/toc.ncx", ncx_doc(nav_map, page_list).into_bytes())]);
    let original = fs::read(&path).unwrap();

    assert_cover_error(remove_directly(&path, tmp.path()), "목차 참조");

    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn test_replacing_a_cover_whose_image_is_not_in_the_manifest_keeps_no_plate() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));

    rewrite_epub_members(&path, |files| {
        let opf = bytes_replace(files.get("OPS/book.opf"), b"</package>", br#"<guide><reference type="cover" href="front.xhtml"/></guide></package>"#);
        files.set("OPS/book.opf", opf);
        files.set("OPS/front.xhtml", br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><img src="pic.jpg"/></body></html>"#.to_vec());
        files.set("OPS/pic.jpg", red_jpeg());
    });
    assert!(has_cover(&path));

    epub_cover::add_cover(&path, &color_jpeg("blue"), &revision(&path), &tmp.path().join(".cover_backups"), true).unwrap();

    assert!(!Members::read(&path).names().iter().any(|name| name.contains("tm-plate")));
    assert!(Members::read(&path).names().iter().any(|name| name.starts_with("OPS/tm-cover-")));
}

#[test]
fn test_illustration_needs_a_spine_and_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
    rewrite_epub_members(&path, |files| {
        let opf = bytes_replace(files.get("OPS/book.opf"), br#"<spine><itemref idref="chapter"/></spine>"#, b"");
        files.set("OPS/book.opf", opf);
    });
    let original = fs::read(&path).unwrap();

    assert_cover_error(epub_cover::add_illustration(&path, &red_jpeg(), &revision(&path), &tmp.path().join(".cover_backups"), "chapter"), "manifest 또는 spine");

    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn test_illustration_gives_up_when_every_numbered_id_is_taken() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
    let image = red_jpeg();
    let digest = &sha256_hex(&image)[..16];
    let items: String = (1..1000)
        .map(|serial| {
            let id = if serial == 1 { format!("tm-plate-{digest}") } else { format!("tm-plate-{digest}-{serial}") };
            format!(r#"<item id="{id}" href="p{serial}.jpg" media-type="image/jpeg"/>"#)
        })
        .collect();
    rewrite_epub_members(&path, |files| {
        let opf = bytes_replace(files.get("OPS/book.opf"), b"</manifest>", format!("{items}</manifest>").as_bytes());
        files.set("OPS/book.opf", opf);
    });

    let source = File::open(&path).unwrap();
    assert_cover_error(epub_cover::stage_illustration(&source, &tmp.path().join("staged.epub"), &image, "chapter"), "삽화 ID");
}

#[test]
fn test_corrupt_staged_epub_is_rejected_for_illustration_and_removal() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
    let original = fs::read(&path).unwrap();
    let hooks = EpubCoverHooks::default().with_testzip(Arc::new(|_path: &Path| Some("OPS/chapter.xhtml".to_string())));

    assert_cover_error(epub_cover::add_illustration_with(&path, &red_jpeg(), &revision(&path), &tmp.path().join(".cover_backups"), "chapter", &hooks), "검증에 실패");
    assert_eq!(fs::read(&path).unwrap(), original);

    let removal_dir = tmp.path().join("removal");
    let covered = publisher_cover_epub(&removal_dir, &[]);
    let original = fs::read(&covered).unwrap();
    assert_cover_error(remove_directly_with(&covered, &removal_dir, &hooks), "검증에 실패");
    assert_eq!(fs::read(&covered).unwrap(), original);
}

#[test]
fn test_removal_is_rejected_when_the_staged_epub_still_reports_a_cover() {
    let tmp = tempfile::tempdir().unwrap();
    let path = publisher_cover_epub(tmp.path(), &[]);
    let original = fs::read(&path).unwrap();
    let rev = revision(&path);
    let hooks = EpubCoverHooks::default().with_inspect_archive(Arc::new(|_inspected: Value| json!({"has_cover": true})));

    assert_cover_error(epub_cover::remove_cover_with(&path, &rev, &tmp.path().join(".cover_backups"), &hooks), "표지를 삭제한 EPUB의 검증에 실패");

    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn test_cover_removal_needs_manifest_metadata_and_spine() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", true));
    rewrite_epub_members(&path, |files| {
        let opf = bytes_replace(&bytes_replace(files.get("OPS/book.opf"), b"<spine>", b"<x>"), b"</spine>", b"</x>");
        files.set("OPS/book.opf", opf);
    });
    let original = fs::read(&path).unwrap();

    assert_cover_error(remove_directly(&path, tmp.path()), "manifest, metadata 또는 spine");
    assert_eq!(fs::read(&path).unwrap(), original);

    let source = File::open(&path).unwrap();
    assert_cover_error(epub_cover::stage_cover_removal(&source, &tmp.path().join("staged.epub")), "manifest, metadata 또는 spine");
}

#[test]
fn test_unnamed_backup_collision_is_an_error_not_a_silent_reuse() {
    let tmp = tempfile::tempdir().unwrap();
    let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", false));
    let original = fs::read(&path).unwrap();
    let rev = revision(&path);
    let hooks = EpubCoverHooks::default().with_uuid_hex(Arc::new(|| "f".repeat(32)));
    let backup_dir = tmp.path().join(".cover_backups");
    fs::create_dir(&backup_dir).unwrap();
    let existing = backup_dir.join(format!("{}-{}.epub", sha256_hex(format!("{}:{}", path.display(), rev).as_bytes()), "f".repeat(32)));
    fs::write(&existing, b"older backup").unwrap();

    let result = epub_cover::add_cover_with(&path, &red_jpeg(), &rev, &backup_dir, false, &hooks);
    match result {
        Err(e) if e.is_io() => assert_eq!(e.io_kind(), Some(std::io::ErrorKind::AlreadyExists)),
        other => panic!("FileExistsError를 기대했지만 {other:?}"),
    }

    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(fs::read(&existing).unwrap(), b"older backup");
}
