//! 조회·검색·추가. Python `get_categories`, `get_books_in_category(_paged)`, `get_book`, `get_book_content`,
//! `search_by_keyword(_paged)`, `search_similar_books(_paged)`, `add_book`에 대응한다.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use std::sync::Arc;

use super::{BookContent, BookManager, MAX_CATEGORY_PAGE_SIZE, MAX_LATEST_BOOK_COUNT};
use crate::book::{Book, BookLookupError, BookPage};
use crate::es_manager::{Docs, EsManagerApi, decode_category_cursor, encode_category_cursor};

/// Python `if doc:`: 없거나 빈 객체면 문서가 없는 것이다.
fn non_empty(doc: Option<Value>) -> Option<Value> {
    doc.filter(|d| !d.is_null() && d.as_object().is_none_or(|o| !o.is_empty()))
}

/// 유사 문서 검색에 쓰는 요약 길이(문자).
const SIMILAR_SUMMARY_CHARS: usize = 3500;

impl BookManager {
    fn books_from_hits(&self, hits: &[(i64, Value, f64)], keep_score: bool) -> Result<Vec<Book>, String> {
        hits.iter().map(|(id, doc, score)| self.try_make_book(*id, doc, if keep_score { *score } else { 0.0 })).collect()
    }

    /// 동기 ES 호출을 워커 스레드로 넘겨 이벤트 루프를 막지 않는다(동기 클라이언트를 루프에서 부르면 같은 워커의 다른 요청이 멈춘다).
    pub(crate) async fn blocking<T: Send + 'static>(&self, f: impl FnOnce(Arc<dyn EsManagerApi>) -> T + Send + 'static) -> T {
        let es = self.es.clone();
        match tokio::task::spawn_blocking(move || f(es)).await {
            Ok(value) => value,
            Err(e) if e.is_panic() => std::panic::resume_unwind(e.into_panic()),
            Err(e) => panic!("blocking task failed: {e}"),
        }
    }

    /// `find_doc`의 비동기 판. 이벤트 루프 밖에서 ES를 부른다.
    pub(crate) async fn find_doc_async(&self, book_id: i64) -> Result<Option<Value>, String> {
        self.blocking(move |es| es.search_by_id(book_id).map(non_empty).map_err(|e| e.to_string())).await
    }

    /// `es.search_by_id`를 Python 진리값 규칙으로 읽는다. ES 오류는 문자열로 돌려준다.
    pub(crate) fn find_doc(&self, book_id: i64) -> Result<Option<Value>, String> {
        self.es.search_by_id(book_id).map(non_empty).map_err(|e| e.to_string())
    }

    pub async fn get_categories(&self) -> Result<BTreeMap<String, i64>, String> {
        log::debug!("# get_categories()");
        self.blocking(|es| es.search_and_aggregate_by_category()).await.map_err(|e| e.to_string())
    }

    pub async fn get_books_in_category(&self, category: &str) -> Result<Vec<Book>, String> {
        let (category_owned, limit) = (category.to_string(), self.max_category_result_count);
        let hits = self.blocking(move |es| es.search_by_category(&category_owned, limit)).await.map_err(|e| e.to_string())?;
        if hits.len() as i64 >= limit {
            log::warn!("get_books_in_category: category '{category}' 결과가 상한({limit})에 도달하여 잘렸습니다.");
        }
        if hits.is_empty() {
            return Err(format!("No books found in '{category}'"));
        }
        self.books_from_hits(&hits, false)
    }

    /// 카테고리 내 책을 커서 기반으로 한 페이지 조회한다.
    pub async fn get_books_in_category_paged(&self, category: &str, size: i64, cursor: &str) -> Result<BookPage, String> {
        log::debug!("# get_books_in_category_paged(category='{category}', size={size}, cursor='{cursor}')");
        let size = size.clamp(1, MAX_CATEGORY_PAGE_SIZE);
        let search_after = if cursor.is_empty() { None } else { decode_category_cursor(cursor) };
        if !cursor.is_empty() && search_after.is_none() {
            return Err("invalid cursor".to_string());
        }
        let category_owned = category.to_string();
        let (hits, total, next) = self.blocking(move |es| es.search_by_category_paged(&category_owned, size, search_after.as_deref())).await.map_err(|e| e.to_string())?;
        let books = self.books_from_hits(&hits, false)?;
        let next_cursor = next.filter(|n| !n.is_empty()).map(|n| encode_category_cursor(&n));
        Ok(BookPage { books, total, next_cursor })
    }

    pub async fn get_book(&self, book_id: i64) -> Result<Book, BookLookupError> {
        log::debug!("# get_book(book_id={book_id})");
        let doc = self.find_doc_async(book_id).await.map_err(BookLookupError::Failed)?.ok_or(BookLookupError::NotFound(book_id))?;
        self.try_make_book(book_id, &doc, 0.0).map_err(BookLookupError::Failed)
    }

    /// 책 파일을 돌려준다. 문서나 파일이 없으면 빈 문자열이다.
    pub async fn get_book_content(&self, book_id: i64) -> BookContent {
        log::debug!("# get_book_content(book_id={book_id})");
        let doc = match self.find_doc_async(book_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => return BookContent::Text(String::new()),
            Err(error) => return BookContent::Error(error),
        };
        let book = match self.try_make_book(book_id, &doc, 0.0) {
            Ok(book) => book,
            Err(error) => return BookContent::Error(error),
        };
        if self.fs.is_file(&book.file_path).unwrap_or(false) {
            return BookContent::File(book.file_path);
        }
        BookContent::Text(String::new())
    }

    pub async fn search_by_keyword(&self, keyword: &str, max_result_count: i64) -> (Vec<Book>, Option<String>) {
        log::debug!("# search_by_keyword(keyword='{keyword}')");
        let keyword_owned = keyword.to_string();
        match self.blocking(move |es| es.search_by_keyword(&keyword_owned, max_result_count)).await {
            Ok(hits) if !hits.is_empty() => match self.books_from_hits(&hits, false) {
                Ok(books) => (books, None),
                Err(error) => (Vec::new(), Some(error)),
            },
            Ok(_) => (Vec::new(), Some("No books found".to_string())),
            Err(e) => (Vec::new(), Some(e.to_string())),
        }
    }

    pub async fn search_by_keyword_paged(&self, keyword: &str, size: i64, offset: i64, exclude_categories: Option<&[String]>, category: Option<&str>) -> Result<BookPage, String> {
        log::debug!("# search_by_keyword_paged(keyword='{keyword}', size={size}, offset={offset}, exclude_categories={exclude_categories:?}, category={category:?})");
        let args = json!({"keyword": keyword, "size": size, "offset": offset, "exclude_categories": exclude_categories, "category": category.filter(|c| !c.is_empty())});
        let (hits, total) = self.blocking(move |es| es.search_by_keyword_paged(&args)).await.map_err(|e| e.to_string())?;
        Ok(BookPage { books: self.books_from_hits(&hits, false)?, total, next_cursor: None })
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
        let doc = match self.find_doc_async(book_id).await {
            Ok(Some(doc)) => doc,
            Ok(None) => return (Vec::new(), Some(format!("No book found with id '{book_id}'"))),
            Err(e) => return (Vec::new(), Some(e)),
        };
        let mut args = Self::similar_args(book_id, &doc);
        args["max_result_count"] = json!(max_result_count);
        match self.blocking(move |es| es.search_similar_docs(&args)).await {
            Ok(hits) if !hits.is_empty() => match self.books_from_hits(&hits, false) {
                Ok(books) => (books, None),
                Err(error) => (Vec::new(), Some(error)),
            },
            Ok(_) => (Vec::new(), Some("No similar books found".to_string())),
            Err(e) => (Vec::new(), Some(e.to_string())),
        }
    }

    pub async fn search_similar_books_paged(&self, book_id: i64, size: i64, offset: i64) -> Result<BookPage, String> {
        log::debug!("# search_similar_books_paged(book_id={book_id}, size={size}, offset={offset})");
        let doc = self.find_doc_async(book_id).await?.ok_or_else(|| format!("No book found with id '{book_id}'"))?;
        let mut args = Self::similar_args(book_id, &doc);
        args["size"] = json!(size);
        args["offset"] = json!(offset);
        let (hits, total) = self.blocking(move |es| es.search_similar_docs_paged(&args)).await.map_err(|e| e.to_string())?;
        if hits.is_empty() {
            return Err("No similar books found".to_string());
        }
        Ok(BookPage { books: self.books_from_hits(&hits, true)?, total, next_cursor: None })
    }

    /// 최신 책 목록. `size`는 1~`MAX_LATEST_BOOK_COUNT`로 맞춘다.
    pub async fn get_latest_books(&self, size: i64, exclude_categories: Option<Vec<String>>) -> Result<BookPage, String> {
        log::debug!("# get_latest_books(size={size}, exclude_categories={exclude_categories:?})");
        let size = size.clamp(1, MAX_LATEST_BOOK_COUNT);
        let (hits, total) = self.blocking(move |es| es.search_latest_docs(size, exclude_categories.as_deref())).await.map_err(|e| e.to_string())?;
        Ok(BookPage { books: self.books_from_hits(&hits, false)?, total, next_cursor: None })
    }

    /// 문서를 색인한다. 문서가 하나일 때만 성공으로 본다. ES 예외는 `Err`로 돌려준다(Python은 예외가 그대로 올라간다).
    pub(crate) fn add_book_raw(&self, data: &Docs) -> Result<(Option<i64>, Option<String>), String> {
        if let Some(adder) = self.seams().book_adder {
            return adder.add_book().map_err(|e| e.to_string());
        }
        let ids = self.es.insert(data).map_err(|e| e.to_string())?;
        if ids.len() == 1 {
            // 단일 문서 추가 후 즉시 검색되도록 한다.
            self.es.refresh().map_err(|e| e.to_string())?;
            return Ok((Some(ids[0]), None));
        }
        Ok((None, Some(format!("can't add book '{}' to ElasticSearch", serde_json::to_string(data).unwrap_or_default()))))
    }

    /// 문서 하나를 색인한다. 성공하면 즉시 검색되도록 `refresh`한다.
    pub async fn add_book(&self, data: &Docs) -> Result<i64, String> {
        log::debug!("# add_book(data={} docs)", data.len());
        let describe = || serde_json::to_string(data).unwrap_or_default();
        match self.add_book_raw(data) {
            Ok((_, Some(error))) => Err(error),
            Ok((Some(book_id), None)) => Ok(book_id),
            Ok((None, None)) => Err(format!("can't add book '{}' to ElasticSearch", describe())),
            Err(e) => Err(format!("can't add book '{}' to ElasticSearch: {e}", describe())),
        }
    }
}
