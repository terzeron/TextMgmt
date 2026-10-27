//! Python tests/test_cover_status.py의 표지 판정, MySQL cache, 목록 조회 계약.

mod common;
#[path = "common/mysql_fixture.rs"]
mod mysql_fixture;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use backend::book::Book as CoverBook;
use backend::book_manager::{BookManager, BookManagerDeps, CoverBooksPager, CoverPage, CoverPageOpts};
use backend::cover_status::{CoverStatusCache, STANDARD_COVER_SIZES, has_official_cover, pdf_first_image_size};
use common::{
    epub::{EpubSpec, make_epub},
    es_fake::FakeEsManager,
    image::solid,
    pdf::{write_blank_pdf, write_image_pdf},
};
use serde_json::json;

static LOCK: Mutex<()> = Mutex::new(());

#[test]
fn cache_invalidates_when_file_metadata_changes_and_does_not_store_errors() {
    mysql_fixture::mysql_container();
    let _db_guard = mysql_fixture::db_lock();
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("file.pdf");
    std::fs::write(&file, b"same").unwrap();
    let cache = CoverStatusCache::new();
    assert!(!cache.get_or_compute(&file, &|| Ok(false)).unwrap());
    let now = std::fs::metadata(&file).unwrap().modified().unwrap();
    std::fs::File::options().write(true).open(&file).unwrap().set_modified(now + Duration::from_secs(1)).unwrap();
    assert!(cache.get_or_compute(&file, &|| Err("decode failed".into())).is_err());
    assert!(cache.get_or_compute(&file, &|| Ok(true)).unwrap());
    assert!(CoverStatusCache::new().get_or_compute(&file, &|| panic!("persisted row should be reused")).unwrap());
}

#[test]
fn cache_rejects_a_missing_file_before_opening_mysql() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = CoverStatusCache::new();
    assert!(cache.get_or_compute(&tmp.path().join("missing"), &|| panic!("no file")).is_err());
}

#[test]
fn pdf_uses_first_drawn_image_not_first_resource() {
    use lopdf::{Object, Stream, dictionary};
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("order.pdf"), Some((480, 623)));
    common::pdf::edit_pdf(&pdf, |doc| {
        let unused = doc.add_object(Stream::new(dictionary! {"Type"=>"XObject","Subtype"=>"Image","Width"=>11,"Height"=>22}, vec![]));
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        let objects = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap().get_mut(b"XObject").unwrap().as_dict_mut().unwrap();
        let drawn = objects.get(b"Im0").unwrap().clone();
        *objects = dictionary! {"FirstButUnused"=>Object::Reference(unused),"Im0"=>drawn};
    });
    assert_eq!(pdf_first_image_size(&pdf, &LOCK).unwrap(), Some((480, 623)));
}

#[test]
fn inline_image_counts_without_xobject_resources() {
    use lopdf::{
        Object, Stream,
        content::{Content, Operation},
        dictionary,
    };
    let tmp = tempfile::tempdir().unwrap();
    let pdf = tmp.path().join("inline.pdf");
    let mut fixture = common::pdf::PdfBuilder::new();
    let page = fixture.add_page(480, 623);
    let image = Stream::new(dictionary! {"W"=>480,"H"=>623,"BPC"=>8,"CS"=>"RGB"}, vec![42; 480 * 623 * 3]);
    let data = Content { operations: vec![Operation::new("BI", vec![Object::Stream(image)])] }.encode().unwrap();
    fixture.content(page, &data);
    fixture.save(&pdf);
    assert_eq!(pdf_first_image_size(&pdf, &LOCK).unwrap(), Some((480, 623)));
    assert!(has_official_cover(&pdf, "pdf", &LOCK).unwrap());
}

