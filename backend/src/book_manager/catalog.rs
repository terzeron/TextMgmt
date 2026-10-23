//! 조회·검색·추가. Python `get_categories`, `get_books_in_category(_paged)`, `get_book`, `get_book_content`,
//! `search_by_keyword(_paged)`, `search_similar_books(_paged)`, `add_book`에 대응한다.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{BookContent, BookManager, MAX_CATEGORY_PAGE_SIZE, MAX_CATEGORY_RESULT_COUNT};
use crate::book::Book;
use crate::es_manager::{Docs, decode_category_cursor, encode_category_cursor};

/// Python `if doc:`: 없거나 빈 객체면 문서가 없는 것이다.
fn non_empty(doc: Option<Value>) -> Option<Value> {
    doc.filter(|d| !d.is_null() && d.as_object().is_none_or(|o| !o.is_empty()))
}

/// 유사 문서 검색에 쓰는 요약 길이(문자).
const SIMILAR_SUMMARY_CHARS: usize = 3500;

impl BookManager {
    /// `es.search_by_id`를 Python 진리값 규칙으로 읽는다. ES 오류는 문자열로 돌려준다.
    pub(crate) fn find_doc(&self, book_id: i64) -> Result<Option<Value>, String> {
        self.es.search_by_id(book_id).map(non_empty).map_err(|e| e.to_string())
    }

    pub async fn get_categories(&self) -> (BTreeMap<String, i64>, Option<String>) {
        log::debug!("# get_categories()");
        match self.es.search_and_aggregate_by_category() {
            Ok(categories) => (categories, None),
            Err(e) => (BTreeMap::new(), Some(e.to_string())),
        }
    }

    pub async fn get_books_in_category(&self, category: &str) -> (Vec<Book>, Option<String>) {
        let hits = match self.es.search_by_category(category, MAX_CATEGORY_RESULT_COUNT) {
            Ok(hits) => hits,
            Err(e) => return (Vec::new(), Some(e.to_string())),
        };
        if hits.len() as i64 >= MAX_CATEGORY_RESULT_COUNT {
            log::warn!("get_books_in_category: category '{category}' 결과가 상한({MAX_CATEGORY_RESULT_COUNT})에 도달하여 잘렸습니다.");
        }
        if hits.is_empty() {
            return (Vec::new(), Some(format!("No books found in '{category}'")));
        }
        (hits.iter().map(|(id, doc, _)| self.make_book(*id, doc, 0.0)).collect(), None)
    }

    /// 카테고리 내 책을 커서 기반으로 한 페이지 조회한다. 반환: (책 목록, 전체 수, 다음 커서, 오류).
    pub async fn get_books_in_category_paged(&self, category: &str, size: i64, cursor: &str) -> (Vec<Book>, i64, Option<String>, Option<String>) {
        log::debug!("# get_books_in_category_paged(category='{category}', size={size}, cursor='{cursor}')");
        let size = size.clamp(1, MAX_CATEGORY_PAGE_SIZE);
        let search_after = if cursor.is_empty() { None } else { decode_category_cursor(cursor) };
        if !cursor.is_empty() && search_after.is_none() {
            return (Vec::new(), 0, None, Some("invalid cursor".to_string()));
        }
        match self.es.search_by_category_paged(category, size, search_after.as_deref()) {
            Ok((hits, total, next)) => {
                let books = hits.iter().map(|(id, doc, _)| self.make_book(*id, doc, 0.0)).collect();
                let next_cursor = next.filter(|n| !n.is_empty()).map(|n| encode_category_cursor(&n));
                (books, total, next_cursor, None)
            }
            Err(e) => (Vec::new(), 0, None, Some(e.to_string())),
        }
    }

    pub async fn get_book(&self, book_id: i64) -> (Option<Book>, Option<String>) {
        log::debug!("# get_book(book_id={book_id})");
        match self.find_doc(book_id) {
            Ok(Some(doc)) => (Some(self.make_book(book_id, &doc, 0.0)), None),
            Ok(None) => (None, Some(format!("No book found by '{book_id}'"))),
            Err(e) => (None, Some(e)),
        }
    }

    /// 책 파일을 돌려준다. 문서나 파일이 없으면 빈 문자열이다.
    pub async fn get_book_content(&self, book_id: i64) -> BookContent {
        log::debug!("# get_book_content(book_id={book_id})");
        let Ok(Some(doc)) = self.find_doc(book_id) else { return BookContent::Text(String::new()) };
        let book = self.make_book(book_id, &doc, 0.0);
        if self.fs.is_file(&book.file_path).unwrap_or(false) {
            return BookContent::File(book.file_path);
        }
        BookContent::Text(String::new())
    }

