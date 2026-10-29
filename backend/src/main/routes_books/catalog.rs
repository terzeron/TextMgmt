//! 도서·만화의 메타데이터, 검색, 카테고리 및 최신 목록.
use super::{ensure_allowed, hidden_categories, is_hidden, kind, manager};
use crate::book::Book;
use crate::book_manager::MAX_LATEST_BOOK_COUNT;
use crate::main::{ApiError, ApiQuery, ApiResult, AppDeps, BookManagerApi, CategoryMappingApi, RequireAuth, Routes};
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
    kind(uri) == "comic"
}
fn mapping(deps: &AppDeps) -> ApiResult<Arc<dyn CategoryMappingApi>> {
    deps.category_mapping().map_err(|e| {
        log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
        ApiError::unavailable()
    })
}
fn hidden(deps: &AppDeps, payload: &Value, uri: &Uri) -> ApiResult<Vec<String>> {
    hidden_categories(deps, payload, comics(uri))
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
    match manager.get_book(id).await {
        Ok(book) => {
            ensure_allowed(&book.category, &hidden(&deps, &payload, &uri)?)?;
            Ok(Json(json!({"status":"success","result":book_json(&book,manager.as_ref())})))
        }
        Err(error) => Ok(failure(Some(error.to_string()))),
    }
}

async fn categories(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri) -> ApiResult<Json<Value>> {
    let manager = manager(&deps, &uri)?;
    let mut result = match manager.get_categories().await {
        Ok(result) => result,
        Err(error) => return Ok(failure(Some(error))),
    };
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
        return Ok(match manager.get_books_in_category_paged(&category, Some(query.limit), Some(&query.cursor)).await {
            Ok(page) => Json(json!({"status":"success","result":books_json(&page.books,manager.as_ref()),"total":page.total,"next_cursor":page.next_cursor})),
            Err(error) => failure(Some(error)),
        });
    }
    Ok(match manager.get_books_in_category(&category).await {
        Ok(books) => Json(json!({"status":"success","result":books_json(&books,manager.as_ref())})),
        Err(error) => failure(Some(error)),
    })
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
    for category in mapping(&deps)?.get_latest_excluded_categories(kind(&uri))? {
        if !excluded.contains(&category) {
            excluded.push(category);
        }
    }
    let manager = manager(&deps, &uri)?;
    Ok(match manager.get_latest_books(limit, Some(&excluded)).await {
        Ok(page) => Json(json!({"status":"success","result":books_json(&page.books,manager.as_ref()),"total":page.total})),
        Err(error) => failure(Some(error)),
    })
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
    Ok(match manager.search_by_keyword_paged(&keyword, query.limit, query.offset, excluded, category).await {
        Ok(page) => Json(json!({"status":"success","result":books_json(&page.books,manager.as_ref()),"total":page.total})),
        Err(error) => failure(Some(error)),
    })
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
    let similar = manager.search_similar_books_paged(id, query.limit, query.offset).await;
    let hidden = hidden(&deps, &payload, &uri)?;
    let error = match similar {
        Ok(mut page) => {
            if !hidden.is_empty() {
                page.books.retain(|book| !is_hidden(&book.category, &hidden));
                page.total = page.books.len() as i64;
            }
            if !page.books.is_empty() {
                return Ok(Json(json!({"status":"success","result":books_json(&page.books,manager.as_ref()),"total":page.total})));
            }
            None
        }
        Err(error) => Some(error),
    };
    match manager.get_book(id).await {
        Ok(book) => Ok(Json(json!({"status":"success","result":[book_json(&book,manager.as_ref())],"total":1}))),
        Err(second_error) => Ok(failure(error.or(Some(second_error.to_string())))),
    }
}
