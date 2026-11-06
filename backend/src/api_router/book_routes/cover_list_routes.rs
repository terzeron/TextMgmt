//! 표지 썸네일과 표지 목록 라우트(`book_manager::cover_list`).

use super::super::app_deps::AppDeps;
use super::super::error_responses::ApiResult;
use super::super::request_extractors::ApiQuery;
use super::super::request_extractors::RequireAuth;
use super::super::route_registry::Routes;
use super::ensure_allowed;
use super::hidden_categories;
use super::kind;
use super::manager_for;
use crate::api_router::request_extractors::parse_py_bool;
use axum::Json;
use axum::extract::OriginalUri;
use axum::extract::Path;
use axum::extract::State;
use axum::response::Response;
use book_manager::CoverPage;
use book_manager::MAX_CATEGORY_PAGE_SIZE;
use serde::Deserialize;
use serde_json::Value;
use serde_json::json;
use std::sync::Arc;

#[allow(unused_imports)]
use super::*;

pub(super) fn routes() -> Routes {
    let mut routes = Routes::new().get("/browse", &["require_auth"], browse).get("/comics/browse", &["require_auth"], browse);
    for prefix in ["", "/comics"] {
        routes = routes.get(&format!("{prefix}/cover/{{book_id}}"), &["require_auth"], cover);
    }
    routes
}

#[derive(Deserialize)]
struct CoverQuery {
    w: Option<i64>,
}

async fn cover(
    RequireAuth(payload): RequireAuth,
    State(deps): State<Arc<AppDeps>>,
    Path(raw_id): Path<String>,
    OriginalUri(uri): OriginalUri,
    ApiQuery(query): ApiQuery<CoverQuery>,
) -> ApiResult<Response> {
    let book_id = parse_book_id(&raw_id)?;
    let manager = checked_manager(&deps, &payload, book_id, uri.path().starts_with("/comics/")).await?;
    book_response(manager.get_cover(book_id, query.w).await)
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
        let uncovered = raw.uncovered.as_deref().map_or(Some(false), parse_py_bool).ok_or("uncovered should be a valid boolean")?;
        Ok(Self { category: raw.category.unwrap_or_default(), limit, cursor: raw.cursor.unwrap_or_default(), uncovered })
    }
}

async fn browse(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    let comics = kind(&uri) == "comic";
    let manager = manager_for(&deps, comics)?;
    let hidden = hidden_categories(&deps, &payload, comics)?;
    ensure_allowed(&query.category, &hidden)?;
    let page = if query.uncovered {
        manager.get_uncovered_books_paged(&query.category, query.limit, &query.cursor, Some(&hidden)).await
    } else {
        manager.get_cover_books_paged(&query.category, query.limit, &query.cursor, Some(&hidden)).await
    };
    let CoverPage { items, total, next_cursor: next } = match page {
        Ok(page) => page,
        Err(error) => return Ok(Json(json!({"status":"failure","error":error}))),
    };
    /// 표지 목록의 책. 화면에 보일 제목을 덧붙인다.
    #[derive(serde::Serialize)]
    struct CoverBook {
        #[serde(flatten)]
        book: book_record::book_metadata::BookView,
        display_title: String,
    }
    let books: Vec<CoverBook> = items.into_iter().map(|(book, display_title)| CoverBook { book: super::book_view(&book, manager.as_ref()), display_title }).collect();
    Ok(Json(json!({"status":"success","result":books,"total":total,"next_cursor":next})))
}

#[cfg(test)]
mod tests {
    use super::{Query, RawQuery};

    fn raw(uncovered: Option<&str>, limit: Option<i64>) -> RawQuery {
        RawQuery { category: Some("fiction".into()), limit, cursor: Some("next".into()), uncovered: uncovered.map(str::to_owned) }
    }

    #[test]
    fn query_validation_accepts_boolean_aliases_and_defaults() {
        let defaults = Query::try_from(RawQuery { category: None, limit: None, cursor: None, uncovered: None }).unwrap();
        assert_eq!((defaults.category.as_str(), defaults.limit, defaults.cursor.as_str(), defaults.uncovered), ("", 60, "", false));
        for value in ["1", "TRUE", "on", "yes", "t", "Y"] {
            assert!(Query::try_from(raw(Some(value), Some(1))).unwrap().uncovered, "{value}");
        }
        for value in ["0", "FALSE", "off", "no", "f", "N"] {
            assert!(!Query::try_from(raw(Some(value), Some(100))).unwrap().uncovered, "{value}");
        }
        assert!(Query::try_from(raw(Some("maybe"), Some(1))).err().unwrap().contains("boolean"));
        assert!(Query::try_from(raw(None, Some(0))).err().unwrap().contains("limit"));
        assert!(Query::try_from(raw(None, Some(10_001))).err().unwrap().contains("limit"));
    }
}