    pub async fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> (Vec<Book>, Option<String>) {
        log::debug!("# search_by_keyword(keyword='{keyword}')");
        match self.es.search_by_keyword(keyword, max_result_count) {
            Ok(hits) if !hits.is_empty() => (hits.iter().map(|(id, doc, _)| self.make_book(*id, doc, 0.0)).collect(), None),
            Ok(_) => (Vec::new(), Some("No books found".to_string())),
            Err(e) => (Vec::new(), Some(e.to_string())),
        }
    }

    pub async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[String]>, category: Option<&str>) -> (Vec<Book>, i64, Option<String>) {
        log::debug!("# search_by_keyword_paged(keyword='{keyword}', size={size}, offset={offset}, exclude_categories={exclude_categories:?}, category={category:?})");
        let args = json!({"keyword": keyword, "size": size, "offset": offset, "exclude_categories": exclude_categories, "category": category.filter(|c| !c.is_empty())});
        match self.es.search_by_keyword_paged(&args) {
            Ok((hits, total)) => (hits.iter().map(|(id, doc, _)| self.make_book(*id, doc, 0.0)).collect(), total, None),
            Err(e) => (Vec::new(), 0, Some(e.to_string())),
        }
    }

    /// 유사 문서 검색 인자. 요약은 앞 3500자만 쓴다.
    fn similar_args(book_id: i64, doc: &Value) -> Value {
        let text = |key: &str| doc.get(key).and_then(Value::as_str).unwrap_or("").to_string();
        let summary: String = text("summary").chars().take(SIMILAR_SUMMARY_CHARS).collect();
        json!({
            "category": text("category"), "title": text("title"), "author": text("author"), "file_type": text("file_type"),
            "file_size": doc.get("file_size").and_then(Value::as_i64).unwrap_or(0), "summary": summary, "exclude_id": book_id,
        })
    }

    pub async fn search_similar_books(&self, book_id: i64, max_result_count: i64) -> (Vec<Book>, Option<String>) {
        log::debug!("# search_similar_books(book_id={book_id})");
        let doc = match self.find_doc(book_id) {
            Ok(Some(doc)) => doc,
            Ok(None) => return (Vec::new(), Some(format!("No book found with id '{book_id}'"))),
            Err(e) => return (Vec::new(), Some(e)),
        };
        let mut args = Self::similar_args(book_id, &doc);
        args["max_result_count"] = json!(max_result_count);
        match self.es.search_similar_docs(&args) {
            Ok(hits) if !hits.is_empty() => (hits.iter().map(|(id, similar, _)| self.make_book(*id, similar, 0.0)).collect(), None),
            Ok(_) => (Vec::new(), Some("No similar books found".to_string())),
            Err(e) => (Vec::new(), Some(e.to_string())),
        }
    }

    pub async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> (Vec<Book>, i64, Option<String>) {
        log::debug!("# search_similar_books_paged(book_id={book_id}, size={size}, offset={offset})");
        let doc = match self.find_doc(book_id) {
            Ok(Some(doc)) => doc,
            Ok(None) => return (Vec::new(), 0, Some(format!("No book found with id '{book_id}'"))),
            Err(e) => return (Vec::new(), 0, Some(e)),
        };
        let mut args = Self::similar_args(book_id, &doc);
        args["size"] = json!(size);
        args["offset"] = json!(offset);
        match self.es.search_similar_docs_paged(&args) {
            Ok((hits, total)) if !hits.is_empty() => (hits.iter().map(|(id, similar, score)| self.make_book(*id, similar, *score)).collect(), total, None),
            Ok((_, total)) => (Vec::new(), total, Some("No similar books found".to_string())),
            Err(e) => (Vec::new(), 0, Some(e.to_string())),
        }
    }

    /// 문서를 색인한다. 문서가 하나일 때만 성공으로 본다. ES 예외는 `Err`로 돌려준다(Python은 예외가 그대로 올라간다).
    pub(crate) fn add_book_raw(&self, data: &Docs) -> Result<(Option<i64>, Option<String>), String> {
        let ids = self.es.insert(data).map_err(|e| e.to_string())?;
        if ids.len() == 1 {
            // 단일 문서 추가 후 즉시 검색되도록 한다.
            self.es.refresh().map_err(|e| e.to_string())?;
            return Ok((Some(ids[0]), None));
        }
        Ok((None, Some(format!("can't add book '{}' to ElasticSearch", serde_json::to_string(data).unwrap_or_default()))))
    }

    /// 문서 하나를 색인한다. 성공하면 즉시 검색되도록 `refresh`한다.
    pub async fn add_book(&self, data: &Docs) -> (Option<i64>, Option<String>) {
        log::debug!("# add_book(data={} docs)", data.len());
        match self.add_book_raw(data) {
            Ok(result) => result,
            Err(e) => (None, Some(format!("can't add book '{}' to ElasticSearch: {e}", serde_json::to_string(data).unwrap_or_default()))),
        }
    }
}
