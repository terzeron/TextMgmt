//! Online bookstore search endpoint.

use axum::{
    Json,
    extract::{Path, State},
};
use bookstore::store_clients::SearchRow;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, RequireAdmin, RequireAuth, Routes};
use book_manager::store_category_matcher::{similarity_debug, top_similar_categories};

const MISSING_INPUT: &str = "제목 또는 저자를 입력해주세요";
const BLOCKED_MESSAGE: &str = "서점이 자동 검색을 차단했습니다. 서점 링크로 직접 검색하세요.";

pub(super) fn routes() -> Routes {
    Routes::new().get("/search/bookstore/{store_name}", &["require_auth"], search_bookstore).post("/bookstore/category-suggestions", &["require_admin"], category_suggestions)
}

#[derive(Deserialize)]
struct SuggestionQuery {
    #[serde(default = "default_content_type")]
    content_type: String,
}

fn default_content_type() -> String {
    "book".to_string()
}

fn default_top_n() -> usize {
    5
}

#[derive(Deserialize)]
struct SuggestionBody {
    /// `{"yes24_0_0": "SF 한국SF", ...}`. 키 순서가 동점 순서를 정한다.
    suggested: serde_json::Map<String, Value>,
    categories: Vec<String>,
    #[serde(default = "default_top_n")]
    top_n: usize,
    #[serde(default)]
    debug: bool,
}

/// 편집 화면의 서점 카테고리 추천. 키워드 매핑은 서버가 읽는다. 읽지 못하면 매핑 없이(디렉토리 이름만으로) 계산한다.
async fn category_suggestions(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiQuery(query): ApiQuery<SuggestionQuery>, ApiJson(body): ApiJson<SuggestionBody>) -> ApiResult<Json<Value>> {
    let mapping = deps.category_mapping().map_err(|_| ApiError::unavailable())?;
    let content_type = query.content_type;
    let mappings = tokio::task::spawn_blocking(move || mapping.get_all_mappings(&content_type)).await.map_err(|error| ApiError::Internal(error.to_string()))?.unwrap_or_else(|error| {
        log::warn!("서점 추천 키워드 매핑 조회 실패: {error}");
        Default::default()
    });
    let suggested: Vec<(String, String)> = body.suggested.into_iter().filter_map(|(key, value)| value.as_str().map(|category| (key, category.to_string()))).collect();
    let top = top_similar_categories(&suggested, &body.categories, &mappings, body.top_n);
    let debug = if body.debug { similarity_debug(&suggested, &body.categories, &mappings, body.top_n) } else { None };
    Ok(Json(json!({"status":"success","result":{"top":top,"debug":debug}})))
}

#[derive(Deserialize)]
struct SearchQuery {
    #[serde(default)]
    title: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    isbn: String,
}

async fn search_bookstore(_: RequireAuth, State(deps): State<Arc<AppDeps>>, Path(store_name): Path<String>, ApiQuery(query): ApiQuery<SearchQuery>) -> ApiResult<Json<Value>> {
    let Some(store) = deps.bookstore(&store_name) else {
        return Err(ApiError::http(404, "Bookstore not found"));
    };
    if query.title.is_empty() && query.author.is_empty() && query.isbn.is_empty() {
        return Err(ApiError::http(400, MISSING_INPUT));
    }
    let title = query.title.trim().to_string();
    let author = query.author.trim().to_string();
    let isbn = query.isbn.trim().to_string();
    let (rows, search_keyword, search_method, search_url, blocked) = tokio::task::spawn_blocking(move || {
        let (rows, keyword, method) = store.search(&isbn, &title, &author);
        let url = if keyword.is_empty() { String::new() } else { store.api_search_url(&keyword) };
        (rows, keyword, method, url, store.blocked())
    })
    .await
    .map_err(|error| ApiError::Internal(error.to_string()))?;

    let result: Vec<Value> = rows
        .into_iter()
        .take(4)
        .map(|SearchRow { title, author, category, detail_url: book_url, isbn, .. }| {
            let mut item = json!({"title":title,"author":author,"category":category,"book_url":book_url});
            if !isbn.is_empty() {
                item["isbn"] = json!(isbn);
            }
            item
        })
        .collect();
    let status = match (result.is_empty(), blocked) {
        (false, _) => "success",
        (true, true) => "blocked",
        (true, false) => "not_found",
    };
    let mut body = json!({"status":status,"store":store_name,"search_keyword":search_keyword,"search_method":search_method,"search_url":search_url,"result":result});
    if status == "blocked" {
        body["message"] = json!(BLOCKED_MESSAGE);
    }
    Ok(Json(body))
}
