//! 도서·만화 수정과 카테고리 관리. 실제 파일 작업은 BookManager에 위임한다.
use crate::main::extract::python_bool;
use crate::main::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, BookManagerApi, CategoryMappingApi, MappingError, RequireAdmin, Routes};
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
            .put(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], update_book)
            .delete(&format!("{prefix}/books/{{book_id}}"), &["require_admin"], delete_book)
            .put(&format!("{prefix}/categories/rename"), &["require_admin"], rename_category)
            .post(&format!("{prefix}/categories/delete"), &["require_admin"], delete_category)
            .get(&format!("{prefix}/category-delete-preview"), &["require_admin"], delete_preview)
            .get(&format!("{prefix}/category-pdf-stats"), &["require_admin"], pdf_stats);
    }
    routes
}
fn kind(uri: &Uri) -> &'static str {
    if uri.path().starts_with("/comics/") { "comic" } else { "book" }
}
fn manager(deps: &AppDeps, uri: &Uri) -> ApiResult<Arc<dyn BookManagerApi>> {
    if kind(uri) == "comic" { deps.comics_manager() } else { deps.book_manager() }.map_err(|e| {
        log::warn!("도서 관리자를 쓸 수 없다: {e}");
        ApiError::unavailable()
    })
}
fn result(value: Value, error: Option<String>) -> Json<Value> {
    Json(match error {
        Some(error) => json!({"status":"failure","error":error}),
        None => json!({"status":"success","result":value}),
    })
}
async fn mapping<T: Send + 'static>(deps: Arc<AppDeps>, action: impl FnOnce(Arc<dyn CategoryMappingApi>) -> Result<T, MappingError> + Send + 'static) -> ApiResult<T> {
    let result = tokio::task::spawn_blocking(move || deps.category_mapping().map_err(MappingError::Other).and_then(action)).await.map_err(|e| ApiError::Internal(e.to_string()))?;
    result.map_err(|e| match e {
        MappingError::Http(code, message) => ApiError::http(code, &message),
        MappingError::Other(message) => ApiError::Internal(message),
    })
}

#[derive(Deserialize)]
struct ForceQuery {
    #[serde(default, deserialize_with = "python_bool")]
    force: bool,
}
// 사용하지 않는 BookModel 필드도 HTTP 입력 계약에 따라 검증한다.
#[derive(Deserialize)]
struct BookModel {
    #[serde(rename = "book_id")]
    _book_id: i64,
    category: String,
    title: String,
    author: String,
    file_path: String,
    file_type: String,
    #[serde(rename = "file_size")]
    _file_size: i64,
    #[serde(rename = "updated_time")]
    _updated_time: String,
    #[serde(default, rename = "line_count")]
    _line_count: i64,
    #[serde(default, rename = "page_count")]
    _page_count: i64,
    #[serde(default, rename = "isbn")]
    _isbn: String,
    #[serde(default, rename = "created_time")]
    _created_time: String,
    #[serde(default, rename = "score")]
    _score: f64,
}
async fn update_book(
    _: RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    OriginalUri(uri): OriginalUri,
    Path(id): Path<String>,
    ApiQuery(query): ApiQuery<ForceQuery>,
    ApiJson(body): ApiJson<BookModel>,
) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let manager = manager(&deps, &uri)?;
    let path = manager.path_prefix().join(&body.file_path);
    let (value, error) = manager.update_book(id, &body.category, &body.title, &body.author, &path, &body.file_type, query.force).await;
    Ok(result(value, error))
}
async fn delete_book(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let id = super::parse_book_id(&id)?;
    let (status, message) = manager(&deps, &uri)?.delete_book(id).await;
    Ok(Json(match status.as_str() {
        "Ok" => json!({"status":"success","result":status}),
        "Warning" => json!({"status":"success","result":status,"warning":message}),
        _ => json!({"status":"failure","error":message}),
    }))
}
#[derive(Deserialize)]
struct Rename {
    old_category: String,
    new_category: String,
}
async fn rename_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Rename>) -> ApiResult<Json<Value>> {
    let (mut value, error) = manager(&deps, &uri)?.rename_category(&body.old_category, &body.new_category).await;
    if error.is_none() {
        let ct = kind(&uri);
        let updated = mapping(deps, move |m| m.rename_category(&body.old_category, &body.new_category, ct)).await?;
        if !updated {
            log::warn!("rename_category: MySQL 매핑 갱신 실패");
        }
        value["mapping_updated"] = json!(updated);
    }
    Ok(result(value, error))
}
#[derive(Deserialize)]
struct Delete {
    category: String,
    #[serde(default, deserialize_with = "python_bool")]
    delete_files: bool,
}
async fn delete_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiJson(body): ApiJson<Delete>) -> ApiResult<Json<Value>> {
    let (mut value, error) = manager(&deps, &uri)?.delete_category(&body.category, body.delete_files).await;
    if error.is_none() {
        let ct = kind(&uri);
        let deleted = mapping(deps, move |m| {
            let deleted = m.delete_category(&body.category, ct, true)?;
            let prefix = format!("{}/", body.category);
            m.set_hidden(&body.category, false, ct)?;
            for hidden in m.get_hidden_categories(ct)? {
                if hidden.starts_with(&prefix) {
                    m.set_hidden(&hidden, false, ct)?;
                }
            }
            m.set_latest_excluded(&body.category, false, ct)?;
            for excluded in m.get_latest_excluded_categories(ct)? {
                if excluded.starts_with(&prefix) {
                    m.set_latest_excluded(&excluded, false, ct)?;
                }
            }
            Ok(deleted)
        })
        .await?;
        if !deleted {
            log::warn!("delete_category: MySQL 키워드 매핑 삭제 대상 없음");
        }
        value["mapping_deleted"] = json!(deleted);
    }
    Ok(result(value, error))
}
#[derive(Deserialize)]
struct CategoryQuery {
    category: String,
}
async fn delete_preview(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    let (value, error) = manager(&deps, &uri)?.preview_delete_category(&query.category).await;
    Ok(result(value, error))
}
async fn pdf_stats(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<CategoryQuery>) -> ApiResult<Json<Value>> {
    if query.category.is_empty() || query.category == "_root" {
        return Err(ApiError::http(400, "A directory category is required"));
    }
    let es = manager(&deps, &uri)?.es_manager();
    match tokio::task::spawn_blocking(move || es.search_category_pdf_stats(&query.category)).await {
        Ok(Ok(value)) => Ok(result(value, None)),
        error => {
            log::error!("get_category_pdf_stats failed: {error:?}");
            Ok(result(Value::Null, Some("PDF 통계를 불러오지 못했습니다.".into())))
        }
    }
}
