#[path = "../common/mod.rs"]
mod common;
use crate::common::shared_fixtures::{asyncio_runner, deps, make_doc, make_manager, tmp_dir};
use backend::book_manager::{BookContent, BookManager, MAX_CATEGORY_RESULT_COUNT};
use backend::book_metadata::BookPage;
use backend::main::BookManagerApi;
use common::es_fake::FakeEsManager;
use common::log_capture::LogCapture;
use serde_json::json;
use std::collections::BTreeSet;
use std::fs;
use std::sync::{Arc, Mutex};

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
