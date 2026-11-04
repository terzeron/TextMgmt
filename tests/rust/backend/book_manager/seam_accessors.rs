//! `backend::book_manager::seam_accessors` 단위 테스트.

#[path = "../common/mod.rs"]
mod common;
use crate::common::category_apply::{ApplyOpts, apply_opts};
use crate::common::shared_fixtures::CONTAINER;
use crate::common::shared_fixtures::{
    allowed_set, asyncio_runner, blank_pdf_bytes_sized, category_with_files, deps, image_bytes, make_deps, make_doc, make_manager, make_source_file, outcome, tmp_dir,
};
use backend::book_manager::{ApplyOptions, BackfillThread, BookManager, CREATED_TIME_BACKFILL_ENV, ProposeOptions};
use backend::main::{BookManagerApi, InlineRunner, TaskHandle, TaskRunner};
use backend::pdf_page_reader::PdfCacheLimits;
use common::env::EnvGuard;
use common::es_fake::FakeEsManager;
use common::fsx::set_mtime;
use common::zipx::write_zip;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{fs, io};
use tempfile::TempDir;

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

#[tokio::test]
async fn test_preview_downscales_large_epub_images() {
    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 최소 바이트 값을 0으로 두고 축소 함수 호출 길이를 기록하는 훅
    let calls: Arc<Mutex<Vec<usize>>> = Arc::new(Mutex::new(Vec::new()));
    let spy = calls.clone();

    let tmp = TempDir::new().unwrap();
    let opf = r#"<?xml version="1.0"?>
            <package xmlns="http://www.idpf.org/2007/opf">
              <manifest>
                <item id="c1" href="ch1.xhtml" media-type="application/xhtml+xml"/>
                <item id="img" href="images/a.png" media-type="image/png"/>
              </manifest>
              <spine><itemref idref="c1"/></spine>
            </package>"#;
    write_zip(
        &tmp.path().join("pic.epub"),
        &[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OEBPS/ch1.xhtml", br#"<html><body><img src="images/a.png"/></body></html>"#),
            ("OEBPS/images/a.png", &image_bytes(image::ImageFormat::Png, (64, 64))),
            ("OEBPS/content.opf", opf.as_bytes()),
        ],
    );
    let deps = make_deps(tmp.path(), FakeEsManager::with_doc(make_doc("pic.epub", "epub"))).with_epub_image_min_bytes(0).with_downscale_epub_image_hook(Arc::new(move |data: &[u8]| {
        spy.lock().unwrap().push(data.len());
        BookManager::_downscale_epub_image(data)
    }));
    let manager = BookManager::with_deps(deps);

    let resp = BookManagerApi::get_book_preview(&manager, 1, Some(1), None).await;

    assert_eq!(resp.status_code, 200);
    assert!(!calls.lock().unwrap().is_empty(), "이미지 멤버가 축소 경로를 지나야 한다");
}

#[tokio::test]
async fn test_propose_awaits_async_progress_callback_and_records_failures() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "A", &["ok.txt", "bad.txt"]);

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 파일별 제안 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), FakeEsManager::new()).with_propose_category_for_file_hook(Arc::new(|file_path: &Path| {
        if file_path.file_name().unwrap() == "bad.txt" {
            return Err("분류기 오류".to_string());
        }
        Ok(json!({"target_category": "B", "grade": "high"}))
    }));
    let manager = BookManager::with_deps(deps);
    let reports: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = reports.clone();
    // 진행 보고 콜백은 동기 함수다(Python은 코루틴도 받는다).
    let on_progress = move |payload: Value| sink.lock().unwrap().push(payload);
    let opts = ProposeOptions { content_type: Some("book"), delay: Some(0.0), use_bookstore: Some(false), use_content_meta: Some(false), on_progress: Some(&on_progress) };

    let result = manager.propose_category_changes("A", None, opts).await.unwrap();

    let titles: Vec<&str> = result["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, vec!["ok"]);
    assert_eq!(result["failures"], json!([{"file_path": "A/bad.txt", "error": "분류 제안에 실패했습니다"}]));
    assert!(reports.lock().unwrap().iter().any(|r| r.get("updated_items").is_some()));
}

// ---- 분류 적용 ----

#[tokio::test]
async fn test_apply_awaits_async_callbacks() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "1_A", &["a.txt"]);
    fs::create_dir(tmp.path().join("2_B")).unwrap();
    let done: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let progress: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 파일 이동 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), FakeEsManager::new()).with_move_classified_file_hook(Arc::new(|_path: &Path, _target: &str| Ok(json!({"status": "moved"}))));
    let manager = BookManager::with_deps(deps);
    let (done_sink, progress_sink) = (done.clone(), progress.clone());
    // 콜백은 동기 함수다(Python은 코루틴도 받는다).
    let should_continue = || true;
    let on_item_done = move |payload: Value| done_sink.lock().unwrap().push(payload["apply_status"].as_str().unwrap().to_string());
    let on_progress = move |payload: Value| progress_sink.lock().unwrap().push(payload);
    let opts = ApplyOptions { should_continue: Some(&should_continue), on_item_done: Some(&on_item_done), on_progress: Some(&on_progress), ..Default::default() };
    let allowed: HashSet<String> = ["1_A/a.txt".to_string()].into();

    let result = manager.apply_category_changes_with(&[json!({"file_path": "1_A/a.txt", "target_category": "2_B"})], &allowed, opts).await.unwrap();

    assert_eq!(result["applied_count"], 1);
    assert_eq!(*done.lock().unwrap(), vec!["moving", "moved"]);
    assert_eq!(*progress.lock().unwrap(), vec![json!({"apply_total_count": 1, "applied_count": 1, "failed_count": 0})]);
}

