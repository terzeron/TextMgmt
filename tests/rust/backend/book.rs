//! 원본: tests/test_book.py
//!
//! 가정한 API (아직 구현 전이다. Python 이름 그대로이고 비공개 `_x`는 `pub fn x`로 본다):
//! - `backend::book::Book::new(book_id: i64, info: &serde_json::Value, score: f64) -> Book` (Python의 `score=0.0` 기본값은 호출부에서 0.0으로 채운다)
//! - `backend::book::Book::path_prefix() -> PathBuf` (`TM_BOOK_DIR`에서 한 번 읽는다)
//! - `Book`의 공개 필드: `category`, `title`, `author`, `file_path: PathBuf`, `file_type`, `file_size`, `line_count`, `page_count`, `isbn`, `summary`, `score: f64`, `created_time`, `updated_time`
//! - `Book::dict(&self) -> serde_json::Value`, `Book::json(&self) -> String`(Python `json.dumps` 형식, 구분자 ", "와 ": "), `impl Display for Book`
//! - `backend::book::path_prefix_from_env(get: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf, String>` (import 시점 `SystemExit`에 대응. loader의 `Config::from_env`와 같은 방식)

mod common;

use std::path::PathBuf;

use backend::book::{Book, path_prefix_from_env};
use serde_json::{Value, json};

fn prefix() -> PathBuf {
    common::setup_env();
    Book::path_prefix()
}

fn info_with_path(file_path: PathBuf, file_type: &str, file_size: u64) -> Value {
    json!({"category": "category1", "title": "title1", "author": "author1", "file_path": file_path.to_str().unwrap(), "file_type": file_type, "file_size": file_size, "updated_time": "2021-01-01T00:00:00.000000"})
}

mod test_book {
    use super::*;

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
    fn test_dict() {
        let book_id = 3;
        let info = info_with_path(prefix().join("category1").join("[anonymous] any book.epub"), "file_type1", 100);

        let book = Book::new(book_id, &info, 0.0);
        assert_eq!(
            book.dict(),
            json!({"book_id": 3, "category": "category1", "title": "title1", "author": "author1", "file_path": "category1/[anonymous] any book.epub", "file_type": "file_type1", "file_size": 100, "line_count": 0, "page_count": 0, "isbn": "", "created_time": "2021-01-01T00:00:00.000000", "updated_time": "2021-01-01T00:00:00.000000", "score": 0.0})
        );
    }

    #[test]
    fn test_json_and_str() {
        let book_id = 4;
        let info = info_with_path(prefix().join("category1").join("book.txt"), "txt", 10);
        let book = Book::new(book_id, &info, 0.0);
        assert!(book.json().contains("\"title\": \"title1\""), "{}", book.json());
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
        assert_eq!(book.dict()["file_path"], "category2/book2.pdf");
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

        assert_eq!(book.dict()["created_time"], "2020-01-01T01:02:03.000000");
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

        assert_eq!(book.dict()["created_time"], "2020-01-02T03:04:05.123456");
        assert_eq!(book.dict()["updated_time"], "2022-02-03T04:05:06.000000");
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

        assert_eq!(book.dict()["created_time"], "2020-01-02T03:04:05.123456");
    }

    #[test]
    #[should_panic]
    fn test_dict_rejects_paths_outside_book_root() {
        common::setup_env();
        let info = info_with_path(PathBuf::from("/outside/book.pdf"), "pdf", 10);
        Book::new(9, &info, 0.0).dict();
    }
}

// ---- merged from test_book_env_guard.py ----

#[test]
fn test_book_requires_book_dir() {
    // TM_BOOK_DIR가 없으면 import 시점에 SystemExit가 난다. 환경 변수를 지우지 않고 조회 함수를 주입해 같은 조건을 만든다.
    let missing = path_prefix_from_env(&|_| None);
    assert!(missing.is_err(), "TM_BOOK_DIR 없이도 성공했다: {missing:?}");
}
