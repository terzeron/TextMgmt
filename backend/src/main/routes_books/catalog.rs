//! 도서·만화의 메타데이터, 검색, 카테고리 및 최신 목록.
use crate::book::Book;
use crate::book_manager::MAX_LATEST_BOOK_COUNT;
use crate::main::{ApiError, ApiQuery, ApiResult, AppDeps, BookManagerApi, CategoryMappingApi, MappingError, RequireAuth, Routes};
use axum::{
    Json,
    extract::{OriginalUri, Path, State},
    http::Uri,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new();
    for prefix in ["", "/comics"] {
        routes = routes
            .get(&format!("{prefix}/books/{{book_id}}"), &["require_auth"], metadata)
            .get(&format!("{prefix}/categories"), &["require_auth"], categories)
            .get(&format!("{prefix}/categories/{{*category}}"), &["require_auth"], in_category)
            .get(&format!("{prefix}/categories/rename"), &["require_auth"], reserved_category)
            .get(&format!("{prefix}/categories/delete"), &["require_auth"], reserved_category)
            .get(&format!("{prefix}/latest"), &["require_auth"], latest)
            .get(&format!("{prefix}/search/{{keyword}}"), &["require_auth"], search)
            .get(&format!("{prefix}/similar/{{book_id}}"), &["require_auth"], similar);
    }
    routes
}

