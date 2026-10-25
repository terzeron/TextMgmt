//! 카테고리 설정. 동기 DB 작업과 lazy 초기화는 blocking pool에서 실행한다.
use super::extract::python_bool;
use super::{ApiError, ApiJson, ApiQuery, ApiResult, AppDeps, CategoryMappingApi, MappingError, RequireAdmin, RequireAuth, Routes};
use axum::{
    Json,
    extract::{OriginalUri, Path, State},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashMap, sync::Arc};

const MAPPING_ERROR: &str = "카테고리 매핑 처리 중 오류가 발생했습니다";
const HIDDEN_ERROR: &str = "비노출 카테고리 처리 중 오류가 발생했습니다";
const EXCLUDED_ERROR: &str = "최신 자료 검색 제외 카테고리 처리 중 오류가 발생했습니다";

pub(super) fn routes() -> Routes {
    Routes::new()
        .get("/category-mappings", &["require_auth"], all)
        .put("/category-mappings", &["require_admin"], replace_all)
        .get("/category-mappings/{category}", &["require_auth"], keywords)
        .put("/category-mappings/{category}", &["require_admin"], set_keywords)
        .delete("/category-mappings/{category}", &["require_admin"], delete_category)
        .post("/category-mappings/{category}/keywords", &["require_admin"], add_keyword)
        .delete("/category-mappings/{category}/keywords/{keyword}", &["require_admin"], remove_keyword)
        .get("/hidden-categories", &["require_auth"], policy)
        .post("/hidden-categories/{*category}", &["require_admin"], set_hidden)
        .get("/latest-excluded-categories", &["require_auth"], policy)
        .post("/latest-excluded-categories/{*category}", &["require_admin"], set_excluded)
}

#[derive(Deserialize)]
#[serde(default)]
struct Query {
    content_type: String,
}
impl Default for Query {
    fn default() -> Self {
        Self { content_type: "book".into() }
    }
}
#[derive(Deserialize)]
struct Keywords {
    keywords: Vec<String>,
}
#[derive(Deserialize)]
struct Mappings {
    mappings: HashMap<String, Vec<String>>,
}
#[derive(Deserialize)]
struct Hidden {
    #[serde(deserialize_with = "python_bool")]
    hidden: bool,
}
#[derive(Deserialize)]
struct Excluded {
    #[serde(deserialize_with = "python_bool")]
    excluded: bool,
}

fn success(result: impl serde::Serialize) -> Value {
    json!({"status":"success","result":result})
}
fn require_success(ok: bool, code: u16, message: &str) -> Result<(), MappingError> {
    if ok { Ok(()) } else { Err(MappingError::Http(code, message.into())) }
}
async fn run(
    deps: Arc<AppDeps>,
    name: &'static str,
    detail: &'static str,
    preserve_http: bool,
    action: impl FnOnce(Arc<dyn CategoryMappingApi>) -> Result<Value, MappingError> + Send + 'static,
) -> ApiResult<Json<Value>> {
    deps.task_runner().spawn_blocking_observed(name);
    let result = tokio::task::spawn_blocking(move || deps.category_mapping().map_err(MappingError::Other).and_then(action)).await;
    match result {
        Ok(Ok(value)) => Ok(Json(value)),
        Ok(Err(MappingError::Http(code, message))) if preserve_http => Err(ApiError::http(code, &message)),
        error => {
            log::error!("{name} failed: {error:?}");
            Err(ApiError::http(500, detail))
        }
    }
}

