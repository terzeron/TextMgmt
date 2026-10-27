//! Online bookstore search endpoint.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

use super::{ApiError, ApiQuery, ApiResult, AppDeps, RequireAuth, Routes};

const MISSING_INPUT: &str = "제목 또는 저자를 입력해주세요";

pub(super) fn routes() -> Routes {
    Routes::new().get("/search/bookstore/{store_name}", &["require_auth"], search_bookstore)
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
    let (rows, search_keyword, search_method, search_url) = tokio::task::spawn_blocking(move || {
        let (rows, keyword, method) = store.search(&isbn, &title, &author);
        let url = if keyword.is_empty() { String::new() } else { store.api_search_url(&keyword) };
        (rows, keyword, method, url)
    })
    .await
    .map_err(|error| ApiError::Internal(error.to_string()))?;

    let result: Vec<Value> = rows
        .into_iter()
        .take(4)
        .map(|(title, author, category, book_url, _, isbn)| {
            let mut item = json!({"title":title,"author":author,"category":category,"book_url":book_url});
            if !isbn.is_empty() {
                item["isbn"] = json!(isbn);
            }
            item
        })
        .collect();
    let status = if result.is_empty() { "not_found" } else { "success" };
    Ok(Json(json!({"status":status,"store":store_name,"search_keyword":search_keyword,"search_method":search_method,"search_url":search_url,"result":result})))
}