fn comics(uri: &Uri) -> bool {
    uri.path().starts_with("/comics/")
}
fn kind(uri: &Uri) -> &'static str {
    if comics(uri) { "comic" } else { "book" }
}
fn manager(deps: &AppDeps, uri: &Uri) -> ApiResult<Arc<dyn BookManagerApi>> {
    if comics(uri) { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("도서 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })
}
fn mapping(deps: &AppDeps) -> ApiResult<Arc<dyn CategoryMappingApi>> {
    deps.category_mapping().map_err(|e| {
        log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
        ApiError::unavailable()
    })
}
fn mapping_error(error: MappingError) -> ApiError {
    match error {
        MappingError::Http(status, detail) => ApiError::http(status, &detail),
        MappingError::Other(message) => ApiError::Internal(message),
    }
}
fn hidden(deps: &AppDeps, payload: &Value, uri: &Uri) -> ApiResult<Vec<String>> {
    if payload["role"] == "viewer" { mapping(deps)?.get_hidden_categories(kind(uri)).map_err(mapping_error) } else { Ok(vec![]) }
}
fn is_hidden(category: &str, hidden: &[String]) -> bool {
    hidden.iter().any(|h| category == h || category.starts_with(&format!("{h}/")))
}
fn ensure_allowed(category: &str, hidden: &[String]) -> ApiResult<()> {
    if is_hidden(category, hidden) { Err(ApiError::http(403, "접근 권한이 없는 카테고리입니다.")) } else { Ok(()) }
}
pub(super) fn book_json(book: &Book, manager: &dyn BookManagerApi) -> Value {
    let manager_prefix = manager.path_prefix();
    let prefix = if !manager_prefix.as_os_str().is_empty() && book.file_path.starts_with(&manager_prefix) { manager_prefix } else { Book::path_prefix() };
    book.dict_with_prefix(&prefix)
}
fn books_json(books: &[Book], manager: &dyn BookManagerApi) -> Vec<Value> {
    books.iter().map(|b| book_json(b, manager)).collect()
}
fn failure(error: Option<String>) -> Json<Value> {
    Json(json!({"status":"failure","error":error}))
}

async fn metadata(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = manager(&deps, &uri)?;
    let (book, error) = manager.get_book(id).await;
    if let Some(book) = &book {
        ensure_allowed(&book.category, &hidden(&deps, &payload, &uri)?)?;
    }
    match book.filter(|_| error.is_none()) {
        Some(book) => Ok(Json(json!({"status":"success","result":book_json(&book,manager.as_ref())}))),
        None => Ok(failure(error)),
    }
}

async fn categories(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let manager = manager(&deps, &uri)?;
    let (mut result, error) = manager.get_categories().await;
    if error.is_some() {
        return Ok(failure(error));
    }
    let hidden = hidden(&deps, &payload, &uri)?;
    if let Some(categories) = result.as_object_mut() {
        categories.retain(|category, _| !is_hidden(category, &hidden));
    }
    Ok(Json(json!({"status":"success","result":result})))
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct CategoryQuery {
    limit: i64,
    cursor: String,
}

// axum은 static 경로를 wildcard보다 먼저 선택한다. 같은 이름의 카테고리 GET도 유지한다.
async fn reserved_category(auth: RequireAuth, deps: State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, query: ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    let category = uri.path().rsplit('/').next().unwrap_or_default().to_owned();
    in_category(auth, deps, OriginalUri(uri), Path(category), query).await
}
async fn in_category(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(category): Path<String>,
    ApiQuery(query): ApiQuery<CategoryQuery>,
) -> ApiResult<Json<Value>> {
    ensure_allowed(&category, &hidden(&deps, &payload, &uri)?)?;
    let manager = manager(&deps, &uri)?;
    if query.limit > 0 {
        let (books, total, next, error) = manager.get_books_in_category_paged(&category, Some(query.limit), Some(&query.cursor)).await;
        if error.is_some() {
            return Ok(failure(error));
        }
        return Ok(Json(json!({"status":"success","result":books_json(&books,manager.as_ref()),"total":total,"next_cursor":next})));
    }
    let (books, error) = manager.get_books_in_category(&category).await;
    if error.is_some() {
        return Ok(failure(error));
    }
    Ok(Json(json!({"status":"success","result":books_json(&books,manager.as_ref())})))
}

#[derive(Deserialize)]
struct LatestQuery {
    limit: Option<i64>,
}
async fn latest(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<LatestQuery>) -> ApiResult<Json<Value>> {
    let limit = query.limit.unwrap_or(MAX_LATEST_BOOK_COUNT);
    if !(1..=MAX_LATEST_BOOK_COUNT).contains(&limit) {
        return Err(crate::main::RequestValidationError::single(&["query", "limit"], "Invalid latest page size", json!(limit)).into());
    }
    let mut excluded = hidden(&deps, &payload, &uri)?;
    for category in mapping(&deps)?.get_latest_excluded_categories(kind(&uri)).map_err(mapping_error)? {
        if !excluded.contains(&category) {
            excluded.push(category);
        }
    }
    let manager = manager(&deps, &uri)?;
    let (books, total, error) = manager.get_latest_books(limit, Some(&excluded)).await;
    if error.is_some() {
        return Ok(failure(error));
    }
    Ok(Json(json!({"status":"success","result":books_json(&books,manager.as_ref()),"total":total})))
}

#[derive(Deserialize)]
#[serde(default)]
struct SearchQuery {
    limit: i64,
    offset: i64,
    category: String,
    exclude_categories: String,
}
impl Default for SearchQuery {
    fn default() -> Self {
        Self { limit: 10, offset: 0, category: String::new(), exclude_categories: String::new() }
    }
}
async fn search(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(keyword): Path<String>,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let mut excluded: Vec<String> = query.exclude_categories.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let hidden = hidden(&deps, &payload, &uri)?;
    if !hidden.is_empty() {
        // Python dict.fromkeys: 숨긴 카테고리를 합칠 때만 중복을 제거한다.
        let mut merged = Vec::new();
        for category in excluded.into_iter().chain(hidden) {
            if !merged.contains(&category) {
                merged.push(category);
            }
        }
        excluded = merged;
    }
    let manager = manager(&deps, &uri)?;
    let excluded = (!excluded.is_empty()).then_some(excluded.as_slice());
    let category = (!query.category.is_empty()).then_some(query.category.as_str());
    let (books, total, error) = manager.search_by_keyword_paged(&keyword, query.limit, query.offset, excluded, category).await;
    if error.is_some() {
        return Ok(failure(error));
    }
    Ok(Json(json!({"status":"success","result":books_json(&books,manager.as_ref()),"total":total})))
}

async fn similar(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<SearchQuery>,
) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = super::checked_manager(&deps, &payload, id, comics(&uri)).await?;
    let (mut books, mut total, error) = manager.search_similar_books_paged(id, query.limit, query.offset).await;
    let hidden = hidden(&deps, &payload, &uri)?;
    if !hidden.is_empty() {
        books.retain(|book| !is_hidden(&book.category, &hidden));
        total = books.len() as i64;
    }
    if !books.is_empty() && error.is_none() {
        return Ok(Json(json!({"status":"success","result":books_json(&books,manager.as_ref()),"total":total})));
    }
    let (book, second_error) = manager.get_book(id).await;
    match book.filter(|_| second_error.is_none()) {
        Some(book) => Ok(Json(json!({"status":"success","result":[book_json(&book,manager.as_ref())],"total":1}))),
        None => Ok(failure(error.or(second_error))),
    }
}