async fn all(_: RequireAuth, State(deps): State<Arc<AppDeps>>, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    run(deps, "get_all_mappings", MAPPING_ERROR, false, move |m| m.get_all_mappings(&query.content_type).map(success)).await
}
async fn keywords(_: RequireAuth, State(deps): State<Arc<AppDeps>>, Path(category): Path<String>, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    run(deps, "get_keywords", MAPPING_ERROR, false, move |m| m.get_keywords(&category, &query.content_type).map(success)).await
}
async fn set_keywords(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(category): Path<String>, ApiQuery(query): ApiQuery<Query>, ApiJson(body): ApiJson<Keywords>) -> ApiResult<Json<Value>> {
    run(deps, "set_keywords", MAPPING_ERROR, true, move |m| {
        require_success(m.set_keywords(&category, &body.keywords, &query.content_type)?, 500, "Failed to set keywords")?;
        m.get_keywords(&category, &query.content_type).map(success)
    })
    .await
}
async fn add_keyword(
    _: RequireAdmin,
    State(deps): State<Arc<AppDeps>>,
    Path(category): Path<String>,
    ApiQuery(query): ApiQuery<Query>,
    ApiJson(body): ApiJson<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    let keyword = body.get("keyword").cloned().unwrap_or_default();
    if keyword.is_empty() {
        return Err(ApiError::http(400, "Keyword is required"));
    }
    run(deps, "add_keyword", MAPPING_ERROR, false, move |m| {
        let added = m.add_keyword(&category, &keyword, &query.content_type)?;
        let mut result = success(m.get_keywords(&category, &query.content_type)?);
        if !added {
            result["warning"] = json!("이미 등록된 키워드입니다.");
        }
        Ok(result)
    })
    .await
}
async fn remove_keyword(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path((category, keyword)): Path<(String, String)>, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    run(deps, "remove_keyword", MAPPING_ERROR, true, move |m| {
        require_success(m.remove_keyword(&category, &keyword, &query.content_type)?, 404, "Keyword not found")?;
        m.get_keywords(&category, &query.content_type).map(success)
    })
    .await
}
async fn delete_category(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(category): Path<String>, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    run(deps, "delete_category", MAPPING_ERROR, true, move |m| {
        require_success(m.delete_category(&category, &query.content_type, false)?, 404, "Category not found")?;
        Ok(json!({"status":"success"}))
    })
    .await
}
async fn replace_all(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, ApiQuery(query): ApiQuery<Query>, ApiJson(body): ApiJson<Mappings>) -> ApiResult<Json<Value>> {
    run(deps, "update_all_mappings", MAPPING_ERROR, true, move |m| {
        require_success(m.update_all_mappings(&json!(body.mappings), &query.content_type)?, 500, "Failed to update mappings")?;
        m.get_all_mappings(&query.content_type).map(success)
    })
    .await
}
async fn policy(RequireAuth(payload): RequireAuth, State(deps): State<Arc<AppDeps>>, OriginalUri(uri): OriginalUri, ApiQuery(query): ApiQuery<Query>) -> ApiResult<Json<Value>> {
    if payload["role"] == "viewer" {
        return Ok(Json(success(Vec::<String>::new())));
    }
    if uri.path() == "/hidden-categories" {
        run(deps, "get_hidden_categories", HIDDEN_ERROR, false, move |m| m.get_hidden_categories(&query.content_type).map(success)).await
    } else {
        run(deps, "get_latest_excluded_categories", EXCLUDED_ERROR, false, move |m| m.get_latest_excluded_categories(&query.content_type).map(success)).await
    }
}
async fn set_hidden(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(category): Path<String>, ApiQuery(query): ApiQuery<Query>, ApiJson(body): ApiJson<Hidden>) -> ApiResult<Json<Value>> {
    run(deps, "set_hidden", HIDDEN_ERROR, true, move |m| {
        require_success(m.set_hidden(&category, body.hidden, &query.content_type)?, 500, "Failed to update hidden category")?;
        m.get_hidden_categories(&query.content_type).map(success)
    })
    .await
}
async fn set_excluded(_: RequireAdmin, State(deps): State<Arc<AppDeps>>, Path(category): Path<String>, ApiQuery(query): ApiQuery<Query>, ApiJson(body): ApiJson<Excluded>) -> ApiResult<Json<Value>> {
    run(deps, "set_latest_excluded", EXCLUDED_ERROR, true, move |m| {
        require_success(m.set_latest_excluded(&category, body.excluded, &query.content_type)?, 500, "Failed to update latest excluded category")?;
        m.get_latest_excluded_categories(&query.content_type).map(success)
    })
    .await
}
