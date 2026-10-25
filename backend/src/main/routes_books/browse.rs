//! 도서·만화 표지 목록. 카테고리 정책을 확인한 뒤 일반 목록 또는 미표지 목록을 조회한다.
use axum::{
    Json,
    extract::{OriginalUri, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use crate::book_manager::MAX_CATEGORY_PAGE_SIZE;
use crate::main::{
    deps::AppDeps,
    errors::{ApiError, ApiResult},
    extract::{ApiQuery, RequireAuth},
    routes::Routes,
    traits::MappingError,
};

pub(super) fn routes() -> Routes {
    Routes::new().get("/browse", &["require_auth"], browse).get("/comics/browse", &["require_auth"], browse)
}

#[derive(Deserialize)]
#[serde(try_from = "RawQuery")]
struct Query {
    category: String,
    limit: i64,
    cursor: String,
    uncovered: bool,
}
#[derive(Deserialize)]
struct RawQuery {
    category: Option<String>,
    limit: Option<i64>,
    cursor: Option<String>,
    uncovered: Option<String>,
}
impl TryFrom<RawQuery> for Query {
    type Error = String;
    fn try_from(raw: RawQuery) -> Result<Self, String> {
        let limit = raw.limit.unwrap_or(60);
        if !(1..=MAX_CATEGORY_PAGE_SIZE).contains(&limit) {
            return Err(format!("limit should be between 1 and {MAX_CATEGORY_PAGE_SIZE}"));
        }
        let uncovered = match raw.uncovered.as_deref().unwrap_or("false").to_ascii_lowercase().as_str() {
            "1" | "true" | "on" | "yes" => true,
            "0" | "false" | "off" | "no" => false,
            _ => return Err("uncovered should be a valid boolean".into()),
        };
        Ok(Self { category: raw.category.unwrap_or_default(), limit, cursor: raw.cursor.unwrap_or_default(), uncovered })
    }
}

async fn browse(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    let comics = uri.path().starts_with("/comics/");
    let manager = if comics { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("도서 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })?;
    let hidden = if payload["role"] == "viewer" {
        let mapping = deps.category_mapping().map_err(|e| {
            log::warn!("카테고리 접근 정책을 읽을 수 없다: {e}");
            ApiError::unavailable()
        })?;
        mapping.get_hidden_categories(if comics { "comic" } else { "book" }).map_err(|e| match e {
            MappingError::Http(status, detail) => ApiError::http(status, &detail),
            MappingError::Other(message) => ApiError::Internal(message),
        })?
    } else {
        vec![]
    };
    if !query.category.is_empty() && hidden.iter().any(|h| query.category == *h || query.category.starts_with(&format!("{h}/"))) {
        return Err(ApiError::http(403, "접근 권한이 없는 카테고리입니다."));
    }
    let (items, total, next, error) = if query.uncovered {
        manager.get_uncovered_books_paged(&query.category, query.limit, &query.cursor, Some(&hidden)).await
    } else {
        let (items, total, next, error) = manager.get_cover_books_paged(&query.category, query.limit, &query.cursor, Some(&hidden)).await;
        (items, Some(total), next, error)
    };
    if let Some(error) = error {
        return Ok(Json(json!({"status":"failure","error":error})));
    }
    let root = manager.path_prefix();
    let books: Vec<Value> = items
        .into_iter()
        .map(|(book, title)| {
            let mut value = book.dict();
            if let Ok(relative) = book.file_path.strip_prefix(&root) {
                value["file_path"] = json!(relative);
            }
            value["display_title"] = json!(title);
            value
        })
        .collect();
    Ok(Json(json!({"status":"success","result":books,"total":total,"next_cursor":next})))
}
