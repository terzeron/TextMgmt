mod common;
use backend::book_metadata::{Book, path_prefix_from_env};
use serde_json::{Value, json};
use std::path::PathBuf;

fn prefix() -> PathBuf {
    common::setup_env();
    Book::path_prefix()
}

fn info_with_path(file_path: PathBuf, file_type: &str, file_size: u64) -> Value {
    json!({"category": "category1", "title": "title1", "author": "author1", "file_path": file_path.to_str().unwrap(), "file_type": file_type, "file_size": file_size, "updated_time": "2021-01-01T00:00:00.000000"})
}
#[test]
fn test_init() {
    let book_id = 3;
    let info = info_with_path(prefix().join("category1").join("[anonymous] any book.epub"), "file_type1", 100);

    let book = Book::new(book_id, &info, 0.0);
    assert_eq!(book.category, "category1");
    assert_eq!(book.title, "title1");
    assert_eq!(book.author, "author1");
    assert_eq!(book.file_path, prefix().join("category1").join("[anonymous] any book.epub"));
    assert_eq!(book.file_type, "file_type1");
    assert_eq!(book.file_size, 100);
    assert_eq!(book.created_time, book.updated_time);
}

#[test]
fn test_view_serializes_response_shape() {
    let book_id = 3;
    let info = info_with_path(prefix().join("category1").join("[anonymous] any book.epub"), "file_type1", 100);

    let book = Book::new(book_id, &info, 0.0);
    assert_eq!(
        json!(book.view()),
        json!({"book_id": 3, "category": "category1", "title": "title1", "author": "author1", "file_path": "category1/[anonymous] any book.epub", "file_type": "file_type1", "file_size": 100, "line_count": 0, "page_count": 0, "isbn": "", "created_time": "2021-01-01T00:00:00.000000", "updated_time": "2021-01-01T00:00:00.000000", "score": 0.0})
    );
}

#[test]
fn test_json_and_str() {
    let book_id = 4;
    let info = info_with_path(prefix().join("category1").join("book.txt"), "txt", 10);
    let book = Book::new(book_id, &info, 0.0);
    assert_eq!(book.view().title, "title1");
    assert!(book.to_string().contains("category: category1"), "{book}");
}

#[test]
fn test_init_uses_optional_defaults_and_score() {
    common::setup_env();
    let info = json!({
        "category": "category2",
        "title": "title2",
        "author": "author2",
        "file_path": "category2/book2.pdf",
        "file_type": "pdf",
        "file_size": 200,
        "updated_time": "2022-02-02T00:00:00.000000",
    });
    let book = Book::new(5, &info, 12.5);
    assert_eq!(book.line_count, 0);
    assert_eq!(book.page_count, 0);
    assert_eq!(book.isbn, "");
    assert_eq!(book.summary, "");
    assert_eq!(book.score, 12.5);
    assert_eq!(book.view().file_path, "category2/book2.pdf");
}

#[test]
fn test_init_uses_created_time_when_present() {
    common::setup_env();
    let info = json!({
        "category": "category3",
        "title": "title3",
        "author": "author3",
        "file_path": "category3/book3.pdf",
        "file_type": "pdf",
        "file_size": 300,
        "created_time": "2020-01-01T01:02:03.000000",
        "updated_time": "2022-02-02T00:00:00.000000",
    });

    let book = Book::new(6, &info, 0.0);

    assert_eq!(book.view().created_time.unwrap(), "2020-01-01T01:02:03.000000");
}

#[test]
fn test_datetime_fromisoformat_accepts_timezone_suffixes() {
    common::setup_env();
    let info = json!({
        "category": "category3",
        "title": "title3",
        "author": "author3",
        "file_path": "category3/book3.pdf",
        "file_type": "pdf",
        "file_size": 300,
        "created_time": "2020-01-02T03:04:05.123456+09:30",
        "updated_time": "2022-02-03T04:05:06Z",
    });

    let book = Book::new(7, &info, 0.0);

    assert_eq!(book.view().created_time.unwrap(), "2020-01-02T03:04:05.123456");
    assert_eq!(book.view().updated_time, "2022-02-03T04:05:06.000000");
}

#[test]
fn test_datetime_fromisoformat_accepts_comma_fraction() {
    common::setup_env();
    let info = json!({
        "category": "category3",
        "title": "title3",
        "author": "author3",
        "file_path": "category3/book3.pdf",
        "file_type": "pdf",
        "file_size": 300,
        "created_time": "2020-01-02 03:04:05,123456",
        "updated_time": "2022-02-03T04:05:06.000000",
    });

    let book = Book::new(8, &info, 0.0);

    assert_eq!(book.view().created_time.unwrap(), "2020-01-02T03:04:05.123456");
}

#[test]
#[should_panic]
fn test_view_rejects_paths_outside_book_root() {
    common::setup_env();
    let info = info_with_path(PathBuf::from("/outside/book.pdf"), "pdf", 10);
    Book::new(9, &info, 0.0).view();
}

// ---- 환경 변수 누락 검사 ----

#[test]
fn test_book_requires_book_dir() {
    // TM_BOOK_DIR가 없으면 import 시점에 SystemExit가 난다. 환경 변수를 지우지 않고 조회 함수를 주입해 같은 조건을 만든다.
    let missing = path_prefix_from_env(&|_| None);
    assert!(missing.is_err(), "TM_BOOK_DIR 없이도 성공했다: {missing:?}");
}
