//! 도서·만화 표지 목록. 카테고리 정책을 확인한 뒤 일반 목록 또는 미표지 목록을 조회한다.
use axum::{
    Json,
    extract::{OriginalUri, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{ensure_allowed, hidden_categories, kind, manager_for};
use crate::book_manager::{CoverPage, MAX_CATEGORY_PAGE_SIZE};
use crate::main::{
    deps::AppDeps,
    errors::ApiResult,
    extract::{ApiQuery, RequireAuth, parse_py_bool},
    routes::Routes,
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
    let books: Vec<Value> = items
        .into_iter()
        .map(|(book, title)| {
            let mut value = super::catalog::book_json(&book, manager.as_ref());
            value["display_title"] = json!(title);
            value
        })
        .collect();
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