#[test]
fn pdf_cover_status_handles_tree_errors_and_inherited_resources() {
    use lopdf::Object;

    let tmp = tempfile::tempdir().unwrap();
    let inherited = make_pdf(&tmp.path().join("inherited.pdf"), Some((480, 623)));
    common::pdf::edit_pdf(&inherited, |doc| {
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        let resources = page.remove(b"Resources").unwrap();
        let parent = page.get(b"Parent").unwrap().as_reference().unwrap();
        doc.get_object_mut(parent).unwrap().as_dict_mut().unwrap().set("Resources", resources);
    });
    assert_eq!(pdf_first_image_size(&inherited, &LOCK).unwrap(), Some((480, 623)));

    let cyclic = make_pdf(&tmp.path().join("cycle.pdf"), None);
    common::pdf::edit_pdf(&cyclic, |doc| {
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        page.remove(b"Resources");
        page.set("Parent", Object::Reference(page_id));
    });
    assert!(pdf_first_image_size(&cyclic, &LOCK).unwrap_err().contains("순환"));

    let no_tree_links = make_pdf(&tmp.path().join("no-tree-links.pdf"), None);
    common::pdf::edit_pdf(&no_tree_links, |doc| {
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        page.remove(b"Resources");
        page.remove(b"Parent");
        page.set("Contents", Object::Null);
    });
    assert_eq!(pdf_first_image_size(&no_tree_links, &LOCK).unwrap(), None);
}

#[test]
fn pdf_cover_status_skips_non_images_and_rejects_zero_sized_inline_images() {
    use lopdf::{
        Object, Stream,
        content::{Content, Operation},
        dictionary,
    };

    let tmp = tempfile::tempdir().unwrap();
    let non_image = make_pdf(&tmp.path().join("form.pdf"), Some((480, 623)));
    common::pdf::edit_pdf(&non_image, |doc| {
        let page_id = doc.get_pages()[&1];
        let page = doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap();
        let resources = page.get_mut(b"Resources").unwrap().as_dict_mut().unwrap();
        let xobjects = resources.get_mut(b"XObject").unwrap().as_dict_mut().unwrap();
        let image_id = xobjects.get(b"Im0").unwrap().as_reference().unwrap();
        doc.get_object_mut(image_id).unwrap().as_stream_mut().unwrap().dict.set("Subtype", "Form");
        let content = doc.add_object(Stream::new(dictionary! {}, b"/Im0 Do".to_vec()));
        doc.get_object_mut(page_id).unwrap().as_dict_mut().unwrap().set("Contents", content);
    });
    assert_eq!(pdf_first_image_size(&non_image, &LOCK).unwrap(), None);

    let zero_image = tmp.path().join("zero-inline.pdf");
    let mut fixture = common::pdf::PdfBuilder::new();
    let page = fixture.add_page(10, 10);
    let image = Stream::new(dictionary! {"W"=>0,"H"=>10,"BPC"=>8,"CS"=>"RGB"}, vec![]);
    fixture.content(page, &Content { operations: vec![Operation::new("BI", vec![Object::Stream(image)])] }.encode().unwrap());
    fixture.save(&zero_image);
    assert!(pdf_first_image_size(&zero_image, &LOCK).unwrap_err().contains("0입니다"));
}

#[test]
fn default_cover_cache_can_be_constructed_without_connecting() {
    let _cache = CoverStatusCache::default();
}

fn make_pdf(path: &Path, size: Option<(u32, u32)>) -> PathBuf {
    match size {
        None => write_blank_pdf(path, 100, 100),
        Some((w, h)) => write_image_pdf(path, &[solid(w, h, [30, 30, 200])]),
    }
    path.to_path_buf()
}

#[test]
fn test_pdf_with_standard_cover_size_is_covered() {
    let mut sizes = STANDARD_COVER_SIZES.to_vec();
    sizes.sort();
    for size in sizes {
        let tmp = tempfile::tempdir().unwrap();
        let pdf = make_pdf(&tmp.path().join("a.pdf"), Some(size));

        assert_eq!(pdf_first_image_size(&pdf, &LOCK).unwrap(), Some(size), "size={size:?}");
        assert!(has_official_cover(&pdf, "pdf", &LOCK).unwrap(), "size={size:?}");
    }
}

#[test]
fn test_pdf_with_other_first_image_is_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("a.pdf"), Some((690, 1600)));

    assert!(!has_official_cover(&pdf, "pdf", &LOCK).unwrap());
}

#[test]
fn test_pdf_without_image_is_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("a.pdf"), None);

    assert_eq!(pdf_first_image_size(&pdf, &LOCK).unwrap(), None);
    assert!(!has_official_cover(&pdf, "pdf", &LOCK).unwrap());
}