#[tokio::test]
async fn test_apply_records_move_error_as_failed() {
    let tmp = TempDir::new().unwrap();
    category_with_files(tmp.path(), "1_A", &["a.txt"]);
    fs::create_dir(tmp.path().join("2_B")).unwrap();

    // 가정한 seam(문서 3.12에 없음. 3.0 규칙대로 Deps 빌더로 옮겼다): 파일 이동 함수를 교체하는 훅
    let deps = make_deps(tmp.path(), FakeEsManager::new()).with_move_classified_file_hook(Arc::new(|_path: &Path, _target: &str| Err("이동 실패".to_string())));
    let manager = BookManager::with_deps(deps);
    let allowed: HashSet<String> = ["1_A/a.txt".to_string()].into();

    let (result, _) = outcome(manager.apply_category_changes(&[json!({"file_path": "1_A/a.txt", "target_category": "2_B"})], &allowed, Default::default()).await);

    assert_eq!(result["failed_count"], 1);
    assert_eq!(result["results"][0]["apply_error"], "이동 실패");
}

mod pdf_reader_cache_budget {
    use super::common;
    use crate::common::pdf_env::build;
    use crate::common::shared_fixtures::create_test_pdf;
    use backend::pdf_page_reader::PdfCacheLimits;
    use std::fs;
    use std::sync::Arc;
    use std::time::Duration;

    /// 파일당 상한을 넘는 PDF 는 reader 를 캐시하지 않는다 (페이지 수만 캐시).
    #[test]
    fn test_large_pdf_reader_not_cached() {
        let e = build(|dir, d| {
            let pdf = dir.join("test_category").join("budget_large.pdf");
            create_test_pdf(3, &pdf);
            let size = fs::metadata(&pdf).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(size - 1), ..PdfCacheLimits::default() })
        });
        let pdf = e.temp_dir.join("test_category").join("budget_large.pdf");

        let (reader, total_pages) = e.bm._get_cached_pdf_reader(&pdf);

        assert_eq!(total_pages, 3);
        assert!(Arc::strong_count(&reader) >= 1); // reader 자체는 정상 반환되어야 한다
        assert!(!e.bm.pdf_cache().contains(&pdf.to_string_lossy())); // 그러나 보관하지 않는다
        // 페이지 수는 int 하나라 비용이 없으므로 항상 캐시한다
        assert!(e.bm.page_count_cache().contains(&(pdf.to_string_lossy().into_owned(), common::fsx::mtime_key(&pdf))));
    }

    #[test]
    fn test_small_pdf_reader_is_cached_and_reused() {
        let e = build(|dir, d| {
            let pdf = dir.join("test_category").join("budget_small.pdf");
            create_test_pdf(2, &pdf);
            let size = fs::metadata(&pdf).unwrap().len();
            d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(size + 1), ..PdfCacheLimits::default() })
        });
        let pdf = e.temp_dir.join("test_category").join("budget_small.pdf");

        let (reader1, _) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(e.bm.pdf_cache().contains(&pdf.to_string_lossy()));
        let (reader2, _) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(Arc::ptr_eq(&reader1, &reader2)); // 같은 객체를 재사용 (재파싱 없음)
    }

    #[test]
    fn test_mtime_change_invalidates_reader_cache() {
        let e = build(|_, d| d.with_pdf_cache_limits(PdfCacheLimits { max_file_bytes: Some(10 * 1024 * 1024), ..PdfCacheLimits::default() }));
        let pdf = e.temp_dir.join("test_category").join("budget_mtime.pdf");
        create_test_pdf(2, &pdf);

        let (reader1, pages1) = e.bm._get_cached_pdf_reader(&pdf);
        assert_eq!(pages1, 2);

        create_test_pdf(4, &pdf); // 내용 교체
        let meta = fs::metadata(&pdf).unwrap();
        let times = fs::FileTimes::new().set_accessed(meta.accessed().unwrap() + Duration::from_secs(10)).set_modified(meta.modified().unwrap() + Duration::from_secs(10));
        fs::OpenOptions::new().write(true).open(&pdf).unwrap().set_times(times).unwrap();

        let (reader2, pages2) = e.bm._get_cached_pdf_reader(&pdf);
        assert!(!Arc::ptr_eq(&reader2, &reader1));
        assert_eq!(pages2, 4);
        assert_eq!(e.bm.pdf_cache().len(), 1); // 낡은 항목이 남지 않는다
    }
}