#[test]
fn test_unsupported_type_is_not_reported_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    assert!(has_official_cover(&tmp.path().join("a.txt"), "txt", &LOCK).unwrap());
}

#[test]
fn test_cache_reuses_result_until_file_changes() {
    mysql_fixture::mysql_container();
    let _db_guard = mysql_fixture::db_lock();
    let tmp = tempfile::tempdir().unwrap();
    let pdf = make_pdf(&tmp.path().join("a.pdf"), Some((690, 1600)));
    let cache = CoverStatusCache::new();
    let calls = AtomicUsize::new(0);

    let compute = || {
        calls.fetch_add(1, Ordering::SeqCst);
        has_official_cover(&pdf, "pdf", &LOCK)
    };

    assert!(!cache.get_or_compute(&pdf, &compute).unwrap());
    assert!(!cache.get_or_compute(&pdf, &compute).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    make_pdf(&pdf, Some((480, 623)));
    assert!(cache.get_or_compute(&pdf, &compute).unwrap());
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

/// 구 `BookManager::without_es(path)`. ES는 쓰지 않으므로 빈 `FakeEsManager`를 넘긴다.
fn deps(tmp: &Path) -> BookManagerDeps {
    BookManagerDeps::new(tmp.to_path_buf(), FakeEsManager::new())
}

fn make_manager(tmp: &Path) -> BookManager {
    BookManager::with_deps(deps(tmp))
}

fn book(name: &str, tmp: &Path, size: (u32, u32)) -> CoverBook {
    common::setup_env();
    CoverBook::new(1, &json!({"category":"", "title":name, "author":"", "file_path":make_pdf(&tmp.join(name),Some(size)), "file_type":"pdf", "file_size":0, "updated_time":"2026-01-01T00:00:00"}), 0.0)
}

fn opts(size: Option<usize>, cursor: Option<&str>) -> CoverPageOpts {
    CoverPageOpts { size: size.unwrap_or(60), cursor: cursor.map(str::to_string), ..Default::default() }
}

/// 스크립트한 페이지를 차례로 돌려주고, 받은 커서를 기록하는 pager. Python `AsyncMock(side_effect=[...])`.
fn scripted(pages: Vec<CoverPage>) -> (CoverBooksPager, Arc<Mutex<Vec<Option<String>>>>) {
    let queue = Arc::new(Mutex::new(VecDeque::from(pages)));
    let cursors = Arc::new(Mutex::new(Vec::new()));
    let (q, c) = (queue.clone(), cursors.clone());
    let pager: CoverBooksPager = Arc::new(move |opts: CoverPageOpts| {
        c.lock().unwrap().push(opts.cursor);
        let page = q.lock().unwrap().pop_front().expect("스크립트한 페이지를 모두 썼다");
        Box::pin(async move { page })
    });
    (pager, cursors)
}

fn names(items: &[(CoverBook, String)]) -> Vec<&str> {
    items.iter().map(|entry| entry.0.title.as_str()).collect()
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_uncovered_page_collects_across_scan_pages_and_keeps_scan_cursor() {
    mysql_fixture::mysql_container();
    let _db_guard = mysql_fixture::db_lock();
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let plain1 = book("plain1.pdf", tmp.path(), (690, 1600));
    let plain2 = book("plain2.pdf", tmp.path(), (690, 1600));
    let (pager, cursors) = scripted(vec![(vec![(covered, String::new()), (plain1, String::new())], 3, Some("c1".into()), None), (vec![(plain2, String::new())], 3, Some("c2".into()), None)]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(pager));

    let (items, total, next_cursor, error) = manager.get_uncovered_books_paged(opts(Some(2), None)).await;

    assert_eq!(error, None);
    assert_eq!(names(&items), ["plain1.pdf", "plain2.pdf"]);
    assert_eq!(total, None);
    assert_eq!(next_cursor.as_deref(), Some("c2"));
    assert_eq!(cursors.lock().unwrap()[1].as_deref(), Some("c1"));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_uncovered_page_ends_when_list_ends() {
    mysql_fixture::mysql_container();
    let _db_guard = mysql_fixture::db_lock();
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let (pager, _cursors) = scripted(vec![(vec![(covered, String::new())], 1, None, None)]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(pager));

    let (items, total, next_cursor, error) = manager.get_uncovered_books_paged(opts(Some(5), None)).await;

    assert!(items.is_empty());
    assert_eq!((total, next_cursor, error), (None, None, None));
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_uncovered_page_stops_at_budget_even_without_items() {
    mysql_fixture::mysql_container();
    let _db_guard = mysql_fixture::db_lock();
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let (pager, cursors) = scripted(vec![(vec![(covered.clone(), String::new())], 9, Some("c1".into()), None), (vec![(covered, String::new())], 9, Some("c2".into()), None)]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_uncovered_scan_budget_seconds(-1.0).with_cover_books_pager(pager));

    let (items, _total, next_cursor, error) = manager.get_uncovered_books_paged(opts(Some(5), None)).await;

    assert!(items.is_empty());
    assert_eq!((next_cursor.as_deref(), error), (Some("c1"), None));
    assert_eq!(cursors.lock().unwrap().len(), 1);
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn test_uncovered_page_stops_at_page_cap_and_returns_resume_cursor() {
    mysql_fixture::mysql_container();
    let _db_guard = mysql_fixture::db_lock();
    let tmp = tempfile::tempdir().unwrap();
    let covered = book("covered.pdf", tmp.path(), (480, 623));
    let (pager, cursors) = scripted(vec![
        (vec![(covered.clone(), String::new())], 9, Some("c1".into()), None),
        (vec![(covered.clone(), String::new())], 9, Some("c2".into()), None),
        (vec![(covered, String::new())], 9, Some("c3".into()), None),
    ]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_uncovered_max_scan_pages(2).with_cover_books_pager(pager));

    let (items, _total, next_cursor, _error) = manager.get_uncovered_books_paged(opts(Some(5), None)).await;

    assert!(items.is_empty());
    assert_eq!(next_cursor.as_deref(), Some("c2"));
    assert_eq!(cursors.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn test_uncovered_scans_run_at_most_two_at_once() {
    let tmp = tempfile::tempdir().unwrap();
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));

    let (r, p) = (running.clone(), peak.clone());
    let slow_page: CoverBooksPager = Arc::new(move |_cursor| {
        let (r, p) = (r.clone(), p.clone());
        Box::pin(async move {
            let now = r.fetch_add(1, Ordering::SeqCst) + 1;
            p.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            r.fetch_sub(1, Ordering::SeqCst);
            (Vec::new(), 0, None, None)
        })
    });
    let manager = Arc::new(BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(slow_page)));
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..6 {
        let manager = manager.clone();
        tasks.spawn(async move { manager.get_uncovered_books_paged(opts(None, None)).await });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }

    assert_eq!(peak.load(Ordering::SeqCst), BookManager::UNCOVERED_MAX_CONCURRENT_SCANS);
}

#[tokio::test]
async fn test_uncovered_page_passes_through_errors() {
    let tmp = tempfile::tempdir().unwrap();
    let (pager, _cursors) = scripted(vec![(Vec::new(), 0, None, Some("invalid cursor".into()))]);
    let manager = BookManager::with_deps(deps(tmp.path()).with_cover_books_pager(pager));

    let (items, total, next_cursor, error) = manager.get_uncovered_books_paged(opts(None, Some("x"))).await;

    assert!(items.is_empty());
    assert_eq!((total, next_cursor, error.as_deref()), (None, None, Some("invalid cursor")));
}

#[test]
fn test_unreadable_file_is_not_reported_uncovered() {
    let tmp = tempfile::tempdir().unwrap();
    let mut broken = book("exists.pdf", tmp.path(), (480, 623));
    broken.file_path = tmp.path().join("missing.pdf");
    let manager = make_manager(tmp.path());

    assert_eq!(manager.uncovered_flags(&[broken]), [false]);
}

#[test]
fn test_pdf_without_pages_has_no_first_image() {
    let tmp = tempfile::tempdir().unwrap();
    let pdf = tmp.path().join("empty.pdf");
    common::pdf::PdfBuilder::new().save(&pdf);
    assert_eq!(pdf_first_image_size(&pdf, &LOCK).unwrap(), None);
    assert!(!has_official_cover(&pdf, "pdf", &LOCK).unwrap());
}

#[test]
fn test_epub_is_judged_by_cover_metadata() {
    for cover in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let path = make_epub(&tmp.path().join("book.epub"), &EpubSpec::cover("3.0", cover));

        assert_eq!(has_official_cover(&path, "epub", &LOCK).unwrap(), cover, "cover={cover}");
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)]
async fn uncovered_http_uses_real_book_and_comic_managers() {
    use backend::main::{AppDeps, AuthMode, app_with};
    common::setup_env();
    tokio::task::spawn_blocking(mysql_fixture::mysql_container).await.unwrap();
    let _db_guard = mysql_fixture::db_lock();
    for prefix in ["", "/comics"] {
        let book_root = CoverBook::path_prefix();
        let tmp = tempfile::tempdir_in(&book_root).unwrap();
        let make = |name: &str, size: (u32, u32)| {
            let path = tmp.path().join(name);
            make_pdf(&path, Some(size));
            let relative = path.strip_prefix(&book_root).unwrap();
            CoverBook::new(1, &json!({"category":"", "title":name, "author":"", "file_path":relative, "file_type":"pdf", "file_size":0, "updated_time":"2026-01-01T00:00:00"}), 0.0)
        };
        let covered = make("covered.pdf", (480, 623));
        let plain = make("plain.pdf", (690, 1600));
        let (pager, _) = scripted(vec![(vec![(covered, String::new()), (plain, "작품 제목".into())], 2, None, None)]);
        let managers = deps(&book_root).with_cover_books_pager(pager);
        let deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"})));
        let deps = if prefix.is_empty() {
            deps.with_book_manager(Arc::new(BookManager::with_deps(managers)))
        } else {
            deps.with_comics_manager(Arc::new(backend::comics_manager::ComicsManager::with_deps(managers)))
        };
        let client = common::TestClient::with_app(app_with(deps));
        let response = client.get(&format!("{prefix}/browse?uncovered=true&limit=2")).await;
        assert_eq!(response.status, 200, "{}", response.text());
        let body = response.json();
        assert_eq!(body["status"], "success");
        assert_eq!(body["total"], serde_json::Value::Null);
        assert_eq!(body["result"].as_array().unwrap().len(), 1);
        assert_eq!(body["result"][0]["title"], "plain.pdf");
        assert_eq!(body["result"][0]["file_path"], tmp.path().strip_prefix(&book_root).unwrap().join("plain.pdf").to_string_lossy().as_ref());
        assert_eq!(body["result"][0]["display_title"], "작품 제목");
        assert_eq!(body["next_cursor"], serde_json::Value::Null);
    }
}

struct BrowseManager(Mutex<Vec<serde_json::Value>>);
#[async_trait::async_trait]
impl backend::main::BookManagerApi for BrowseManager {
    async fn get_cover_books_paged(&self, category: &str, size: i64, cursor: &str, excluded: Option<&[String]>) -> (Vec<(CoverBook, String)>, i64, Option<String>, Option<String>) {
        self.0.lock().unwrap().push(json!([category, size, cursor, excluded]));
        (vec![], 17, Some("next".into()), None)
    }
}
struct Hidden;
impl backend::main::CategoryMappingApi for Hidden {
    fn get_hidden_categories(&self, kind: &str) -> Result<Vec<String>, backend::main::MappingError> {
        Ok(vec![if kind == "comic" { "hidden-comic".into() } else { "hidden-book".into() }])
    }
}

struct FailingHidden(backend::main::MappingError);
impl backend::main::CategoryMappingApi for FailingHidden {
    fn get_hidden_categories(&self, _kind: &str) -> Result<Vec<String>, backend::main::MappingError> {
        Err(self.0.clone())
    }
}

struct BrowseOutcomes;
#[async_trait::async_trait]
impl backend::main::BookManagerApi for BrowseOutcomes {
    async fn get_cover_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _excluded: Option<&[String]>) -> (Vec<(CoverBook, String)>, i64, Option<String>, Option<String>) {
        (vec![], 0, None, Some("cover query failed".into()))
    }

    async fn get_uncovered_books_paged(&self, _category: &str, _size: i64, _cursor: &str, _excluded: Option<&[String]>) -> (Vec<(CoverBook, String)>, Option<i64>, Option<String>, Option<String>) {
        (vec![], None, Some("next-uncovered".into()), None)
    }
}

#[tokio::test]
async fn browse_passes_hidden_categories_and_blocks_hidden_descendants() {
    use backend::main::{AppDeps, AuthMode, app_with};
    let manager = Arc::new(BrowseManager(Mutex::new(vec![])));
    let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_book_manager(manager.clone()).with_comics_manager(manager.clone()).with_category_mapping(Arc::new(Hidden));
    let client = common::TestClient::with_app(app_with(deps));
    for (prefix, hidden) in [("", "hidden-book"), ("/comics", "hidden-comic")] {
        assert_eq!(client.get(&format!("{prefix}/browse?category={hidden}/child")).await.status, 403);
        let response = client.get(&format!("{prefix}/browse?category=public&limit=30&cursor=old")).await;
        assert_eq!(response.status, 200);
        assert_eq!(response.json(), json!({"status":"success","result":[],"total":17,"next_cursor":"next"}));
        assert_eq!(manager.0.lock().unwrap().last().unwrap(), &json!(["public", 30, "old", [hidden]]));
    }
    assert_eq!(manager.0.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn browse_requires_auth_and_validates_limits() {
    use backend::main::{AppDeps, AuthMode, app_with};
    let client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Cookies)));
    assert_eq!(client.get("/browse").await.status, 401);
    let manager = Arc::new(BrowseManager(Mutex::new(vec![])));
    let client = common::TestClient::with_app(app_with(AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(manager.clone())));
    for query in ["limit=0", "limit=-1", "limit=10001", "uncovered=invalid"] {
        assert_eq!(client.get(&format!("/browse?{query}")).await.status, 422);
    }
    assert_eq!(client.get("/browse").await.status, 200);
    assert_eq!(manager.0.lock().unwrap().as_slice(), &[json!(["", 60, "", []])]);
}

#[tokio::test]
async fn browse_handles_manager_initialization_and_query_errors() {
    use backend::main::{AppDeps, AuthMode, app_with};

    let unavailable = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager_factory(|| Err("manager unavailable".into()));
    let response = common::TestClient::with_app(app_with(unavailable)).get("/browse").await;
    assert_eq!(response.status, 503);

    let deps = AppDeps::new(AuthMode::Fixed(json!({"role":"admin"}))).with_book_manager(Arc::new(BrowseOutcomes));
    let client = common::TestClient::with_app(app_with(deps));
    let failed = client.get("/browse").await;
    assert_eq!(failed.status, 200);
    assert_eq!(failed.json(), json!({"status":"failure","error":"cover query failed"}));

    let uncovered = client.get("/browse?uncovered=true&category=public&limit=7&cursor=old").await;
    assert_eq!(uncovered.status, 200);
    assert_eq!(uncovered.json(), json!({"status":"success","result":[],"total":null,"next_cursor":"next-uncovered"}));
}

#[tokio::test]
async fn browse_translates_hidden_category_store_errors() {
    use backend::main::{AppDeps, AuthMode, MappingError, app_with};

    for (error, expected_status, expected_detail) in
        [(MappingError::Http(429, "slow down".into()), 429, "slow down"), (MappingError::Other("database failed".into()), 500, "서버 내부 오류가 발생했습니다")]
    {
        let deps = AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_book_manager(Arc::new(BrowseManager(Mutex::new(vec![])))).with_category_mapping(Arc::new(FailingHidden(error)));
        let response = common::TestClient::with_app(app_with(deps)).get("/browse").await;
        assert_eq!(response.status, expected_status);
        assert_eq!(response.json()["detail"], expected_detail);
    }

    let unavailable_mapping =
        AppDeps::new(AuthMode::AuthOnly(json!({"role":"viewer"}))).with_book_manager(Arc::new(BrowseManager(Mutex::new(vec![])))).with_category_mapping_factory(|| Err("mapping unavailable".into()));
    let response = common::TestClient::with_app(app_with(unavailable_mapping)).get("/browse").await;
    assert_eq!(response.status, 503);
}
